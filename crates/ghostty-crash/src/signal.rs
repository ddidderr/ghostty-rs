//! Fatal-signal handling for crash capture.
//!
//! This is best effort, like the Sentry native handler it replaces: the
//! process is in an undefined state when this runs. We therefore avoid heap
//! allocation and only use async-signal-safe libc calls (`open`, `write`,
//! `mkdir`, `getrandom`, `raise`, ...). All formatting goes into a static
//! buffer; a flag makes sure only one crash is written at a time.
//!
//! Unsafe is needed for `sigaction`/`sigaltstack` and the static buffer.
#![allow(unsafe_code)]

use std::{
    cell::UnsafeCell,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::sentry::{self, Crash};

const SIGNALS: [(libc::c_int, &str, &str); 5] = [
    (libc::SIGSEGV, "SIGSEGV", "Segmentation fault"),
    (libc::SIGBUS, "SIGBUS", "Bus error"),
    (libc::SIGILL, "SIGILL", "Illegal instruction"),
    (libc::SIGFPE, "SIGFPE", "Arithmetic exception"),
    (libc::SIGABRT, "SIGABRT", "Aborted"),
];

const EVENT_CAP: usize = 6 * 1024;
const HEAD_CAP: usize = 512;
const PATH_CAP: usize = 4096 + 64;

/// Static scratch space for the handler.
struct Scratch {
    event: [u8; EVENT_CAP],
    head: [u8; HEAD_CAP],
    path: [u8; PATH_CAP],
}

struct ScratchCell(UnsafeCell<Scratch>);
// SAFETY: access is serialized by `BUSY`.
unsafe impl Sync for ScratchCell {}

static SCRATCH: ScratchCell = ScratchCell(UnsafeCell::new(Scratch {
    event: [0; EVENT_CAP],
    head: [0; HEAD_CAP],
    path: [0; PATH_CAP],
}));
static BUSY: AtomicBool = AtomicBool::new(false);

/// A `fmt::Write` into a fixed byte slice; errors when full.
struct SliceWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl fmt::Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        let end = self.len.checked_add(b.len()).ok_or(fmt::Error)?;
        if end > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(b);
        self.len = end;
        Ok(())
    }
}

/// Fill `out` with random bytes using only async-signal-safe calls, falling
/// back to a time/pid mix if the kernel refuses.
pub(crate) fn random_bytes(out: &mut [u8; 16]) {
    #[cfg(not(target_vendor = "apple"))]
    {
        // SAFETY: `out` is a valid writable buffer of the given length.
        let n = unsafe { libc::getrandom(out.as_mut_ptr().cast(), out.len(), libc::GRND_NONBLOCK) };
        if usize::try_from(n).is_ok_and(|n| n == out.len()) {
            return;
        }
    }
    // Darwin has no getrandom(2); getentropy(2) fills up to 256 bytes.
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: `out` is a valid writable buffer of the given length.
        if unsafe { libc::getentropy(out.as_mut_ptr().cast(), out.len()) } == 0 {
            return;
        }
    }
    let (s, us) = sentry::now();
    // SAFETY: getpid has no preconditions.
    let pid = unsafe { libc::getpid() };
    let mut x = (s as u64) ^ (u64::from(us) << 20) ^ (u64::from(pid.unsigned_abs()) << 40);
    for chunk in out.chunks_mut(8) {
        // splitmix64
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
    }
}

fn write_all(fd: libc::c_int, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        // SAFETY: `data` is a valid readable buffer.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if n == 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// `mkdir -p` on a NUL-terminated path in `path[..len]` (mutated temporarily).
fn mkdir_p(path: &mut [u8], len: usize) {
    for i in 1..=len {
        if i == len || path[i] == b'/' {
            let saved = path[i];
            path[i] = 0;
            // SAFETY: path[..=i] is NUL terminated.
            unsafe { libc::mkdir(path.as_ptr().cast(), 0o755) };
            path[i] = saved;
        }
    }
}

/// Write a crash report for `sig`. Returns without doing anything if the
/// crash handler isn't active or another crash is being written.
fn write_signal_report(name: &str, desc: &str, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    let Some(st) = sentry::state() else { return };
    if BUSY.swap(true, Ordering::Acquire) {
        return;
    }
    // SAFETY: BUSY gives us exclusive access to the scratch space (and it is
    // never released, since the process is about to die).
    let sc = unsafe { &mut *SCRATCH.0.get() };

    let mut raw = [0u8; 16];
    random_bytes(&mut raw);
    let mut u = [0u8; 36];
    sentry::uuid_string(raw, &mut u);
    let Ok(uuid) = std::str::from_utf8(&u) else {
        return;
    };
    let (secs, micros) = sentry::now();
    let crash = Crash {
        uuid,
        secs,
        micros,
        kind: name,
        value: desc,
        mechanism: "signalhandler",
    };

    let mut ev = SliceWriter {
        buf: &mut sc.event,
        len: 0,
    };
    if sentry::write_event(&mut ev, st, &crash).is_err() {
        return;
    }
    let ev_len = ev.len;
    let mut head = SliceWriter {
        buf: &mut sc.head,
        len: 0,
    };
    if sentry::write_envelope_head(&mut head, &crash, ev_len).is_err() {
        return;
    }
    let head_len = head.len;

    // <dir>/<uuid>.ghosttycrash\0
    let ext = b".ghosttycrash";
    let total = st.dir.len() + 1 + u.len() + ext.len();
    if total + 1 > PATH_CAP {
        return;
    }
    let mut o = 0;
    for part in [st.dir.as_slice(), b"/", &u, ext] {
        sc.path[o..o + part.len()].copy_from_slice(part);
        o += part.len();
    }
    sc.path[o] = 0;

    // The directory must exist for writing to work.
    mkdir_p(&mut sc.path, st.dir.len());

    // Preferred: an envelope with a minidump of the whole process, written by
    // a forked helper that ptraces us (Linux only; see `minidump`).
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `info`/`uc` are the pointers the kernel gave the handler.
        let cc = unsafe { crate::minidump::crash_context(info, uc) };
        let cap = crate::minidump::Capture {
            path: &sc.path[..=o],
            head: &sc.head[..head_len],
            event: &sc.event[..ev_len],
        };
        if crate::minidump::capture(&cap, cc) {
            write_all(
                2,
                b"warning(sentry): crash report with minidump written to disk path=",
            );
            write_all(2, &sc.path[..o]);
            write_all(2, b"\n");
            return;
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (info, uc);
    // SAFETY: path is NUL terminated.
    let fd = unsafe {
        libc::open(
            sc.path.as_ptr().cast(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        return;
    }
    let ok = write_all(fd, &sc.head[..head_len]) && write_all(fd, &sc.event[..ev_len]);
    // SAFETY: fd is open and owned by us.
    unsafe { libc::close(fd) };
    if ok {
        write_all(2, b"warning(sentry): crash report written to disk path=");
        write_all(2, &sc.path[..o]);
        write_all(2, b"\n");
    }
}

extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    if let Some((_, name, desc)) = SIGNALS.iter().find(|(s, ..)| *s == sig) {
        write_signal_report(name, desc, info, ctx);
    }
    // Restore the default action and re-raise so the process dies with the
    // original signal (core dump, exit status) as if we were never here.
    // SAFETY: signal and raise are async-signal-safe.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// Make sure the calling thread has an alternate signal stack so that stack
/// overflows can be handled. Rust's runtime already installs one for the main
/// thread and threads it spawns, so this is usually a no-op.
fn ensure_altstack() {
    // SAFETY: sigaltstack is called with valid pointers; the new stack is
    // leaked so it lives for the whole process.
    unsafe {
        let mut cur: libc::stack_t = std::mem::zeroed();
        if libc::sigaltstack(std::ptr::null(), &raw mut cur) == 0
            && cur.ss_flags & libc::SS_DISABLE == 0
        {
            return;
        }
        let size = 64 * 1024;
        let stack: &'static mut [u8] = Box::leak(vec![0u8; size].into_boxed_slice());
        let ss = libc::stack_t {
            ss_sp: stack.as_mut_ptr().cast(),
            ss_flags: 0,
            ss_size: size,
        };
        libc::sigaltstack(&raw const ss, std::ptr::null_mut());
    }
}

/// Install handlers for the fatal signals.
pub(crate) fn install_handlers() {
    ensure_altstack();
    for (sig, ..) in SIGNALS {
        // SAFETY: a zeroed sigaction is valid to fill in; the handler is an
        // `extern "C"` fn with the SA_SIGINFO signature.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = handler as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_NODEFER;
            libc::sigemptyset(&raw mut sa.sa_mask);
            libc::sigaction(sig, &raw const sa, std::ptr::null_mut());
        }
    }
}
