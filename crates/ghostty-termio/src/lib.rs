//! ghostty-termio: see docs/rewrite/ARCHITECTURE.md for the module map.
//!
//! Linux, with a Windows (ConPTY) port behind `cfg(windows)` (the `*_windows.rs`
//! files, selected with `#[path]`). Ported: `pty.zig`, `Command.zig`, the `terminfo/*` source
//! generator and everything in `termio/` (`Options`, `backend`, `message`,
//! `mailbox`, `Termio`, `Thread`, `Exec`, `stream_handler`,
//! `shell_integration`). See `docs/rewrite/notes/ghostty-termio-IO1.md` and
//! `ghostty-termio-IO2.md`.
//!
//! This crate may use limited `unsafe` (fork/exec in `command`, a few
//! async-signal-safe libc calls in `pty`); everything else is safe.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::assert_is_empty,
        clippy::items_after_statements
    )
)]

pub mod backend;
#[cfg_attr(windows, path = "command_windows.rs")]
pub mod command;
#[cfg_attr(windows, path = "exec_windows.rs")]
pub mod exec;
pub mod mailbox;
pub mod message;
pub mod options;
#[cfg_attr(windows, path = "pty_windows.rs")]
pub mod pty;
#[cfg(unix)]
pub mod reactor;
pub mod shell_integration;
pub mod sinks;
pub mod state;
pub mod stream_handler;
pub mod terminfo;
pub mod termio;
pub mod thread;
#[cfg_attr(windows, path = "wakeup_windows.rs")]
pub mod wakeup;
/// Pure Windows `CreateProcessW` input encoders (tested on every host).
pub mod windows_args;

pub use backend::Backend;
pub use command::Command;
pub use exec::Exec;
pub use mailbox::Mailbox;
pub use message::Message;
pub use options::Options;
pub use pty::{Pty, Winsize};
pub use sinks::{RendererMailbox, SurfaceMailbox};
pub use state::{SharedState, SharedStateExt, State, TerminalGuard};
pub use stream_handler::StreamHandler;
pub use termio::{DerivedConfig, Termio};
pub use thread::Thread;
pub use wakeup::Wakeup;
