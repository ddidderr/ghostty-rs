//! `QuickTerminalPosition.swift`: the `quick-terminal-position` value.
//! The window animation helpers (`setLoaded`, `setInitial`, `animateIn`, ...)
//! are AppKit and belong to MA4 (TODO(MA4)).

/// `enum QuickTerminalPosition: String`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum QuickTerminalPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Center,
}

impl QuickTerminalPosition {
    /// `init?(rawValue:)`.
    #[must_use]
    pub fn from_raw_value(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "center" => Some(Self::Center),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
            Self::Center => "center",
        }
    }
}
