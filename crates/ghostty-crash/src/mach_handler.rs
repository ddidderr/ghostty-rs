//! macOS minidump capture through a Mach exception port, a port of the
//! structure of Breakpad's in-process `ExceptionHandler` (`exception_handler.cc`)
//! which sentry-native's breakpad backend uses upstream.
//!
//! `install` allocates a receive right, remembers the task's previous
//! exception ports (`task_get_exception_ports`), registers the new port for
//! `EXC_BAD_ACCESS`, `EXC_BAD_INSTRUCTION`, `EXC_ARITHMETIC` and
//! `EXC_BREAKPOINT` (`task_set_exception_ports`) and starts a handler thread
//! that blocks in `mach_msg`. When a thread faults, the kernel suspends it and
//! sends a `mach_exception_raise` message; the handler thread (which runs on
//! its own healthy stack, so stack overflows work) then:
//!
//! 1. suspends all other threads,
//! 2. reads the crashed thread's registers with `thread_get_state` for the
//!    event's stack trace,
//! 3. writes a minidump with `minidump-writer` for the whole task and attaches
//!    it to the envelope (`event.minidump`),
//! 4. restores the previous exception ports and replies `KERN_FAILURE`, so the
//!    kernel continues with the next handler: the BSD signal is delivered and
//!    the process dies as usual (Apple's crash reporter included). The signal
//!    path (`signal.rs`) sees `BUSY` set and writes nothing more.
//!
//! Signals that are not Mach exceptions (`SIGABRT` from `abort()`) keep going
//! through the signal handler. A watchdog `alarm(2)` kills the process if the
//! dump hangs (e.g. a suspended thread held the allocator lock).
//!
//! Unlike Breakpad this does not forward the exception to previously
//! registered handlers; it restores them, which only matters for debuggers
//! attached before `install` (they are re-armed for the next exception).
#![allow(unsafe_code)]

use std::{
    io::Cursor,
    mem::size_of,
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

use mach2::{
    exception_types::{
        EXC_MASK_ARITHMETIC,
        EXC_MASK_BAD_ACCESS,
        EXC_MASK_BAD_INSTRUCTION,
        EXC_MASK_BREAKPOINT,
        EXCEPTION_DEFAULT,
        MACH_EXCEPTION_CODES,
        exception_behavior_t,
        exception_mask_t,
    },
    kern_return::{KERN_FAILURE, KERN_SUCCESS},
    mach_types::{thread_act_t, thread_t},
    message::{
        MACH_MSG_TIMEOUT_NONE,
        MACH_MSG_TYPE_MAKE_SEND,
        MACH_MSGH_BITS,
        MACH_MSGH_BITS_REMOTE_MASK,
        MACH_RCV_LARGE,
        MACH_RCV_MSG,
        MACH_RCV_PORT_DIED,
        MACH_SEND_MSG,
        mach_msg,
        mach_msg_header_t,
    },
    ndr::{NDR_record, NDR_record_t},
    port::{MACH_PORT_NULL, MACH_PORT_RIGHT_RECEIVE, mach_port_t},
    thread_status::thread_state_flavor_t,
};
use minidump_writer::{
    crash_context::{CrashContext, ExceptionInfo},
    minidump_writer::MinidumpWriter,
};

use crate::{
    exception::{self, MachException},
    native::Native,
    sentry,
    signal::BUSY,
};

/// Maximum number of registered exception handlers (`EXC_TYPES_COUNT`).
const MAX_PORTS: usize = 14;

/// Seconds the dump may take before the watchdog kills the process.
const WATCHDOG_SECS: libc::c_uint = 20;

const MASK: exception_mask_t =
    EXC_MASK_BAD_ACCESS | EXC_MASK_BAD_INSTRUCTION | EXC_MASK_ARITHMETIC | EXC_MASK_BREAKPOINT;

/// The task's exception ports before `install`.
struct Previous {
    count: u32,
    masks: [exception_mask_t; MAX_PORTS],
    ports: [mach_port_t; MAX_PORTS],
    behaviors: [exception_behavior_t; MAX_PORTS],
    flavors: [thread_state_flavor_t; MAX_PORTS],
}

static PREVIOUS: OnceLock<Previous> = OnceLock::new();
static PORT: AtomicU32 = AtomicU32::new(MACH_PORT_NULL);

#[cfg(target_arch = "x86_64")]
type ThreadState = mach2::structs::x86_thread_state64_t;
#[cfg(target_arch = "aarch64")]
type ThreadState = mach2::structs::arm_thread_state64_t;

#[cfg(target_arch = "x86_64")]
fn state_flavor() -> thread_state_flavor_t {
    mach2::thread_status::x86_THREAD_STATE64
}
#[cfg(target_arch = "aarch64")]
fn state_flavor() -> thread_state_flavor_t {
    mach2::thread_status::ARM_THREAD_STATE64
}

/// Register the exception port and start the handler thread. Returns false
/// (leaving the signal path as the only capture) if anything fails.
pub(crate) fn install() -> bool {
    if PORT.load(Ordering::Acquire) != MACH_PORT_NULL {
        return true;
    }
    // SAFETY: plain Mach calls with valid out pointers.
    unsafe {
        let task = mach2::traps::mach_task_self();
        let mut port: mach_port_t = MACH_PORT_NULL;
        if mach2::mach_port::mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &raw mut port)
            != KERN_SUCCESS
        {
            return false;
        }
        if mach2::mach_port::mach_port_insert_right(task, port, port, MACH_MSG_TYPE_MAKE_SEND)
            != KERN_SUCCESS
        {
            mach2::mach_port::mach_port_destroy(task, port);
            return false;
        }

        let mut prev = Previous {
            count: MAX_PORTS as u32,
            masks: [0; MAX_PORTS],
            ports: [MACH_PORT_NULL; MAX_PORTS],
            behaviors: [0; MAX_PORTS],
            flavors: [0; MAX_PORTS],
        };
        if mach2::task::task_get_exception_ports(
            task,
            MASK,
            prev.masks.as_mut_ptr(),
            &raw mut prev.count,
            prev.ports.as_mut_ptr(),
            prev.behaviors.as_mut_ptr(),
            prev.flavors.as_mut_ptr(),
        ) != KERN_SUCCESS
        {
            prev.count = 0;
        }
        let _ = PREVIOUS.set(prev);

        let spawned = std::thread::Builder::new()
            .name("ghostty-exception-handler".into())
            .spawn(move || handler_thread(port));
        if spawned.is_err() {
            mach2::mach_port::mach_port_destroy(task, port);
            return false;
        }
        PORT.store(port, Ordering::Release);
        mach2::task::task_set_exception_ports(
            task,
            MASK,
            port,
            EXCEPTION_DEFAULT as exception_behavior_t
                | MACH_EXCEPTION_CODES as exception_behavior_t,
            state_flavor(),
        ) == KERN_SUCCESS
    }
}

/// Put the previously registered exception ports back.
fn restore_ports() {
    let Some(prev) = PREVIOUS.get() else { return };
    // SAFETY: plain Mach calls; `prev` holds ports the kernel gave us.
    unsafe {
        let task = mach2::traps::mach_task_self();
        // Unregister ours for the masks we took over first.
        mach2::task::task_set_exception_ports(
            task,
            MASK,
            MACH_PORT_NULL,
            EXCEPTION_DEFAULT as exception_behavior_t,
            mach2::thread_status::THREAD_STATE_NONE,
        );
        for i in 0..(prev.count as usize).min(MAX_PORTS) {
            mach2::task::task_set_exception_ports(
                task,
                prev.masks[i],
                prev.ports[i],
                prev.behaviors[i],
                prev.flavors[i],
            );
        }
    }
}

/// Receive buffer: header, body, descriptors, NDR, codes and the trailer.
#[repr(C, align(8))]
struct Buffer([u8; 1024]);

#[repr(C, packed(4))]
struct Reply {
    head: mach_msg_header_t,
    ndr: NDR_record_t,
    ret_code: i32,
}

fn handler_thread(port: mach_port_t) {
    // SAFETY: Mach calls with valid pointers.
    let handler: thread_t = unsafe { mach2::mach_init::mach_thread_self() };
    let mut buf = Buffer([0; 1024]);
    loop {
        // SAFETY: `buf` is a writable 1024 byte aligned buffer.
        let r = unsafe {
            mach_msg(
                buf.0.as_mut_ptr().cast(),
                MACH_RCV_MSG | MACH_RCV_LARGE,
                0,
                buf.0.len() as u32,
                port,
                MACH_MSG_TIMEOUT_NONE,
                MACH_PORT_NULL,
            )
        };
        if r == MACH_RCV_PORT_DIED {
            return;
        }
        if r != 0 {
            continue;
        }
        let Some(req) = exception::parse_raise(&buf.0) else {
            continue;
        };
        handle_exception(&req, handler);

        // The previous ports are back; failing the request makes the kernel
        // fall through to the next handler (the BSD signal).
        // SAFETY: the first bytes of `buf` are the request's header.
        let head = unsafe { *(&raw const buf).cast::<mach_msg_header_t>() };
        let mut reply = Reply {
            head: mach_msg_header_t {
                msgh_bits: MACH_MSGH_BITS(head.msgh_bits & MACH_MSGH_BITS_REMOTE_MASK, 0),
                msgh_size: size_of::<Reply>() as u32,
                msgh_remote_port: head.msgh_remote_port,
                msgh_local_port: MACH_PORT_NULL,
                msgh_voucher_port: 0,
                msgh_id: head.msgh_id + 100,
            },
            // SAFETY: `NDR_record` is a constant provided by libsystem.
            ndr: unsafe { NDR_record },
            ret_code: KERN_FAILURE,
        };
        // SAFETY: `reply` is a fully initialized message of the given size.
        unsafe {
            mach_msg(
                (&raw mut reply).cast(),
                MACH_SEND_MSG,
                size_of::<Reply>() as u32,
                0,
                MACH_PORT_NULL,
                MACH_MSG_TIMEOUT_NONE,
                MACH_PORT_NULL,
            );
        }
    }
}

/// Suspend every thread of the task except `keep` (the handler thread).
fn suspend_other_threads(keep: thread_act_t) {
    // SAFETY: task_threads returns a vm-allocated array we only read.
    unsafe {
        let task = mach2::traps::mach_task_self();
        let mut list: mach2::mach_types::thread_act_array_t = std::ptr::null_mut();
        let mut n: u32 = 0;
        if mach2::task::task_threads(task, &raw mut list, &raw mut n) != KERN_SUCCESS {
            return;
        }
        for i in 0..n as usize {
            let t = *list.add(i);
            if t != keep {
                mach2::thread_act::thread_suspend(t);
            }
        }
    }
}

fn handle_exception(req: &exception::RaiseRequest, handler: thread_t) {
    if let Some(st) = sentry::state()
        && !BUSY.swap(true, Ordering::Acquire)
    {
        // SAFETY: alarm has no preconditions. SIGALRM's default action ends
        // the process if the dump below hangs.
        unsafe { libc::alarm(WATCHDOG_SECS) };
        suspend_other_threads(handler);
        write_report(st, req, handler);
        // SAFETY: as above.
        unsafe { libc::alarm(0) };
    }
    restore_ports();
}

fn write_report(st: &sentry::State, req: &exception::RaiseRequest, handler: thread_t) {
    let ex: MachException = req.exception;

    // Registers and frame-pointer walk of the faulting thread.
    let mut native = Box::new(Native::new());
    (native.signo, native.code) = ex.signal();
    native.addr = ex.fault_addr();
    let mut state: ThreadState = ThreadState::default();
    let mut count = (size_of::<ThreadState>() / 4) as u32;
    // SAFETY: `state` is a writable thread state of the requested flavor.
    let kr = unsafe {
        mach2::thread_act::thread_get_state(
            req.thread,
            state_flavor(),
            (&raw mut state).cast(),
            &raw mut count,
        )
    };
    if kr == KERN_SUCCESS {
        // SAFETY: `state` is a valid, aligned native thread state.
        unsafe { crate::darwin::fill_thread_state(&mut native, (&raw const state).cast()) };
    }

    let cc = CrashContext {
        task: req.task,
        thread: req.thread,
        handler_thread: handler,
        exception: Some(ExceptionInfo {
            kind: ex.kind,
            code: ex.code as u64,
            subcode: (ex.kind == exception::EXC_BAD_ACCESS).then_some(ex.subcode as u64),
        }),
    };
    let dump = MinidumpWriter::with_crash_context(cc).dump(&mut Cursor::new(Vec::new()));
    let dump = match dump {
        Ok(d) => Some(d),
        Err(e) => {
            log::warn!("minidump failed err={e}");
            None
        }
    };

    let mut value = String::new();
    ex.describe(&mut value);
    match sentry::write_report_ext(
        st,
        ex.name(),
        &value,
        "mach",
        Some(&native),
        None,
        dump.as_deref(),
    ) {
        Ok(path) => eprintln!(
            "warning(sentry): crash report with minidump written to disk path={}",
            path.display()
        ),
        Err(e) => eprintln!("warning(sentry): failed to persist crash report err={e}"),
    }
}
