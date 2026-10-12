//! The options that are used to configure a renderer (`renderer/Options.zig`).
use std::sync::Arc;

use ghostty_font::SharedGrid;

use crate::{generic::Host, size::Size};

/// Options for constructing a renderer.
///
/// `config` is the derived renderer configuration (`Renderer.DerivedConfig`).
///
/// Zig also carries `rt_surface` (used by the graphics API to prepare the
/// GL context) and `thread`; in Rust the apprt prepares the GL context
/// itself (`OpenGL::prepare_context`) before constructing the renderer, and
/// the thread is wired up by [`crate::thread::Thread`].
pub struct Options<C> {
    /// The derived configuration for this renderer implementation.
    pub config: C,

    /// The font grid that should be used along with the key for deref-ing.
    pub font_grid: Arc<SharedGrid>,

    /// The size of everything.
    pub size: Size,

    /// The surface mailbox / apprt callbacks (Zig: `surface_mailbox`).
    pub host: Arc<dyn Host>,
}
