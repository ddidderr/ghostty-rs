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
//! 1. suspends all other threads (and resumes *all* of them, the faulting one
//!    included, before replying; otherwise the process hangs instead of
//!    dying),
//! 2. reads the crashed thread's registers with `thread_get_state` for the
//!    event's stack trace,
//! 3. writes a minidump with `minidump-writer` for the whole task and attaches
//!    it to the envelope (`event.minidump`),
//! 4. restores the previous exception ports and replies `KERN_FAILURE`, so the
//!    kernel continues with the next handler: the BSD signal is delivered and
//!    the process dies as usual (Apple's crash reporter included). The signal
//!    path (`signal.rs`) sees `BUSY` set and writes nothing more.
//!
//! `SIGABRT` from `abort()` is not a Mach exception. Like Breakpad, the signal
//! handler asks this handler thread for the dump (`request_abort_dump`: a
//! message to the exception port, then it waits on a reply port), so
//! `abort()` gets a minidump too. A watchdog `alarm(2)` kills the process if
//! the dump hangs (e.g. a suspended thread held the allocator lock).
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
        MACH_MSG_TYPE_COPY_SEND,
        MACH_MSG_TYPE_MAKE_SEND,
        MACH_MSGH_BITS,
        MACH_MSGH_BITS_REMOTE_MASK,
        MACH_RCV_LARGE,
        MACH_RCV_MSG,
        MACH_RCV_PORT_DIED,
        MACH_RCV_TIMEOUT,
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
    suspend::{SuspendGuard, Threads},
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
/// Receive port the signal handler waits on while the handler thread dumps.
static REPLY_PORT: AtomicU32 = AtomicU32::new(MACH_PORT_NULL);

/// How long the signal handler waits for the dump (a bit above the watchdog).
const ABORT_WAIT_MS: u32 = (WATCHDOG_SECS + 10) * 1000;

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

        // Reply port for the SIGABRT path (best effort: without it SIGABRT
        // keeps the event-only report).
        let mut reply: mach_port_t = MACH_PORT_NULL;
        if mach2::mach_port::mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &raw mut reply)
            == KERN_SUCCESS
        {
            if mach2::mach_port::mach_port_insert_right(task, reply, reply, MACH_MSG_TYPE_MAKE_SEND)
                == KERN_SUCCESS
            {
                REPLY_PORT.store(reply, Ordering::Release);
            } else {
                mach2::mach_port::mach_port_destroy(task, reply);
            }
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
            if let Some((thread, signo)) = exception::parse_abort(&buf.0) {
                handle_abort(thread, signo, handler);
            }
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

/// The Mach threads of this task (for [`SuspendGuard`]).
struct MachThreads;

impl Threads for MachThreads {
    type Id = thread_act_t;

    fn list(&self) -> Vec<thread_act_t> {
        // SAFETY: task_threads returns a vm-allocated array we copy and free.
        unsafe {
            let task = mach2::traps::mach_task_self();
            let mut list: mach2::mach_types::thread_act_array_t = std::ptr::null_mut();
            let mut n: u32 = 0;
            if mach2::task::task_threads(task, &raw mut list, &raw mut n) != KERN_SUCCESS {
                return Vec::new();
            }
            let threads = std::slice::from_raw_parts(list, n as usize).to_vec();
            mach2::vm::mach_vm_deallocate(
                task,
                list as mach2::vm_types::mach_vm_address_t,
                u64::from(n) * size_of::<thread_act_t>() as u64,
            );
            threads
        }
    }

    fn suspend(&self, t: thread_act_t) {
        // SAFETY: plain Mach call on a thread port of this task.
        unsafe { mach2::thread_act::thread_suspend(t) };
    }

    fn resume(&self, t: thread_act_t) {
        // SAFETY: as above.
        unsafe { mach2::thread_act::thread_resume(t) };
    }
}

fn handle_exception(req: &exception::RaiseRequest, handler: thread_t) {
    if let Some(st) = sentry::state()
        && !BUSY.swap(true, Ordering::Acquire)
    {
        // SAFETY: alarm has no preconditions. SIGALRM's default action ends
        // the process if the dump below hangs.
        unsafe { libc::alarm(WATCHDOG_SECS) };
        {
            // Every thread (the faulting one included) is resumed when the
            // guard drops, before the exception is handed back to the kernel.
            let _threads = SuspendGuard::suspend_all_except(&MachThreads, handler);
            write_report(st, req.exception, req.thread, req.task, handler);
        }
        // SAFETY: as above.
        unsafe { libc::alarm(0) };
    }
    restore_ports();
}

/// A `SIGABRT` dump request from the signal handler: dump the whole task with
/// `thread` as the crashed thread, then wake the waiting signal handler.
fn handle_abort(thread: u32, signo: i32, handler: thread_t) {
    if let Some(st) = sentry::state()
        && !BUSY.swap(true, Ordering::Acquire)
    {
        // SAFETY: as in `handle_exception`.
        unsafe { libc::alarm(WATCHDOG_SECS) };
        {
            let _threads = SuspendGuard::suspend_all_except(&MachThreads, handler);
            // SAFETY: plain Mach trap.
            let task = unsafe { mach2::traps::mach_task_self() };
            write_report(st, MachException::from_signal(signo), thread, task, handler);
        }
        // SAFETY: as above.
        unsafe { libc::alarm(0) };
    }
    notify_signal_handler();
}

/// Wake the signal handler waiting in [`request_abort_dump`].
fn notify_signal_handler() {
    let reply = REPLY_PORT.load(Ordering::Acquire);
    if reply == MACH_PORT_NULL {
        return;
    }
    let mut head = mach_msg_header_t {
        msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0),
        msgh_size: size_of::<mach_msg_header_t>() as u32,
        msgh_remote_port: reply,
        msgh_local_port: MACH_PORT_NULL,
        msgh_voucher_port: 0,
        msgh_id: exception::ABORT_MSG_ID + 1,
    };
    // SAFETY: `head` is a complete message with no body.
    unsafe {
        mach_msg(
            &raw mut head,
            MACH_SEND_MSG,
            size_of::<mach_msg_header_t>() as u32,
            0,
            MACH_PORT_NULL,
            MACH_MSG_TIMEOUT_NONE,
            MACH_PORT_NULL,
        );
    }
}

#[repr(C, packed(4))]
struct Request {
    head: mach_msg_header_t,
    body: [u8; exception::ABORT_BODY_LEN],
}

/// Called from the `SIGABRT` signal handler: ask the exception handler thread
/// to write the report with a minidump and wait for it. Returns false if the
/// handler thread is unavailable (the caller then writes the event-only
/// report itself).
pub(crate) fn request_abort_dump(signo: libc::c_int) -> bool {
    let port = PORT.load(Ordering::Acquire);
    let reply = REPLY_PORT.load(Ordering::Acquire);
    if port == MACH_PORT_NULL || reply == MACH_PORT_NULL || sentry::state().is_none() {
        return false;
    }
    if BUSY.load(Ordering::Acquire) {
        // Another crash is already being written.
        return false;
    }

    // SAFETY: mach_thread_self has no preconditions.
    let thread = unsafe { mach2::mach_init::mach_thread_self() };
    let mut req = Request {
        head: mach_msg_header_t {
            msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0),
            msgh_size: size_of::<Request>() as u32,
            msgh_remote_port: port,
            msgh_local_port: MACH_PORT_NULL,
            msgh_voucher_port: 0,
            msgh_id: exception::ABORT_MSG_ID,
        },
        body: exception::abort_body(thread, signo),
    };
    // SAFETY: `req` is a fully initialized message of the given size.
    let sent = unsafe {
        mach_msg(
            (&raw mut req).cast(),
            MACH_SEND_MSG,
            size_of::<Request>() as u32,
            0,
            MACH_PORT_NULL,
            MACH_MSG_TIMEOUT_NONE,
            MACH_PORT_NULL,
        )
    };
    if sent != 0 {
        return false;
    }

    // Block until the handler thread has finished (it resumes this thread's
    // suspension before replying). Bounded, in case the dump hangs.
    let mut buf = Buffer([0; 1024]);
    // SAFETY: `buf` is a writable 1024 byte aligned buffer.
    let received = unsafe {
        mach_msg(
            buf.0.as_mut_ptr().cast(),
            MACH_RCV_MSG | MACH_RCV_TIMEOUT,
            0,
            buf.0.len() as u32,
            reply,
            ABORT_WAIT_MS,
            MACH_PORT_NULL,
        )
    };
    // Timeout or error: the report was not confirmed, so the caller falls
    // back to the signal-path report.
    exception::abort_reply_ok(received)
}

fn write_report(
    st: &sentry::State,
    ex: MachException,
    thread: thread_t,
    task: mach_port_t,
    handler: thread_t,
) {
    // Registers and frame-pointer walk of the faulting thread.
    let mut native = Box::new(Native::new());
    (native.signo, native.code) = ex.signal();
    native.addr = ex.fault_addr();
    let mut state: ThreadState = ThreadState::default();
    let mut count = (size_of::<ThreadState>() / 4) as u32;
    // SAFETY: `state` is a writable thread state of the requested flavor.
    let kr = unsafe {
        mach2::thread_act::thread_get_state(
            thread,
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
        task,
        thread,
        handler_thread: handler,
        exception: Some(ExceptionInfo {
            kind: ex.kind,
            code: ex.code as u64,
            subcode: ex.minidump_subcode(),
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
