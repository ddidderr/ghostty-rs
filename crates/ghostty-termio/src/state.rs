//! The shared render state (`renderer/State.zig`).
//!
//! The canonical type lives in `ghostty-renderer`
//! ([`ghostty_renderer::state::State`] behind a mutex,
//! [`SharedState`]). This module only provides a convenience for termio:
//! locking the state and borrowing just the [`Terminal`].

use std::{
    ops::{Deref, DerefMut},
    sync::{MutexGuard, PoisonError},
};

pub use ghostty_renderer::state::{SharedState, State};
use ghostty_terminal::terminal::Terminal;

/// A locked [`SharedState`] that dereferences to its [`Terminal`].
pub struct TerminalGuard<'a>(MutexGuard<'a, State>);

impl TerminalGuard<'_> {
    /// The whole locked state (preedit, mouse, ...).
    pub fn state(&mut self) -> &mut State {
        &mut self.0
    }
}

impl Deref for TerminalGuard<'_> {
    type Target = Terminal;
    fn deref(&self) -> &Terminal {
        &self.0.terminal
    }
}

impl DerefMut for TerminalGuard<'_> {
    fn deref_mut(&mut self) -> &mut Terminal {
        &mut self.0.terminal
    }
}

/// Lock helpers for [`SharedState`].
pub trait SharedStateExt {
    /// Lock the state and borrow the terminal. Lock poisoning is ignored: a
    /// panic on another thread must not take the whole terminal down with
    /// it (Zig mutexes don't poison).
    fn lock_terminal(&self) -> TerminalGuard<'_>;
}

impl SharedStateExt for SharedState {
    fn lock_terminal(&self) -> TerminalGuard<'_> {
        TerminalGuard(self.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
