//! The shared render state (`renderer/State.zig`, the part termio needs).
//!
//! In Zig the terminal is owned by `Termio` and `renderer.State` holds a
//! mutex plus a pointer to it. In Rust the terminal lives inside the mutex,
//! shared via `Arc` between the IO thread, the pty read thread, the
//! surface (core) thread and the renderer thread.
//!
//! TODO(integration): `renderer/State.zig` is deferred in ghostty-renderer;
//! when it lands, this type should move there (and gain `inspector`,
//! `preedit`, `mouse`).

use std::sync::{Mutex, MutexGuard, PoisonError};

use ghostty_terminal::terminal::Terminal;

/// The state shared with the renderer.
#[derive(Debug)]
pub struct RendererState {
    /// The terminal emulator state. Hold the lock for as short as possible;
    /// it is contended by the pty read thread.
    pub terminal: Mutex<Terminal>,
}

impl RendererState {
    pub fn new(terminal: Terminal) -> Self {
        Self {
            terminal: Mutex::new(terminal),
        }
    }

    /// Lock the terminal. Lock poisoning is ignored: a panic on another
    /// thread must not take the whole terminal down with it (Zig mutexes
    /// don't poison).
    pub fn lock(&self) -> MutexGuard<'_, Terminal> {
        self.terminal.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
