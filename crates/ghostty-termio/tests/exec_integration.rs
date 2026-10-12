#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! End-to-end tests: real child processes on a real pty, driven through
//! `Termio` + `Thread` (IO thread) + the pty read thread, against a real
//! `Terminal`.
//!
//! The whole suite (`exec_integration/suite.rs`) runs once per IO thread
//! event loop backend (`async-backend`): `epoll`, `io_uring` (skipped, with a
//! message, where the kernel/sandbox denies it) and the portable `poll(2)`
//! loop used where libxev has a single different backend (kqueue).

macro_rules! suite {
    ($name:ident, $be:expr) => {
        mod $name {
            const UNDER_TEST: ghostty_os::xev::Backend = $be;
            include!("exec_integration/suite.rs");
        }
    };
}

// `Kqueue` is not a candidate on Linux, so the reactor uses `poll(2)`.
suite!(poll, ghostty_os::xev::Backend::Kqueue);
#[cfg(target_os = "linux")]
suite!(epoll, ghostty_os::xev::Backend::Epoll);
#[cfg(target_os = "linux")]
suite!(io_uring, ghostty_os::xev::Backend::IoUring);
