//! Messages that can be sent to a renderer thread (`renderer/message.zig`).
use std::sync::Arc;

use ghostty_font::SharedGrid;

use crate::size::Size;

/// Key identifying a font grid in the `SharedGridSet`.
// TODO(integration): replace with ghostty_font's SharedGridSet key once
// SharedGridSet is ported (it needs ghostty-config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharedGridKey(pub u64);

/// The renderer-thread derived config (`renderer.Thread.DerivedConfig`).
// TODO(integration): defined with Thread.zig (deferred; needs ghostty-config).
#[derive(Debug, Clone, Default)]
pub struct ThreadDerivedConfig {}

/// The renderer implementation derived config (`Renderer.DerivedConfig`).
// TODO(integration): defined with generic.zig (deferred; needs ghostty-config).
#[derive(Debug, Clone, Default)]
pub struct RendererDerivedConfig {}

/// A flattened highlight (`terminal.highlight.Flattened`), used for search matches.
// TODO(integration): replace with ghostty_terminal::highlight::Flattened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flattened {}

/// The messages that can be sent to a renderer thread.
pub enum Message {
    /// Purposely crash the renderer. This is used for testing and debugging.
    /// See the "crash" binding action.
    Crash,

    /// A change in state in the window focus that this renderer is
    /// rendering within. This is only sent when a change is detected so
    /// the renderer is expected to handle all of these.
    Focus(bool),

    /// A change in the view occlusion state. This can be used to determine
    /// if the window is visible or not. A window can be not visible (occluded)
    /// and still have focus.
    Visible(bool),

    /// Reset the cursor blink by immediately showing the cursor then
    /// restarting the timer.
    ResetCursorBlink,

    /// Change the font grid. This can happen for any number of reasons
    /// including a font size change, family change, etc.
    FontGrid(FontGridChange),

    /// Changes the size. The screen size might change, padding, grid, etc.
    Resize(Size),

    /// The derived configuration to update the renderer with.
    ChangeConfig {
        thread: Box<ThreadDerivedConfig>,
        r#impl: Box<RendererDerivedConfig>,
    },

    /// Matches for the current viewport from the search thread. These happen
    /// async so they may be off for a frame or two from the actually rendered
    /// viewport. The renderer must handle this gracefully.
    SearchViewportMatches(Vec<Flattened>),

    /// The selected match from the search thread. May be None to indicate
    /// no match currently.
    SearchSelectedMatch(Option<Flattened>),

    /// Activate or deactivate the inspector.
    Inspector(bool),

    /// The macOS display ID has changed for the window.
    MacosDisplayId(u32),
}

/// Payload of [`Message::FontGrid`].
pub struct FontGridChange {
    pub grid: Arc<SharedGrid>,

    /// The key for the new grid. If adopting the new grid fails for any
    /// reason, the old grid should be kept but the new key should be
    /// dereferenced.
    pub new_key: SharedGridKey,

    /// After accepting the new grid, the old grid must be dereferenced
    /// using this key.
    pub old_key: SharedGridKey,
    // TODO(integration): upstream also carries `set: *SharedGridSet`.
}

impl Message {
    /// Initialize a change_config message.
    // TODO(integration): `initChangeConfig(alloc, config)` builds the derived
    // configs from `config.Config`; add once the derived config types exist.
    pub fn init_change_config(
        thread: ThreadDerivedConfig,
        r#impl: RendererDerivedConfig,
    ) -> Message {
        Message::ChangeConfig {
            thread: Box::new(thread),
            r#impl: Box::new(r#impl),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_config_message_roundtrip() {
        let m = Message::init_change_config(ThreadDerivedConfig {}, RendererDerivedConfig {});
        assert!(matches!(m, Message::ChangeConfig { .. }));
    }
}
