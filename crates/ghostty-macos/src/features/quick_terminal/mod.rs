//! `Sources/Features/QuickTerminal` (owner: MA4).
//!
//! MA1 ports only what the foundation needs: the pure config value types
//! ([`QuickTerminalPosition`], [`QuickTerminalScreen`], [`QuickTerminalSize`],
//! [`QuickTerminalSpaceBehavior`]) and the controller surface `AppDelegate`
//! uses (`controller`), which MA4 replaces.

#[cfg(target_vendor = "apple")]
pub mod controller;
pub mod position;
pub mod screen;
pub mod size;
pub mod space_behavior;

pub use position::QuickTerminalPosition;
pub use screen::QuickTerminalScreen;
pub use size::QuickTerminalSize;
pub use space_behavior::QuickTerminalSpaceBehavior;

/// `Notification.Name.quickTerminalDidChangeVisibility`.
pub const QUICK_TERMINAL_DID_CHANGE_VISIBILITY: crate::helpers::notification_center::Name =
    crate::helpers::notification_center::Name("QuickTerminalDidChangeVisibility");
