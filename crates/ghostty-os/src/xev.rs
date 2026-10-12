//! Runtime selection of the async event-loop backend (libxev's
//! `Dynamic` API: `detect`, `prefer`, `backend`; `Backend.candidates`).
//!
//! On Linux the candidates are `io_uring` then `epoll`; every other platform
//! has exactly one backend (kqueue / IOCP / wasi poll), for which libxev's
//! dynamic API collapses to the static one: nothing can be selected.
//!
//! The selection is process-global, like the `pub var backend` in libxev's
//! `dynamic.zig`. It defaults to the *last* candidate (`epoll`) without
//! testing availability; call [`detect`] at startup (`global.zig`) and
//! optionally [`prefer`] (GTK `Application.startupXev`) before any loop is
//! created.

use std::sync::atomic::{AtomicU8, Ordering};

/// Zig: `xev.Backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    IoUring,
    Epoll,
    Kqueue,
    WasiPoll,
    Iocp,
}

impl Backend {
    /// `@tagName`.
    pub fn name(self) -> &'static str {
        match self {
            Backend::IoUring => "io_uring",
            Backend::Epoll => "epoll",
            Backend::Kqueue => "kqueue",
            Backend::WasiPoll => "wasi_poll",
            Backend::Iocp => "iocp",
        }
    }

    fn to_u8(self) -> u8 {
        self as u8
    }

    fn from_u8(v: u8) -> Backend {
        match v {
            0 => Backend::IoUring,
            1 => Backend::Epoll,
            2 => Backend::Kqueue,
            3 => Backend::WasiPoll,
            _ => Backend::Iocp,
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Zig: `Backend.candidates()` in priority order.
pub fn candidates() -> &'static [Backend] {
    #[cfg(target_os = "linux")]
    {
        &[Backend::IoUring, Backend::Epoll]
    }
    #[cfg(target_os = "windows")]
    {
        &[Backend::Iocp]
    }
    #[cfg(target_arch = "wasm32")]
    {
        &[Backend::WasiPoll]
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_arch = "wasm32")))]
    {
        &[Backend::Kqueue]
    }
}

/// Zig: `xev.dynamic` (more than one candidate).
pub fn is_dynamic() -> bool {
    candidates().len() > 1
}

static BACKEND: AtomicU8 = AtomicU8::new(0);
static INIT: std::sync::Once = std::sync::Once::new();

fn init_default() {
    INIT.call_once(|| {
        let last = *candidates().last().unwrap_or(&Backend::Epoll);
        BACKEND.store(last.to_u8(), Ordering::SeqCst);
    });
}

/// The backend in use (Zig: `xev.backend`).
pub fn backend() -> Backend {
    init_default();
    Backend::from_u8(BACKEND.load(Ordering::SeqCst))
}

/// Zig: `Api().available()`.
pub fn available(be: Backend) -> bool {
    match be {
        Backend::Epoll => cfg!(target_os = "linux"),
        Backend::IoUring => io_uring_available(),
        Backend::Kqueue => cfg!(any(target_vendor = "apple", target_os = "freebsd")),
        Backend::WasiPoll => cfg!(target_arch = "wasm32"),
        Backend::Iocp => cfg!(windows),
    }
}

/// libxev's `io_uring.available()`: a harmless `IoUring.init(256, 0)` (an
/// `io_uring_setup` with 256 entries and no flags) that either succeeds or
/// not. Nothing more is required; a reactor that wants a feature the ring
/// lacks (such as the `EXT_ARG` wait timeout) has to cope with its absence.
#[cfg(target_os = "linux")]
fn io_uring_available() -> bool {
    io_uring::IoUring::new(256).is_ok()
}

#[cfg(not(target_os = "linux"))]
fn io_uring_available() -> bool {
    false
}

/// There is no available backend (Zig: `error.NoAvailableBackends`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("NoAvailableBackends")]
pub struct NoAvailableBackends;

/// Detect the preferred backend: the first available candidate.
pub fn detect() -> Result<(), NoAvailableBackends> {
    init_default();
    for &be in candidates() {
        if available(be) {
            BACKEND.store(be.to_u8(), Ordering::SeqCst);
            return Ok(());
        }
    }
    Err(NoAvailableBackends)
}

/// Manually set the backend to use. If the backend is not a candidate or not
/// available the backend in use is unchanged and false is returned.
pub fn prefer(be: Backend) -> bool {
    init_default();
    if !candidates().contains(&be) || !available(be) {
        return false;
    }
    BACKEND.store(be.to_u8(), Ordering::SeqCst);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process-global is shared between tests.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn names_match_zig_tags() {
        assert_eq!(Backend::IoUring.name(), "io_uring");
        assert_eq!(Backend::Epoll.to_string(), "epoll");
    }

    #[test]
    fn detect_picks_first_available_candidate() {
        let _g = LOCK.lock().unwrap();
        detect().unwrap();
        let expected = candidates()
            .iter()
            .copied()
            .find(|b| available(*b))
            .unwrap();
        assert_eq!(backend(), expected);
    }

    #[test]
    fn prefer_rejects_non_candidates() {
        let _g = LOCK.lock().unwrap();
        detect().unwrap();
        let before = backend();
        assert!(!prefer(Backend::Iocp));
        assert_eq!(backend(), before);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn io_uring_available_is_plain_setup() {
        // libxev's check is exactly a successful `IoUring.init(256, 0)`.
        assert_eq!(
            available(Backend::IoUring),
            io_uring::IoUring::new(256).is_ok()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prefer_epoll_then_detect() {
        let _g = LOCK.lock().unwrap();
        assert!(prefer(Backend::Epoll));
        assert_eq!(backend(), Backend::Epoll);
        if available(Backend::IoUring) {
            assert!(prefer(Backend::IoUring));
            assert_eq!(backend(), Backend::IoUring);
        } else {
            let before = backend();
            assert!(!prefer(Backend::IoUring));
            assert_eq!(backend(), before);
        }
        detect().unwrap();
    }
}
