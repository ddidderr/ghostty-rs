//! A link is a clickable element that can be used to trigger some action.
//! A link is NOT just a URL that opens in a browser. A link is any generic
//! regular expression match over terminal text that can trigger various
//! action types. (ZIG/src/input/Link.zig)

use crate::key_mods::Mods;

/// The action that will be triggered when the link is clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the full matched value using the default open program.
    /// For example, on macOS this is "open" and on Linux this is "xdg-open".
    Open,

    /// Open the OSC8 hyperlink under the mouse position. _-prefixed means
    /// this can't be user-specified, it's only used internally.
    _OpenOsc8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Highlight {
    /// Always highlight the link.
    Always,

    /// Only highlight the link when the mouse is hovering over it.
    Hover,

    /// Highlight anytime the given mods are pressed, either when
    /// hovering or always. For always, all links will be highlighted
    /// when the mods are pressed regardless of if the mouse is hovering
    /// over them.
    ///
    /// Note that if "shift" is specified here, this will NEVER match in
    /// TUI programs that capture mouse events. "Shift" with mouse capture
    /// escapes the mouse capture but strips the "shift" so it can't be
    /// detected.
    AlwaysMods(Mods),
    HoverMods(Mods),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The regular expression that will be used to match the link.
    pub regex: String,

    /// The action that will be triggered when the link is clicked.
    pub action: Action,

    /// The situations in which the link will be highlighted. A link is only
    /// clickable by the mouse when it is highlighted, so this also controls
    /// when the link is clickable.
    pub highlight: Highlight,
}

// TODO(integration): `oni_regex` (Zig `oniRegex`) builds an Oniguruma regex;
// the regex engine choice is made by the consumer crate (config/core).

impl Link {
    /// Check if two links are equal.
    pub fn equal(&self, other: &Link) -> bool {
        self == other
    }
}
