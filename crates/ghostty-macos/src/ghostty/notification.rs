//! The notification names and `userInfo` keys of `GhosttyPackage.swift`
//! (`extension Notification.Name` and `extension Ghostty.Notification`), with
//! the same strings as the Swift app.
//!
//! Post and observe them through
//! [`NotificationCenter`](crate::helpers::notification_center::NotificationCenter);
//! `userInfo` values keep their Rust type (see that module).
//!
//! The Swift code has two namespaces (`Notification.Name.*` and the older
//! `Ghostty.Notification.*`); both are flattened here, the Swift spelling is
//! given in each doc comment.

use crate::helpers::notification_center::Name;

// MARK: Notification.Name

/// Configuration change. If the object is nil then it is app-wide. Otherwise its surface-specific.
pub const GHOSTTY_CONFIG_DID_CHANGE: Name = Name("com.mitchellh.ghostty.configDidChange");
/// `GhosttyConfigChangeKey` (value: `Rc<Config>`).
pub const GHOSTTY_CONFIG_CHANGE_KEY: &str = GHOSTTY_CONFIG_DID_CHANGE.0;

/// Color change. Object is the surface changing.
pub const GHOSTTY_COLOR_DID_CHANGE: Name = Name("com.mitchellh.ghostty.ghosttyColorDidChange");
/// `GhosttyColorChangeKey` (value: `action::ColorChange`).
pub const GHOSTTY_COLOR_CHANGE_KEY: &str = GHOSTTY_COLOR_DID_CHANGE.0;

/// Goto tab. Has tab index in the userinfo.
pub const GHOSTTY_MOVE_TAB: Name = Name("com.mitchellh.ghostty.moveTab");
/// `GhosttyMoveTabKey` (value: `action::MoveTab`).
pub const GHOSTTY_MOVE_TAB_KEY: &str = GHOSTTY_MOVE_TAB.0;

/// Close tab
pub const GHOSTTY_CLOSE_TAB: Name = Name("com.mitchellh.ghostty.closeTab");

/// Close other tabs
pub const GHOSTTY_CLOSE_OTHER_TABS: Name = Name("com.mitchellh.ghostty.closeOtherTabs");

/// Close tabs to the right of the focused tab
pub const GHOSTTY_CLOSE_TABS_ON_THE_RIGHT: Name = Name("com.mitchellh.ghostty.closeTabsOnTheRight");

/// Close window
pub const GHOSTTY_CLOSE_WINDOW: Name = Name("com.mitchellh.ghostty.closeWindow");

/// Resize the window to a default size.
pub const GHOSTTY_RESET_WINDOW_SIZE: Name = Name("com.mitchellh.ghostty.resetWindowSize");

/// Ring the bell
pub const GHOSTTY_BELL_DID_RING: Name = Name("com.mitchellh.ghostty.ghosttyBellDidRing");

/// Readonly mode changed
pub const GHOSTTY_DID_CHANGE_READONLY: Name = Name("com.mitchellh.ghostty.didChangeReadonly");
/// `ReadonlyKey` (value: `bool`).
pub const READONLY_KEY: &str = "com.mitchellh.ghostty.didChangeReadonly.readonly";
pub const GHOSTTY_COMMAND_PALETTE_DID_TOGGLE: Name =
    Name("com.mitchellh.ghostty.commandPaletteDidToggle");

/// Toggle maximize of current window
pub const GHOSTTY_MAXIMIZE_DID_TOGGLE: Name = Name("com.mitchellh.ghostty.maximizeDidToggle");

/// Notification sent when scrollbar updates
pub const GHOSTTY_DID_UPDATE_SCROLLBAR: Name = Name("com.mitchellh.ghostty.didUpdateScrollbar");
/// `ScrollbarKey` (value: `action::Scrollbar`).
pub const SCROLLBAR_KEY: &str = "com.mitchellh.ghostty.didUpdateScrollbar.scrollbar";

/// Focus the search field
pub const GHOSTTY_SEARCH_FOCUS: Name = Name("com.mitchellh.ghostty.searchFocus");

// MARK: Ghostty.Notification

/// Used to pass a configuration along when creating a new tab/window/split
/// (value: `SurfaceConfiguration`).
pub const NEW_SURFACE_CONFIG_KEY: &str = "com.mitchellh.ghostty.newSurfaceConfig";

/// Posted when a new split is requested. The sending object will be the surface that had focus. The
/// userdata has one key "direction" with the direction to split to.
pub const GHOSTTY_NEW_SPLIT: Name = Name("com.mitchellh.ghostty.newSplit");

/// Close the calling surface.
pub const GHOSTTY_CLOSE_SURFACE: Name = Name("com.mitchellh.ghostty.closeSurface");

/// Focus previous/next split. Has a SplitFocusDirection in the userinfo.
pub const GHOSTTY_FOCUS_SPLIT: Name = Name("com.mitchellh.ghostty.focusSplit");
/// `SplitDirectionKey`.
pub const SPLIT_DIRECTION_KEY: &str = GHOSTTY_FOCUS_SPLIT.0;

/// Goto tab. Has tab index in the userinfo.
pub const GHOSTTY_GOTO_TAB: Name = Name("com.mitchellh.ghostty.gotoTab");
/// `GotoTabKey`.
pub const GOTO_TAB_KEY: &str = GHOSTTY_GOTO_TAB.0;

/// New tab. Has base surface config requested in userinfo.
pub const GHOSTTY_NEW_TAB: Name = Name("com.mitchellh.ghostty.newTab");

/// New window. Has base surface config requested in userinfo.
pub const GHOSTTY_NEW_WINDOW: Name = Name("com.mitchellh.ghostty.newWindow");

/// Present terminal. Bring the surface's window to focus without activating the app.
pub const GHOSTTY_PRESENT_TERMINAL: Name = Name("com.mitchellh.ghostty.presentTerminal");

/// Toggle fullscreen of current window
pub const GHOSTTY_TOGGLE_FULLSCREEN: Name = Name("com.mitchellh.ghostty.toggleFullscreen");
/// `FullscreenModeKey`.
pub const FULLSCREEN_MODE_KEY: &str = GHOSTTY_TOGGLE_FULLSCREEN.0;

/// Notification sent to toggle split maximize/unmaximize.
pub const DID_TOGGLE_SPLIT_ZOOM: Name = Name("com.mitchellh.ghostty.didToggleSplitZoom");

/// Notification
pub const DID_RECEIVE_INITIAL_WINDOW_FRAME: Name =
    Name("com.mitchellh.ghostty.didReceiveInitialWindowFrame");
/// `FrameKey`.
pub const FRAME_KEY: &str = "com.mitchellh.ghostty.frame";

/// Notification to render the inspector for a surface
pub const INSPECTOR_NEEDS_DISPLAY: Name = Name("com.mitchellh.ghostty.inspectorNeedsDisplay");

/// Notification to show/hide the inspector
pub const DID_CONTROL_INSPECTOR: Name = Name("com.mitchellh.ghostty.didControlInspector");

pub const CONFIRM_CLIPBOARD: Name = Name("com.mitchellh.ghostty.confirmClipboard");
/// `ConfirmClipboardStrKey` (value: `String`).
pub const CONFIRM_CLIPBOARD_STR_KEY: &str = "com.mitchellh.ghostty.confirmClipboard.str";
/// `ConfirmClipboardStateKey` (value: the opaque request state pointer as `usize`).
pub const CONFIRM_CLIPBOARD_STATE_KEY: &str = "com.mitchellh.ghostty.confirmClipboard.state";
/// `ConfirmClipboardRequestKey` (value: `package::ClipboardRequest`).
pub const CONFIRM_CLIPBOARD_REQUEST_KEY: &str = "com.mitchellh.ghostty.confirmClipboard.request";

/// Notification sent to the active split view to resize the split.
pub const DID_RESIZE_SPLIT: Name = Name("com.mitchellh.ghostty.didResizeSplit");
/// `ResizeSplitDirectionKey`.
pub const RESIZE_SPLIT_DIRECTION_KEY: &str = "com.mitchellh.ghostty.didResizeSplit.direction";
/// `ResizeSplitAmountKey`.
pub const RESIZE_SPLIT_AMOUNT_KEY: &str = "com.mitchellh.ghostty.didResizeSplit.amount";

/// Notification sent to the split root to equalize split sizes
pub const DID_EQUALIZE_SPLITS: Name = Name("com.mitchellh.ghostty.didEqualizeSplits");

/// Notification that renderer health changed
pub const DID_UPDATE_RENDERER_HEALTH: Name = Name("com.mitchellh.ghostty.didUpdateRendererHealth");

/// Notifications related to key sequences
pub const DID_CONTINUE_KEY_SEQUENCE: Name = Name("com.mitchellh.ghostty.didContinueKeySequence");
pub const DID_END_KEY_SEQUENCE: Name = Name("com.mitchellh.ghostty.didEndKeySequence");
/// `KeySequenceKey`.
pub const KEY_SEQUENCE_KEY: &str = "com.mitchellh.ghostty.didContinueKeySequence.key";

/// Notifications related to key tables
pub const DID_CHANGE_KEY_TABLE: Name = Name("com.mitchellh.ghostty.didChangeKeyTable");
/// `KeyTableKey`.
pub const KEY_TABLE_KEY: &str = "com.mitchellh.ghostty.didChangeKeyTable.action";

/// `Notification.Name.terminalWindowBellDidChangeNotification` (declared in
/// `BaseTerminalController.swift`; MA3 posts it) and its `userInfo` key.
pub const TERMINAL_WINDOW_BELL_DID_CHANGE_NOTIFICATION: Name =
    Name("com.mitchellh.ghostty.terminalWindowBellDidChange");
pub const TERMINAL_WINDOW_HAS_BELL_KEY: &str =
    "com.mitchellh.ghostty.terminalWindowBellDidChange.hasBell";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_swift() {
        assert_eq!(
            GHOSTTY_CONFIG_DID_CHANGE.raw_value(),
            "com.mitchellh.ghostty.configDidChange"
        );
        assert_eq!(
            GHOSTTY_CONFIG_CHANGE_KEY,
            "com.mitchellh.ghostty.configDidChange"
        );
        assert_eq!(
            READONLY_KEY,
            "com.mitchellh.ghostty.didChangeReadonly.readonly"
        );
        assert_eq!(
            KEY_SEQUENCE_KEY,
            "com.mitchellh.ghostty.didContinueKeySequence.key"
        );
    }
}
