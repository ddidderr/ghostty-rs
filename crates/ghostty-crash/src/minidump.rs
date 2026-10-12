//! Minidump capture for fatal signals (Linux), the counterpart of what
//! sentry-native's Breakpad/crashpad backend does upstream.
//!
//! A process cannot ptrace itself, so (like Breakpad's `ExceptionHandler`)
//! the signal handler forks a short-lived child, allows it to ptrace the
//! crashing process (`PR_SET_PTRACER`), and waits. The child uses
//! `minidump-writer` to snapshot the parent -- every thread's registers and
//! stack, the module list, memory mappings and, for the crashing thread, the
//! `siginfo` + machine context captured by the handler -- and writes the final
//! envelope: the event item followed by the minidump as an
//! `event.minidump` attachment. If anything fails (ptrace forbidden by a
//! sandbox, the child dies or times out) the caller falls back to the plain
//! event-only envelope written from the signal-safe path.
//!
//! The fork uses the raw `clone` syscall without `CLONE_VM`, so no `atfork`
//! handlers (which could take locks the crashed thread holds) run. The child
//! allocates, which could in theory deadlock on a copied locked malloc arena;
//! the parent therefore enforces a timeout and kills the child.
#![allow(unsafe_code)]

use std::{
    cell::UnsafeCell,
    io::{Cursor, Write},
    mem::MaybeUninit,
    os::fd::FromRawFd,
    time::{Duration, Instant},
};

use minidump_writer::{
    CrashContextExt,
    crash_context::{CrashContext, ucontext_t},
    minidump_writer::MinidumpWriterConfig,
};

/// Stack size of the dumping child.
const STACK: usize = 4 << 20;

/// How long the crashing process waits for the dumping child.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Everything the child needs; borrowed from the handler's scratch space.
pub(crate) struct Capture<'a> {
    /// NUL-terminated destination path of the envelope.
    pub path: &'a [u8],
    /// Envelope header line + event item header line.
    pub head: &'a [u8],
    /// The event JSON payload.
    pub event: &'a [u8],
}

/// The crash context lives in static memory: the handler may run on a tiny
/// alternate stack (Rust's default is a few KiB) that cannot hold a ~2 KiB
/// struct plus the temporaries of building it.
struct CcCell(UnsafeCell<MaybeUninit<CrashContext>>);
// SAFETY: access is serialized by the caller (`signal::BUSY`).
unsafe impl Sync for CcCell {}
static CC: CcCell = CcCell(UnsafeCell::new(MaybeUninit::zeroed()));

/// Fill the static `crash-context` description of the fault from the
/// handler's arguments and return it.
///
/// # Safety
/// `info` and `uc` must be the pointers the kernel passed to an `SA_SIGINFO`
/// handler (or null), and the caller must have exclusive access to the static
/// (the crash flag `signal::BUSY`).
pub(crate) unsafe fn crash_context(
    info: *const libc::siginfo_t,
    uc: *const libc::c_void,
) -> &'static CrashContext {
    // SAFETY: exclusive access per the contract; CrashContext is plain old
    // data for which the all-zero pattern is valid.
    let cc: &'static mut CrashContext = unsafe { (*CC.0.get()).assume_init_mut() };
    // SAFETY: gettid/getpid have no preconditions.
    cc.tid = unsafe { libc::syscall(libc::SYS_gettid) } as libc::pid_t;
    // SAFETY: as above.
    cc.pid = unsafe { libc::getpid() };

    // SAFETY: per the function contract; the kernel's siginfo is valid.
    if let Some(info) = unsafe { info.as_ref() } {
        cc.siginfo.ssi_signo = info.si_signo as u32;
        cc.siginfo.ssi_code = info.si_code;
        cc.siginfo.ssi_errno = info.si_errno;
        // SAFETY: si_addr is valid for fault signals; for others it is
        // unspecified but reading it is harmless.
        cc.siginfo.ssi_addr = unsafe { info.si_addr() } as u64;
    }
    if !uc.is_null() {
        // SAFETY: the kernel passes a ucontext whose layout is arch-specific
        // and identical to `crash_context::ucontext_t` (the premise of that
        // crate); `uc` is valid for reads of that size and `cc.context` for
        // writes; they cannot overlap (static vs. signal frame).
        unsafe {
            std::ptr::copy_nonoverlapping(uc.cast::<ucontext_t>(), &raw mut cc.context, 1);
        }
        #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
        {
            let fp = cc.context.uc_mcontext.fpregs;
            if !fp.is_null() {
                // SAFETY: the kernel points fpregs at the saved FP state in
                // the signal frame; the destination is our static.
                unsafe { std::ptr::copy_nonoverlapping(fp, &raw mut cc.float_state, 1) };
            }
        }
        // AArch64 has no `fpregs` pointer: the FP/SIMD state is the
        // `fpsimd_context` record in the chain of context extension records
        // stored in `mcontext_t::__reserved`.
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: `__reserved` is plain initialized memory inside `cc`.
            let reserved: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    (&raw const cc.context.uc_mcontext.__reserved).cast::<u8>(),
                    size_of_val(&cc.context.uc_mcontext.__reserved),
                )
            };
            if let Some(off) = fpsimd_offset(reserved)
                && off + size_of::<minidump_writer::crash_context::fpregset_t>() <= reserved.len()
            {
                // SAFETY: bounds checked above; both sides are plain old
                // data and the destination is our static (unaligned source).
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        reserved.as_ptr().add(off),
                        (&raw mut cc.float_state).cast::<u8>(),
                        size_of::<minidump_writer::crash_context::fpregset_t>(),
                    );
                }
            }
        }
    }
    cc
}

/// Magic of the AArch64 `fpsimd_context` record (`FPSIMD_MAGIC`).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
const FPSIMD_MAGIC: u32 = 0x4650_8001;

/// Walk the AArch64 context extension records (`struct _aarch64_ctx`: u32
/// magic, u32 size, native endian) in `mcontext_t::__reserved` and return the
/// byte offset of the `fpsimd_context` record, if present. The chain ends at
/// a zero magic. Pure, so it is testable on every architecture.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
fn fpsimd_offset(reserved: &[u8]) -> Option<usize> {
    let mut off = 0usize;
    while off + 8 <= reserved.len() {
        let magic = u32::from_ne_bytes(reserved[off..off + 4].try_into().ok()?);
        let size = u32::from_ne_bytes(reserved[off + 4..off + 8].try_into().ok()?) as usize;
        if magic == FPSIMD_MAGIC {
            return Some(off);
        }
        if magic == 0 || size < 8 {
            return None;
        }
        off += size;
    }
    None
}

/// Fork the dumping child and wait for it. Returns true if the envelope with
/// the minidump has been written to `cap.path`.
pub(crate) fn capture(cap: &Capture<'_>, cc: &CrashContext) -> bool {
    // Allow any process (our child) to ptrace us under Yama's ptrace_scope=1.
    // SAFETY: prctl with these arguments only changes our own ptracer policy.
    unsafe {
        libc::prctl(
            libc::PR_SET_PTRACER,
            libc::PR_SET_PTRACER_ANY as libc::c_ulong,
            0,
            0,
            0,
        )
    };

    // The handler runs on a small alternate stack, so the child gets its own.
    // SAFETY: anonymous private mapping; checked for failure below.
    let stack = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            STACK,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK,
            -1,
            0,
        )
    };
    if stack == libc::MAP_FAILED {
        return false;
    }
    let mut arg = ChildArg { cap, cc };
    // SAFETY: fork-style clone (no CLONE_VM, so the child gets a private
    // copy-on-write image and `arg` stays valid in it) running `child_entry`
    // on the fresh stack; glibc's wrapper runs no atfork handlers.
    let pid = unsafe {
        libc::clone(
            child_entry,
            stack.cast::<u8>().add(STACK).cast(),
            libc::SIGCHLD,
            (&raw mut arg).cast(),
        )
    };
    if pid < 0 {
        // SAFETY: unmapping our own mapping.
        unsafe { libc::munmap(stack, STACK) };
        return false;
    }
    let r = wait_for(pid);
    // SAFETY: the child is gone; unmapping our own mapping.
    unsafe { libc::munmap(stack, STACK) };
    r
}

fn wait_for(pid: libc::pid_t) -> bool {
    let start = Instant::now();
    loop {
        let mut status = 0;
        // SAFETY: waitpid with a valid out pointer.
        let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
        if r == pid {
            return libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
        }
        if r < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return false;
        }
        if start.elapsed() > TIMEOUT {
            // SAFETY: kill/waitpid on our own child.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, &raw mut status, 0);
            }
            return false;
        }
        let ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 5_000_000,
        };
        // SAFETY: nanosleep with valid pointers.
        unsafe { libc::nanosleep(&raw const ts, std::ptr::null_mut()) };
    }
}

struct ChildArg<'a, 'b> {
    cap: &'a Capture<'b>,
    cc: &'a CrashContext,
}

extern "C" fn child_entry(arg: *mut libc::c_void) -> libc::c_int {
    // SAFETY: `arg` points to the parent's `ChildArg`, copied into this
    // process by the clone.
    let arg = unsafe { &*arg.cast::<ChildArg<'_, '_>>() };
    i32::from(!child_main(arg.cap, arg.cc))
}

fn child_main(cap: &Capture<'_>, cc: &CrashContext) -> bool {
    #[allow(clippy::result_large_err)]
    let attempt = |cc: &CrashContext| {
        let mut writer = MinidumpWriterConfig::new(cc.pid, cc.tid);
        writer.set_crash_context(CrashContextExt { inner: cc.clone() });
        let mut sink = Cursor::new(Vec::new());
        writer
            .write(&mut sink)
            .map(|d| if d.is_empty() { sink.into_inner() } else { d })
    };
    let dump = match attempt(cc) {
        Ok(d) => d,
        Err(e) => {
            // The child may allocate; say why the first try failed.
            let _ = writeln!(
                std::io::stderr(),
                "warning(sentry): minidump failed err={e}"
            );
            // A stack overflow faults with the stack pointer in the guard
            // page, which the writer cannot read a stack from. Retry with
            // the pointer moved to the first readable address above it so
            // the thread's stack is still captured.
            let mut fixed = cc.clone();
            if !clamp_stack_pointer(&mut fixed) {
                return false;
            }
            match attempt(&fixed) {
                Ok(d) => d,
                Err(e) => {
                    let _ = writeln!(
                        std::io::stderr(),
                        "warning(sentry): minidump retry failed err={e}"
                    );
                    return false;
                }
            }
        }
    };
    write_envelope(cap, &dump)
}

#[cfg(target_arch = "x86_64")]
fn sp_mut(cc: &mut CrashContext) -> &mut i64 {
    &mut cc.context.uc_mcontext.gregs[libc::REG_RSP as usize]
}
#[cfg(target_arch = "aarch64")]
fn sp_mut(cc: &mut CrashContext) -> &mut u64 {
    &mut cc.context.uc_mcontext.sp
}

/// If the page holding the stack pointer cannot be read (a stack overflow
/// faults in/at the guard page), move the pointer up to the first readable page
/// above it (within the 1 MiB the kernel's stack guard gap can span). Returns
/// whether it was changed.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn clamp_stack_pointer(cc: &mut CrashContext) -> bool {
    const PAGE: u64 = 4096;
    let pid = cc.pid;
    let readable = |addr: u64| {
        let mut b = [0u8; 8];
        let local = libc::iovec {
            iov_base: b.as_mut_ptr().cast(),
            iov_len: b.len(),
        };
        let remote = libc::iovec {
            iov_base: addr as *mut libc::c_void,
            iov_len: b.len(),
        };
        // SAFETY: both iovecs describe valid ranges (the remote one is only
        // interpreted by the kernel in the other process).
        unsafe { libc::process_vm_readv(pid, &raw const local, 1, &raw const remote, 1, 0) == 8 }
    };
    let sp = *sp_mut(cc) as u64;
    let base = sp & !(PAGE - 1);
    if readable(base) {
        return false;
    }
    let mut addr = base + PAGE;
    while addr <= base + (1 << 20) {
        if readable(addr) {
            *sp_mut(cc) = addr as _;
            return true;
        }
        addr += PAGE;
    }
    false
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn clamp_stack_pointer(_cc: &mut CrashContext) -> bool {
    false
}

fn write_envelope(cap: &Capture<'_>, dump: &[u8]) -> bool {
    // SAFETY: `path` is NUL terminated by the caller.
    let fd = unsafe {
        libc::open(
            cap.path.as_ptr().cast(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        return false;
    }
    // SAFETY: we own the fd just opened.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let item_head = format!(
        "\n{{\"type\":\"attachment\",\"length\":{},\"filename\":\"minidump.dmp\",\"attachment_type\":\"event.minidump\"}}\n",
        dump.len()
    );
    let ok = f
        .write_all(cap.head)
        .and_then(|()| f.write_all(cap.event))
        .and_then(|()| f.write_all(item_head.as_bytes()))
        .and_then(|()| f.write_all(dump))
        .and_then(|()| f.write_all(b"\n"))
        .is_ok();
    drop(f);
    if !ok {
        // SAFETY: remove the partial file so the caller can fall back.
        unsafe { libc::unlink(cap.path.as_ptr().cast()) };
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::{FPSIMD_MAGIC, fpsimd_offset};

    fn record(magic: u32, size: u32, out: &mut Vec<u8>) {
        out.extend_from_slice(&magic.to_ne_bytes());
        out.extend_from_slice(&size.to_ne_bytes());
        out.resize(out.len() + size as usize - 8, 0);
    }

    #[test]
    fn finds_fpsimd_record_in_extension_chain() {
        let mut buf = Vec::new();
        // An unrelated record (e.g. esr_context) first, then fpsimd.
        record(0x4553_5201, 16, &mut buf);
        let at = buf.len();
        record(FPSIMD_MAGIC, 528, &mut buf);
        record(0, 8, &mut buf);
        assert_eq!(fpsimd_offset(&buf), Some(at));
    }

    #[test]
    fn missing_or_corrupt_chain_yields_none() {
        let mut buf = Vec::new();
        record(0, 8, &mut buf);
        assert_eq!(fpsimd_offset(&buf), None);
        assert_eq!(fpsimd_offset(&[]), None);
        // A record with a bogus size must not loop forever.
        let mut bad = Vec::new();
        bad.extend_from_slice(&1u32.to_ne_bytes());
        bad.extend_from_slice(&0u32.to_ne_bytes());
        assert_eq!(fpsimd_offset(&bad), None);
    }
}
