//! ghostty-termio: see docs/rewrite/ARCHITECTURE.md for the module map.
//!
//! Ported so far (Linux only): `pty.zig`, `Command.zig`,
//! `termio/shell_integration.zig`, the `terminfo/*` source generator.
//! See `docs/rewrite/notes/ghostty-termio-IO1.md`.
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

pub mod command;
pub mod pty;
pub mod shell_integration;
pub mod terminfo;

pub use command::Command;
pub use pty::{Pty, Winsize};
