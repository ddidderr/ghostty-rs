//! The options that are used to configure a terminal IO implementation
//! (`termio/Options.zig`).

use std::sync::Arc;

use ghostty_config::config::Config;
use ghostty_renderer::size::Size;

use crate::backend::Backend;
use crate::mailbox::Mailbox;
use crate::sinks::{RendererMailbox, SurfaceMailbox};
use crate::termio::DerivedConfig;
use crate::wakeup::Wakeup;

/// See the module docs.
pub struct Options<'a> {
    /// All size metrics for the terminal.
    pub size: Size,

    /// The full app configuration. This is only available during
    /// initialization; any values needed later must be copied.
    pub full_config: &'a Config,

    /// The derived configuration for this termio implementation.
    pub config: DerivedConfig,

    /// The backend for termio that implements where reads/writes are sourced.
    pub backend: Backend,

    /// The mailbox for the terminal. This is how messages are delivered.
    /// If you're using `termio::Thread` this MUST be "mailbox" (keep a clone
    /// to send messages to the IO thread).
    pub mailbox: Mailbox,

    /// A handle to wake up the renderer. This hints to the renderer that that
    /// a repaint should happen.
    ///
    /// (The render state itself is created by `Termio::init`; get it with
    /// `Termio::renderer_state()`.)
    pub renderer_wakeup: Wakeup,

    /// The mailbox for renderer messages.
    pub renderer_mailbox: Arc<dyn RendererMailbox>,

    /// The mailbox for sending the surface messages.
    pub surface_mailbox: Arc<dyn SurfaceMailbox>,
}
