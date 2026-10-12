//! Port of `action.zig`: the actions an apprt has to react to.
//!
//! C-ABI plumbing (`C`, `CValue`, `cval`, `ghostty.h` sync tests) is not
//! ported; the enums keep their C integer discriminants via `as i32`.

use std::sync::Arc;

use ghostty_config::{config::Config, types_b::Duration};
use ghostty_input::binding::Trigger;
use ghostty_vt::{mouse_shape::MouseShape, osc::ProgressReport};

use crate::surface::{ChildExited, RendererHealth, Scrollbar};

/// The target for an action. This is generally the thing that had focus
/// while the action was made but the concept of "focus" is not guaranteed
/// since actions can also be triggered by timers, scripts, etc.
///
/// Generic over the surface handle `S` (Zig: `*CoreSurface`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target<S> {
    App,
    Surface(S),
}

/// Mirrors `Target.Key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TargetKey {
    App,
    Surface,
}

impl<S> Target<S> {
    #[must_use]
    pub fn key(&self) -> TargetKey {
        match self {
            Target::App => TargetKey::App,
            Target::Surface(_) => TargetKey::Surface,
        }
    }
}

/// The possible actions an apprt has to react to. Actions are one-way
/// messages that are sent to the app runtime to trigger some behavior.
///
/// Actions are very often key binding actions but can also be triggered
/// by lifecycle events. For example, the `quit_timer` action is not bindable.
///
/// Importantly, actions are generally OPTIONAL to implement by an apprt.
/// Required functionality is called directly on the runtime structure so
/// there is a compiler error if an action is not implemented.
#[derive(Debug, Clone)]
pub enum Action {
    /// Quit the application.
    Quit,

    /// Open a new window. The target determines whether properties such
    /// as font size should be inherited.
    NewWindow,

    /// Open a new tab. If the target is a surface it should be opened in
    /// the same window as the surface. If the target is the app then
    /// the tab should be opened in a new window.
    NewTab,

    /// Closes the tab belonging to the currently focused split, or all other
    /// tabs, depending on the mode.
    CloseTab(CloseTabMode),

    /// Create a new split. The value determines the location of the split
    /// relative to the target.
    NewSplit(SplitDirection),

    /// Close all open windows.
    CloseAllWindows,

    /// Toggle maximized window state.
    ToggleMaximize,

    /// Toggle fullscreen mode.
    ToggleFullscreen(Fullscreen),

    /// Toggle tab overview.
    ToggleTabOverview,

    /// Toggle whether window directions are shown.
    ToggleWindowDecorations,

    /// Toggle the quick terminal in or out.
    ToggleQuickTerminal,

    /// Toggle the command palette.
    ToggleCommandPalette,

    /// Toggle the visibility of all Ghostty terminal windows.
    ToggleVisibility,

    /// Toggle the window background opacity. This only has an effect
    /// if the window started as transparent (non-opaque), and toggles
    /// it between fully opaque and the configured background opacity.
    ToggleBackgroundOpacity,

    /// Moves a tab by a relative offset.
    ///
    /// Adjusts the tab position based on `offset` (e.g., -1 for left, +1
    /// for right). If the new position is out of bounds, it wraps around
    /// cyclically within the tab range.
    MoveTab(MoveTab),

    /// Jump to a specific tab. Must handle the scenario that the tab
    /// value is invalid.
    GotoTab(GotoTab),

    /// Jump to a specific split.
    GotoSplit(GotoSplit),

    /// Jump to next/previous window.
    GotoWindow(GotoWindow),

    /// Resize the split in the given direction.
    ResizeSplit(ResizeSplit),

    /// Equalize all the splits in the target window.
    EqualizeSplits,

    /// Toggle whether a split is zoomed or not. A zoomed split is resized
    /// to take up the entire window.
    ToggleSplitZoom,

    /// Present the target terminal whether its a tab, split, or window.
    PresentTerminal,

    /// Sets a size limit (in pixels) for the target terminal.
    SizeLimit(SizeLimit),

    /// Resets the window size to the default size. See the
    /// `reset_window_size` keybinding for more information.
    ResetWindowSize,

    /// Specifies the initial size of the target terminal.
    ///
    /// This may be sent once during the initialization of a surface
    /// (as part of the init call) to indicate the initial size requested
    /// for the window if it is not maximized or fullscreen.
    ///
    /// This may also be sent at any time after the surface is initialized
    /// to note the new "default size" of the window. This should in general
    /// be ignored, but may be useful if the apprt wants to support
    /// a "return to default size" action.
    InitialSize(InitialSize),

    /// The cell size has changed to the given dimensions in pixels.
    CellSize(CellSize),

    /// The scrollbar is updating.
    Scrollbar(Scrollbar),

    /// The target should be re-rendered. This usually has a specific
    /// surface target but if the app is targeted then all active
    /// surfaces should be redrawn.
    Render,

    /// Control whether the inspector is shown or hidden.
    Inspector(Inspector),

    /// Show the GTK inspector.
    ShowGtkInspector,

    /// The inspector for the given target has changes and should be
    /// rendered at the next opportunity.
    RenderInspector,

    /// Show a desktop notification.
    DesktopNotification(DesktopNotification),

    /// Set the title of the target to the requested value.
    SetTitle(SetTitle),

    /// Set the tab title override for the target's tab.
    SetTabTitle(SetTitle),

    /// Set the title of the target to a prompted value. It is up to
    /// the apprt to prompt. The value specifies whether to prompt for the
    /// surface title or the tab title.
    PromptTitle(PromptTitle),

    /// The current working directory has changed for the target terminal.
    Pwd(Pwd),

    /// Set the mouse cursor shape.
    MouseShape(MouseShape),

    /// Set whether the mouse cursor is visible or not.
    MouseVisibility(MouseVisibility),

    /// Called when the mouse is over or recently left a link.
    MouseOverLink(MouseOverLink),

    /// The health of the renderer has changed.
    RendererHealth(RendererHealth),

    /// Open the Ghostty configuration. This is platform-specific about
    /// what it means; it can mean opening a dedicated UI or just opening
    /// a file in a text editor.
    OpenConfig,

    /// Called when there are no more surfaces and the app should quit
    /// after the configured delay.
    ///
    /// Despite the name, this is the notification that libghostty sends
    /// when there are no more surfaces regardless of if the configuration
    /// wants to quit after close, has any delay set, etc. It's up to the
    /// apprt to implement the proper logic based on the config.
    ///
    /// This can be cancelled by sending another quit_timer action with "stop".
    /// Multiple "starts" shouldn't happen and can be ignored or cause a
    /// restart it isn't that important.
    QuitTimer(QuitTimer),

    /// Set the window floating state. A floating window is one that is
    /// always on top of other windows even when not focused.
    FloatWindow(FloatWindow),

    /// Set the secure input functionality on or off. "Secure input" means
    /// that the user is currently at some sort of prompt where they may be
    /// entering a password or other sensitive information. This can be used
    /// by the app runtime to change the appearance of the cursor, setup
    /// system APIs to not log the input, etc.
    SecureInput(SecureInput),

    /// A sequenced key binding has started, continued, or stopped.
    /// The UI should show some indication that the user is in a sequenced
    /// key mode because other input may be ignored.
    KeySequence(KeySequence),

    /// A key table has been activated or deactivated.
    KeyTable(KeyTable),

    /// A terminal color was changed programmatically through things
    /// such as OSC 10/11.
    ColorChange(ColorChange),

    /// A request to reload the configuration. The reload request can be
    /// from a user or for some internal reason. The reload request may
    /// request it is a soft reload or a full reload. See the struct for
    /// more documentation.
    ///
    /// The configuration should be passed to updateConfig either at the
    /// app or surface level depending on the target.
    ReloadConfig(ReloadConfig),

    /// The configuration has changed. The value is the new configuration
    /// (shared via `Arc`; in Zig a pointer valid only for the duration of
    /// the action).
    ///
    /// This should be used by apprts to update any internal state that
    /// depends on configuration for the given target (i.e. headerbar colors).
    ///
    /// This allows an apprt to have config-dependent state reactively
    /// change without having to store the entire configuration or poll
    /// for changes.
    ConfigChange(ConfigChange),

    /// Closes the currently focused window.
    CloseWindow,

    /// Called when the bell character is seen. The apprt should do whatever
    /// it needs to ring the bell. This is usually a sound or visual effect.
    RingBell,

    /// Undo the last action. See the "undo" keybinding for more
    /// details on what can and cannot be undone.
    Undo,

    /// Redo the last undone action.
    Redo,

    CheckForUpdates,

    /// Open a URL using the native OS mechanisms. On macOS this might be `open`
    /// or on Linux this might be `xdg-open`. The exact mechanism is up to the
    /// apprt.
    OpenUrl(OpenUrl),

    /// Show a native GUI notification that the child process has exited.
    ShowChildExited(ChildExited),

    /// Show a native GUI notification about the progress of some TUI operation.
    ProgressReport(ProgressReport),

    /// Show the on-screen keyboard.
    ShowOnScreenKeyboard,

    /// A command has finished,
    CommandFinished(CommandFinished),

    /// Start the search overlay with an optional initial needle. If the
    /// search is already active and the needle is non-empty, update the
    /// current search needle and focus the search input.
    StartSearch(StartSearch),

    /// End the search overlay, clearing the search state and hiding it.
    EndSearch,

    /// The total number of matches found by the search.
    SearchTotal(SearchTotal),

    /// The currently selected search match index (1-based).
    SearchSelected(SearchSelected),

    /// The readonly state of the surface has changed.
    Readonly(Readonly),

    /// Copy the effective title of the surface to the clipboard.
    /// The effective title is the user-overridden title if set,
    /// otherwise the terminal-set title.
    CopyTitleToClipboard,
}

/// Mirrors `Action.Key`. The order matches the libghostty C enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Key {
    Quit,
    NewWindow,
    NewTab,
    CloseTab,
    NewSplit,
    CloseAllWindows,
    ToggleMaximize,
    ToggleFullscreen,
    ToggleTabOverview,
    ToggleWindowDecorations,
    ToggleQuickTerminal,
    ToggleCommandPalette,
    ToggleVisibility,
    ToggleBackgroundOpacity,
    MoveTab,
    GotoTab,
    GotoSplit,
    GotoWindow,
    ResizeSplit,
    EqualizeSplits,
    ToggleSplitZoom,
    PresentTerminal,
    SizeLimit,
    ResetWindowSize,
    InitialSize,
    CellSize,
    Scrollbar,
    Render,
    Inspector,
    ShowGtkInspector,
    RenderInspector,
    DesktopNotification,
    SetTitle,
    SetTabTitle,
    PromptTitle,
    Pwd,
    MouseShape,
    MouseVisibility,
    MouseOverLink,
    RendererHealth,
    OpenConfig,
    QuitTimer,
    FloatWindow,
    SecureInput,
    KeySequence,
    KeyTable,
    ColorChange,
    ReloadConfig,
    ConfigChange,
    CloseWindow,
    RingBell,
    Undo,
    Redo,
    CheckForUpdates,
    OpenUrl,
    ShowChildExited,
    ProgressReport,
    ShowOnScreenKeyboard,
    CommandFinished,
    StartSearch,
    EndSearch,
    SearchTotal,
    SearchSelected,
    Readonly,
    CopyTitleToClipboard,
}

impl Action {
    /// The tag of this action.
    #[must_use]
    pub fn key(&self) -> Key {
        match self {
            Action::Quit => Key::Quit,
            Action::NewWindow => Key::NewWindow,
            Action::NewTab => Key::NewTab,
            Action::CloseTab(_) => Key::CloseTab,
            Action::NewSplit(_) => Key::NewSplit,
            Action::CloseAllWindows => Key::CloseAllWindows,
            Action::ToggleMaximize => Key::ToggleMaximize,
            Action::ToggleFullscreen(_) => Key::ToggleFullscreen,
            Action::ToggleTabOverview => Key::ToggleTabOverview,
            Action::ToggleWindowDecorations => Key::ToggleWindowDecorations,
            Action::ToggleQuickTerminal => Key::ToggleQuickTerminal,
            Action::ToggleCommandPalette => Key::ToggleCommandPalette,
            Action::ToggleVisibility => Key::ToggleVisibility,
            Action::ToggleBackgroundOpacity => Key::ToggleBackgroundOpacity,
            Action::MoveTab(_) => Key::MoveTab,
            Action::GotoTab(_) => Key::GotoTab,
            Action::GotoSplit(_) => Key::GotoSplit,
            Action::GotoWindow(_) => Key::GotoWindow,
            Action::ResizeSplit(_) => Key::ResizeSplit,
            Action::EqualizeSplits => Key::EqualizeSplits,
            Action::ToggleSplitZoom => Key::ToggleSplitZoom,
            Action::PresentTerminal => Key::PresentTerminal,
            Action::SizeLimit(_) => Key::SizeLimit,
            Action::ResetWindowSize => Key::ResetWindowSize,
            Action::InitialSize(_) => Key::InitialSize,
            Action::CellSize(_) => Key::CellSize,
            Action::Scrollbar(_) => Key::Scrollbar,
            Action::Render => Key::Render,
            Action::Inspector(_) => Key::Inspector,
            Action::ShowGtkInspector => Key::ShowGtkInspector,
            Action::RenderInspector => Key::RenderInspector,
            Action::DesktopNotification(_) => Key::DesktopNotification,
            Action::SetTitle(_) => Key::SetTitle,
            Action::SetTabTitle(_) => Key::SetTabTitle,
            Action::PromptTitle(_) => Key::PromptTitle,
            Action::Pwd(_) => Key::Pwd,
            Action::MouseShape(_) => Key::MouseShape,
            Action::MouseVisibility(_) => Key::MouseVisibility,
            Action::MouseOverLink(_) => Key::MouseOverLink,
            Action::RendererHealth(_) => Key::RendererHealth,
            Action::OpenConfig => Key::OpenConfig,
            Action::QuitTimer(_) => Key::QuitTimer,
            Action::FloatWindow(_) => Key::FloatWindow,
            Action::SecureInput(_) => Key::SecureInput,
            Action::KeySequence(_) => Key::KeySequence,
            Action::KeyTable(_) => Key::KeyTable,
            Action::ColorChange(_) => Key::ColorChange,
            Action::ReloadConfig(_) => Key::ReloadConfig,
            Action::ConfigChange(_) => Key::ConfigChange,
            Action::CloseWindow => Key::CloseWindow,
            Action::RingBell => Key::RingBell,
            Action::Undo => Key::Undo,
            Action::Redo => Key::Redo,
            Action::CheckForUpdates => Key::CheckForUpdates,
            Action::OpenUrl(_) => Key::OpenUrl,
            Action::ShowChildExited(_) => Key::ShowChildExited,
            Action::ProgressReport(_) => Key::ProgressReport,
            Action::ShowOnScreenKeyboard => Key::ShowOnScreenKeyboard,
            Action::CommandFinished(_) => Key::CommandFinished,
            Action::StartSearch(_) => Key::StartSearch,
            Action::EndSearch => Key::EndSearch,
            Action::SearchTotal(_) => Key::SearchTotal,
            Action::SearchSelected(_) => Key::SearchSelected,
            Action::Readonly(_) => Key::Readonly,
            Action::CopyTitleToClipboard => Key::CopyTitleToClipboard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SplitDirection {
    Right,
    Down,
    Left,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GotoSplit {
    Previous,
    Next,
    Up,
    Left,
    Down,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GotoWindow {
    Previous,
    Next,
}

/// The amount to resize the split by and the direction to resize it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeSplit {
    pub amount: u16,
    pub direction: ResizeSplitDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ResizeSplitDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveTab {
    pub amount: isize,
}

/// The tab to jump to. Non-negative integer values represent the index
/// (zero-based) of the tab to jump to. Negative values are special values
/// (Zig: non-exhaustive enum; unknown negatives are `Index(n)` with n < -3
/// and must be treated as invalid by the apprt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoTab {
    Previous,
    Next,
    Last,
    Index(i32),
}

impl GotoTab {
    #[must_use]
    pub fn from_int(v: i32) -> Self {
        match v {
            -1 => Self::Previous,
            -2 => Self::Next,
            -3 => Self::Last,
            n => Self::Index(n),
        }
    }

    #[must_use]
    pub fn int(self) -> i32 {
        match self {
            Self::Previous => -1,
            Self::Next => -2,
            Self::Last => -3,
            Self::Index(n) => n,
        }
    }
}

/// The fullscreen mode to toggle to if we're moving to fullscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Fullscreen {
    Native,

    /// macOS has a non-native fullscreen mode that is more like a maximized
    /// window. This is much faster to enter and exit than the native mode.
    MacosNonNative,
    MacosNonNativeVisibleMenu,
    MacosNonNativePaddedNotch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FloatWindow {
    On,
    Off,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SecureInput {
    On,
    Off,
    Toggle,
}

/// The inspector mode to toggle to if we're toggling the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Inspector {
    Toggle,
    Show,
    Hide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum QuitTimer {
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Readonly {
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MouseVisibility {
    Visible,
    Hidden,
}

/// Whether to prompt for the surface title or tab title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PromptTitle {
    Surface,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseOverLink {
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeLimit {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTitle {
    pub title: String,
}

impl std::fmt::Display for SetTitle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SetTitle{{ {} }}", self.title)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pwd {
    /// The working directory as reported by OSC 7. Zig keeps raw bytes; here
    /// they are carried as an [`EncodedOs`](ghostty_os::osbytes::EncodedOs)
    /// that must be decoded (`to_path`/`to_bytes`) at the OS/C boundary.
    pub pwd: ghostty_os::osbytes::EncodedOs,
}

impl std::fmt::Display for Pwd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pwd{{ {} }}", self.pwd.as_encoded())
    }
}

/// The desktop notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotification {
    pub title: String,
    pub body: String,
}

impl std::fmt::Display for DesktopNotification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DesktopNotification{{ title: {}, body: {} }}",
            self.title, self.body
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySequence {
    Trigger(Trigger),
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyTable {
    Activate(String),
    Deactivate,
    DeactivateAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorChange {
    pub kind: ColorKind,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Negative numbers indicate some named kind; 0+ values indicate a palette
/// index (Zig: non-exhaustive enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorKind {
    Foreground,
    Background,
    Cursor,
    Palette(i32),
}

impl ColorKind {
    #[must_use]
    pub fn from_int(v: i32) -> Self {
        match v {
            -1 => Self::Foreground,
            -2 => Self::Background,
            -3 => Self::Cursor,
            n => Self::Palette(n),
        }
    }

    #[must_use]
    pub fn int(self) -> i32 {
        match self {
            Self::Foreground => -1,
            Self::Background => -2,
            Self::Cursor => -3,
            Self::Palette(n) => n,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReloadConfig {
    /// A soft reload means that the configuration doesn't need to be
    /// read off disk, but libghostty needs the full config again so call
    /// updateConfig with it.
    pub soft: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigChange {
    pub config: Arc<Config>,
}

/// Open a URL
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenUrl {
    /// The type of data that the URL refers to.
    pub kind: OpenUrlKind,

    /// The URL or path to open, as raw bytes (Zig `[]const u8`). Plain text
    /// URLs are wrapped with `EncodedOs::from_plain_text`, file names with
    /// `EncodedOs::from_path`/`from_bytes`; the opener decodes it.
    pub url: ghostty_os::osbytes::EncodedOs,
}

/// The type of the data at the URL to open. This is used as a hint to
/// potentially open the URL in a different way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum OpenUrlKind {
    /// The type is unknown. This is the default and apprts should
    /// open the URL in the most generic way possible. For example,
    /// on macOS this would be the equivalent of `open` or on Linux
    /// this would be `xdg-open`.
    #[default]
    Unknown,

    /// The URL is known to be a text file. In this case, the apprt
    /// should try to open the URL in a text editor or viewer or
    /// some equivalent, if possible.
    Text,

    /// The URL is known to contain HTML content.
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CloseTabMode {
    /// Close the current tab.
    This,
    /// Close all other tabs.
    Other,
    /// Close all tabs to the right of the current tab.
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandFinished {
    pub exit_code: Option<u8>,
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSearch {
    pub needle: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchTotal {
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchSelected {
    pub selected: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_key() {
        assert_eq!(Target::<u32>::App.key(), TargetKey::App);
        assert_eq!(Target::Surface(1u32).key(), TargetKey::Surface);
        assert_eq!(TargetKey::Surface as i32, 1);
    }

    #[test]
    fn action_key_order_matches_c_enum() {
        assert_eq!(Key::Quit as i32, 0);
        assert_eq!(Key::CopyTitleToClipboard as i32, 64);
        assert_eq!(Action::Quit.key(), Key::Quit);
        assert_eq!(Action::EndSearch.key(), Key::EndSearch);
        assert_eq!(
            Action::ResizeSplit(ResizeSplit {
                amount: 1,
                direction: ResizeSplitDirection::Up
            })
            .key(),
            Key::ResizeSplit
        );
        assert_eq!(
            Action::QuitTimer(QuitTimer::Start).key() as i32,
            Key::QuitTimer as i32
        );
    }

    #[test]
    fn goto_tab_roundtrip() {
        assert_eq!(GotoTab::from_int(-1), GotoTab::Previous);
        assert_eq!(GotoTab::from_int(-2), GotoTab::Next);
        assert_eq!(GotoTab::from_int(-3), GotoTab::Last);
        assert_eq!(GotoTab::from_int(4), GotoTab::Index(4));
        assert_eq!(GotoTab::Index(4).int(), 4);
        assert_eq!(GotoTab::Last.int(), -3);
    }

    #[test]
    fn color_kind_roundtrip() {
        assert_eq!(ColorKind::from_int(-2), ColorKind::Background);
        assert_eq!(ColorKind::from_int(7), ColorKind::Palette(7));
        assert_eq!(ColorKind::Cursor.int(), -3);
    }

    #[test]
    fn enum_discriminants() {
        assert_eq!(SplitDirection::Up as i32, 3);
        assert_eq!(GotoSplit::Right as i32, 5);
        assert_eq!(Fullscreen::MacosNonNativePaddedNotch as i32, 3);
        assert_eq!(OpenUrlKind::default(), OpenUrlKind::Unknown);
        assert_eq!(CloseTabMode::Right as i32, 2);
        assert_eq!(Readonly::On as i32, 1);
    }

    #[test]
    fn display_formats() {
        assert_eq!(SetTitle { title: "x".into() }.to_string(), "SetTitle{ x }");
        assert_eq!(
            Pwd {
                pwd: ghostty_os::osbytes::EncodedOs::from_encoded("/a")
            }
            .to_string(),
            "Pwd{ /a }"
        );
        assert_eq!(
            DesktopNotification {
                title: "t".into(),
                body: "b".into()
            }
            .to_string(),
            "DesktopNotification{ title: t, body: b }"
        );
    }

    #[test]
    fn reload_config_default_hard() {
        assert!(!ReloadConfig::default().soft);
    }
}
