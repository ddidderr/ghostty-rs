//! The options that are used to configure a renderer (`renderer/Options.zig`).
use std::sync::Arc;

use ghostty_font::SharedGrid;

use crate::size::Size;

/// Options for constructing a renderer.
///
/// `config` is the derived renderer configuration (`Renderer.DerivedConfig`).
// TODO(integration): upstream also carries `surface_mailbox`, `rt_surface`
// and `thread` (apprt / Thread.zig types). They are added when the generic
// renderer and the apprt exist; the GL loader closure replaces `rt_surface`.
pub struct Options<C> {
    /// The derived configuration for this renderer implementation.
    pub config: C,

    /// The font grid that should be used along with the key for deref-ing.
    pub font_grid: Arc<SharedGrid>,

    /// The size of everything.
    pub size: Size,
}
