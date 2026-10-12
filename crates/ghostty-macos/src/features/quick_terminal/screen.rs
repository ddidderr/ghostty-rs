//! `QuickTerminalScreen.swift`: the `quick-terminal-screen` value.
//! The `screen` accessor (NSScreen lookup) is AppKit: TODO(MA4).

/// `enum QuickTerminalScreen`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum QuickTerminalScreen {
    #[default]
    Main,
    Mouse,
    MenuBar,
}

impl QuickTerminalScreen {
    /// `init?(fromGhosttyConfig:)`.
    #[must_use]
    pub fn from_ghostty_config(s: &str) -> Option<Self> {
        match s {
            "main" => Some(Self::Main),
            "mouse" => Some(Self::Mouse),
            "macos-menu-bar" => Some(Self::MenuBar),
            _ => None,
        }
    }
}
