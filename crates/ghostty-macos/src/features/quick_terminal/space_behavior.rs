//! `QuickTerminalSpaceBehavior.swift`: the `quick-terminal-space-behavior` value.

/// `enum QuickTerminalSpaceBehavior`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum QuickTerminalSpaceBehavior {
    Remain,
    #[default]
    Move,
}

impl QuickTerminalSpaceBehavior {
    /// `init?(fromGhosttyConfig:)`.
    #[must_use]
    pub fn from_ghostty_config(s: &str) -> Option<Self> {
        match s {
            "move" => Some(Self::Move),
            "remain" => Some(Self::Remain),
            _ => None,
        }
    }
}

#[cfg(target_vendor = "apple")]
impl QuickTerminalSpaceBehavior {
    /// `collectionBehavior`.
    #[must_use]
    pub fn collection_behavior(self) -> objc2_app_kit::NSWindowCollectionBehavior {
        use objc2_app_kit::NSWindowCollectionBehavior as B;
        let common = B::IgnoresCycle | B::FullScreenAuxiliary;
        match self {
            // We want this to move the window to the active space.
            Self::Move => B::CanJoinAllSpaces | common,
            // We want this to remain the window in the current space.
            Self::Remain => B::MoveToActiveSpace | common,
        }
    }
}
