//! Backend implementations (`termio/backend.zig`). A backend is responsible
//! for owning the pty behavior and providing read/write capabilities.

use std::sync::Arc;

use ghostty_renderer::size::{GridSize, ScreenSize};
use ghostty_terminal::terminal::Terminal;

use crate::{
    exec::{self, Exec},
    sinks::SurfaceMailbox,
    state::SharedState,
    termio::Output,
};

/// The kinds of backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Exec,
}

/// Configuration for the various backend types.
pub enum Config {
    /// Exec uses posix exec to run a command with a pty.
    Exec(exec::Config),
}

/// Backend implementations.
pub enum Backend {
    Exec(Exec),
}

impl Backend {
    pub fn new(config: Config) -> Result<Backend, exec::Error> {
        match config {
            Config::Exec(cfg) => Ok(Backend::Exec(Exec::new(cfg)?)),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Backend::Exec(_) => Kind::Exec,
        }
    }

    pub fn init_terminal(&mut self, t: &mut Terminal) {
        match self {
            Backend::Exec(e) => e.init_terminal(t),
        }
    }

    pub fn thread_enter(
        &mut self,
        output: Arc<Output>,
        surface_mailbox: Arc<dyn SurfaceMailbox>,
        renderer_state: SharedState,
    ) -> Result<ThreadData, exec::Error> {
        match self {
            Backend::Exec(e) => Ok(ThreadData::Exec(e.thread_enter(
                output,
                surface_mailbox,
                renderer_state,
            )?)),
        }
    }

    pub fn thread_exit(&mut self, td: &mut ThreadData) {
        match (self, td) {
            (Backend::Exec(e), ThreadData::Exec(td)) => e.thread_exit(td),
        }
    }

    pub fn focus_gained(&mut self, td: &mut ThreadData, focused: bool) {
        match (self, td) {
            (Backend::Exec(e), ThreadData::Exec(td)) => e.focus_gained(td, focused),
        }
    }

    pub fn resize(
        &mut self,
        grid_size: GridSize,
        screen_size: ScreenSize,
    ) -> Result<(), exec::Error> {
        match self {
            Backend::Exec(e) => e.resize(grid_size, screen_size),
        }
    }

    pub fn queue_write(&mut self, td: &mut ThreadData, data: &[u8], linefeed: bool) {
        match (self, td) {
            (Backend::Exec(e), ThreadData::Exec(td)) => e.queue_write(td, data, linefeed),
        }
    }
}

/// Termio thread data. See `termio::ThreadData` for docs.
pub enum ThreadData {
    Exec(exec::ThreadData),
}

impl ThreadData {
    pub fn kind(&self) -> Kind {
        match self {
            ThreadData::Exec(_) => Kind::Exec,
        }
    }

    pub fn as_exec(&self) -> &exec::ThreadData {
        match self {
            ThreadData::Exec(e) => e,
        }
    }

    pub fn as_exec_mut(&mut self) -> &mut exec::ThreadData {
        match self {
            ThreadData::Exec(e) => e,
        }
    }

    /// `ThreadData.changeConfig`: nothing to do for the exec backend.
    pub fn change_config(&mut self, _config: &crate::termio::DerivedConfig) {}
}
