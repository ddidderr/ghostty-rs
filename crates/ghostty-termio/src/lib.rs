//! ghostty-termio: see docs/rewrite/ARCHITECTURE.md for the module map.
//!
//! Linux only. Ported: `pty.zig`, `Command.zig`, the `terminfo/*` source
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
pub mod command;
pub mod exec;
pub mod mailbox;
pub mod message;
pub mod options;
pub mod pty;
pub mod shell_integration;
pub mod sinks;
pub mod state;
pub mod stream_handler;
pub mod terminfo;
pub mod termio;
pub mod thread;
pub mod wakeup;

pub use backend::Backend;
pub use command::Command;
pub use exec::Exec;
pub use mailbox::Mailbox;
pub use message::Message;
pub use options::Options;
pub use pty::{Pty, Winsize};
pub use sinks::{RendererMailbox, SurfaceMailbox};
pub use state::RendererState;
pub use stream_handler::StreamHandler;
pub use termio::{DerivedConfig, Termio};
pub use thread::Thread;
pub use wakeup::Wakeup;
