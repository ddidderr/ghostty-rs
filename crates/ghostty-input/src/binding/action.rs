//! The set of actions that a keybinding can take (`Binding.Action` in
//! ZIG/src/input/Binding.zig).

#![allow(clippy::needless_raw_string_hashes)]

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use ghostty_os::osbytes::EncodedOs;

use super::Error;

/// Define a fieldless enum whose variants are named in snake_case in the
/// Zig source (and therefore in configuration / `+list-actions`).
macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$vmeta:meta])* $variant:ident => $s:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// The Zig tag name.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $s,)*
                }
            }

            /// Look up by Zig tag name.
            pub fn from_name(s: &str) -> Option<$name> {
                match s {
                    $($s => Some($name::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

/// Implements `parse_param` for plain string enums.
macro_rules! plain_param {
    ($name:ident) => {
        impl $name {
            pub(super) fn parse_param(s: &str) -> Result<$name, Error> {
                $name::from_name(s).ok_or(Error::InvalidFormat)
            }
        }
    };
}

/// The scope of an action. The scope is the context in which an action
/// must be executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    App,
    Surface,
}

str_enum! {
    pub enum CrashThread {
        Main => "main",
        Io => "io",
        Render => "render",
    }
}
plain_param!(CrashThread);

/// Data to send depending on whether cursor key mode is enabled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CursorKey {
    pub normal: String,
    pub application: String,
}

str_enum! {
    pub enum NavigateSearch {
        Previous => "previous",
        Next => "next",
    }
}
plain_param!(NavigateSearch);

str_enum! {
    pub enum AdjustSelection {
        Left => "left",
        Right => "right",
        Up => "up",
        Down => "down",
        PageUp => "page_up",
        PageDown => "page_down",
        Home => "home",
        End => "end",
        BeginningOfLine => "beginning_of_line",
        EndOfLine => "end_of_line",
    }
}
plain_param!(AdjustSelection);

str_enum! {
    pub enum SplitDirection {
        Right => "right",
        Down => "down",
        Left => "left",
        Up => "up",
        /// splits along the larger direction
        Auto => "auto",
    }
}
plain_param!(SplitDirection);

impl SplitDirection {
    pub const DEFAULT: SplitDirection = SplitDirection::Auto;
}

str_enum! {
    pub enum SplitFocusDirection {
        Previous => "previous",
        Next => "next",
        Up => "up",
        Left => "left",
        Down => "down",
        Right => "right",
    }
}

impl SplitFocusDirection {
    pub fn parse(input: &str) -> Result<SplitFocusDirection, Error> {
        if let Some(v) = SplitFocusDirection::from_name(input) {
            return Ok(v);
        }
        // For backwards compatibility we map "top" and "bottom" onto the enum
        // values "up" and "down"
        match input {
            "top" => Ok(SplitFocusDirection::Up),
            "bottom" => Ok(SplitFocusDirection::Down),
            _ => Err(Error::InvalidFormat),
        }
    }

    pub(super) fn parse_param(s: &str) -> Result<SplitFocusDirection, Error> {
        SplitFocusDirection::parse(s)
    }
}

str_enum! {
    pub enum SplitResizeDirection {
        Up => "up",
        Down => "down",
        Left => "left",
        Right => "right",
    }
}
plain_param!(SplitResizeDirection);

str_enum! {
    pub enum GotoWindow {
        Previous => "previous",
        Next => "next",
    }
}
plain_param!(GotoWindow);

/// Zig: a tuple `struct { SplitResizeDirection, u16 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SplitResizeParameter(pub SplitResizeDirection, pub u16);

impl SplitResizeParameter {
    pub(super) fn parse_param(param: &str) -> Result<SplitResizeParameter, Error> {
        let mut it = param.split(',');
        let dir = SplitResizeDirection::parse_param(it.next().ok_or(Error::InvalidFormat)?)?;
        let amt = it.next().ok_or(Error::InvalidFormat)?;
        let amt = ghostty_os::zig_fmt::parse_int::<u16>(amt.as_bytes(), 10)
            .map_err(|_| Error::InvalidFormat)?;
        // If we have extra parameters it is an error
        if it.next().is_some() {
            return Err(Error::InvalidFormat);
        }
        Ok(SplitResizeParameter(dir, amt))
    }

    pub(super) fn format_into(self, out: &mut String) {
        out.push_str(self.0.name());
        out.push(',');
        out.push_str(&self.1.to_string());
    }
}

str_enum! {
    pub enum CopyToClipboard {
        Plain => "plain",
        Vt => "vt",
        Html => "html",
        /// This type will mix multiple distinct types with a set content-type
        /// such as text/html for html, so that the OS/application can choose
        /// what is best when pasting.
        Mixed => "mixed",
    }
}
plain_param!(CopyToClipboard);

impl CopyToClipboard {
    pub const DEFAULT: CopyToClipboard = CopyToClipboard::Mixed;
}

str_enum! {
    pub enum WriteScreenAction {
        Copy => "copy",
        Paste => "paste",
        Open => "open",
    }
}

str_enum! {
    pub enum WriteScreenFormat {
        Plain => "plain",
        Vt => "vt",
        Html => "html",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WriteScreen {
    pub action: WriteScreenAction,
    pub emit: WriteScreenFormat,
}

impl WriteScreen {
    pub const COPY: WriteScreen = WriteScreen {
        action: WriteScreenAction::Copy,
        emit: WriteScreenFormat::Plain,
    };
    pub const PASTE: WriteScreen = WriteScreen {
        action: WriteScreenAction::Paste,
        emit: WriteScreenFormat::Plain,
    };
    pub const OPEN: WriteScreen = WriteScreen {
        action: WriteScreenAction::Open,
        emit: WriteScreenFormat::Plain,
    };

    pub fn parse(param: &str) -> Result<WriteScreen, Error> {
        let pe = |s: &str| WriteScreenAction::from_name(s).ok_or(Error::InvalidFormat);
        // If we don't have a `,`, default to the plain format. This is
        // also very important for backwards compatibility before Ghostty
        // 1.3 which didn't support output formats.
        let Some(idx) = param.find(',') else {
            return Ok(WriteScreen {
                action: pe(param)?,
                emit: WriteScreenFormat::Plain,
            });
        };
        Ok(WriteScreen {
            action: pe(&param[..idx])?,
            emit: WriteScreenFormat::from_name(&param[idx + 1..]).ok_or(Error::InvalidFormat)?,
        })
    }

    pub(super) fn parse_param(s: &str) -> Result<WriteScreen, Error> {
        WriteScreen::parse(s)
    }

    pub(super) fn format_into(self, out: &mut String) {
        out.push_str(self.action.name());
        out.push(',');
        out.push_str(self.emit.name());
    }
}

str_enum! {
    pub enum InspectorMode {
        Toggle => "toggle",
        Show => "show",
        Hide => "hide",
    }
}
plain_param!(InspectorMode);

str_enum! {
    pub enum CloseTabMode {
        This => "this",
        Other => "other",
        Right => "right",
    }
}
plain_param!(CloseTabMode);

impl CloseTabMode {
    pub const DEFAULT: CloseTabMode = CloseTabMode::This;
}

/// Integer / float parameter parsing (`parseInt`/`parseFloat`).
pub(super) trait ParseNum: Sized {
    fn parse_param(s: &str) -> Result<Self, Error>;
}

macro_rules! int_param {
    ($($t:ty),*) => {$(
        impl ParseNum for $t {
            fn parse_param(s: &str) -> Result<$t, Error> {
                ghostty_os::zig_fmt::parse_int::<$t>(s.as_bytes(), 10)
                    .map_err(|_| Error::InvalidFormat)
            }
        }
    )*};
}
int_param!(usize, isize, i16);

impl ParseNum for f32 {
    fn parse_param(s: &str) -> Result<f32, Error> {
        ghostty_os::zig_fmt::parse_float::<f32>(s.as_bytes()).map_err(|_| Error::InvalidFormat)
    }
}

/// Escape in the style of Zig's `std.zig.stringEscape`.
pub(super) fn string_escape(s: &[u8], out: &mut String) {
    use std::fmt::Write;
    for &b in s {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\'' | b' ' | b'!' | b'#'..=b'&' | b'('..=b'[' | b']'..=b'~' => out.push(b as char),
            _ => {
                let _ = write!(out, "\\x{b:02x}");
            }
        }
    }
}

impl Eq for Action {}

impl Hash for Action {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_into(state);
    }
}

impl fmt::Display for Action {
    /// Encodes the action back into the format used by `parse`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        self.fmt_into(&mut s);
        f.write_str(&s)
    }
}

impl Action {
    /// Parse an action in the format of "key=value" where key is the
    /// action name and value is the action parameter. The parameter
    /// is optional depending on the action.
    pub fn parse(input: &str) -> Result<Action, Error> {
        Self::parse_bytes(input.as_bytes())
    }

    /// Parse raw action bytes. Only enum/numeric parameters require UTF-8;
    /// byte payloads are stored without interpreting the OS escape encoding.
    pub fn parse_bytes(input: &[u8]) -> Result<Action, Error> {
        let idx = input.iter().position(|b| *b == b':');
        let name = std::str::from_utf8(&input[..idx.unwrap_or(input.len())])
            .map_err(|_| Error::InvalidAction)?;
        if name.is_empty() {
            return Err(Error::InvalidFormat);
        }
        let param = idx.map(|i| &input[i + 1..]);
        let bytes = || param.map(EncodedOs::from_bytes).ok_or(Error::InvalidFormat);
        match name {
            "csi" => Ok(Self::Csi(bytes()?)),
            "esc" => Ok(Self::Esc(bytes()?)),
            "text" => Ok(Self::Text(bytes()?)),
            "search" => Ok(Self::Search(bytes()?)),
            "set_surface_title" => Ok(Self::SetSurfaceTitle(bytes()?)),
            "set_tab_title" => Ok(Self::SetTabTitle(bytes()?)),
            "activate_key_table" => Ok(Self::ActivateKeyTable(bytes()?)),
            "activate_key_table_once" => Ok(Self::ActivateKeyTableOnce(bytes()?)),
            // Zig never validates UTF-8 for enum/numeric parameters: the
            // bytes just fail the field's own grammar (`InvalidFormat`) for
            // a known action and `InvalidAction` for an unknown name. U+FFFD
            // is accepted by none of those grammars, so it stands in for any
            // invalid parameter.
            _ => Self::parse_named(
                name,
                param.map(|p| std::str::from_utf8(p).unwrap_or("\u{fffd}")),
            ),
        }
    }

    /// The action name (Zig `@tagName`).
    pub fn name(&self) -> &'static str {
        self.key().name()
    }

    /// The doc comment of this action.
    pub fn docs(&self) -> &'static str {
        self.key().docs()
    }

    /// A hash code that can be used to uniquely identify this action.
    pub fn hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash_into(&mut h);
        h.finish()
    }

    /// Compares two actions for equality.
    pub fn equal(&self, other: &Action) -> bool {
        self == other
    }

    /// Returns the action if it is of the given scope (Zig `scoped`).
    ///
    /// DEVIATION: Zig returns a distinct comptime-generated union type; here
    /// the same `Action` type is returned.
    pub fn scoped(&self, s: Scope) -> Option<&Action> {
        (self.scope() == s).then_some(self)
    }
}

// @generated-once from ZIG/src/input/Binding.zig `Action` (see gen script in notes); now maintained by hand.

/// The set of actions that a keybinding can take.
///
/// The per-action documentation (printed by `ghostty +list-actions --docs`)
/// is available through [`Action::docs`] / [`ActionKey::docs`].
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Ignore this key combination.
    ///
    /// Ghostty will not process this combination nor forward it to the child
    /// process within the terminal, but it may still be processed by the OS or
    /// other applications.
    Ignore,
    /// Unbind a previously bound key binding.
    ///
    /// This cannot unbind bindings that were not bound by Ghostty or the user
    /// (e.g. bindings set by the OS or some other application).
    Unbind,
    /// Send a CSI sequence.
    ///
    /// The value should be the CSI sequence without the CSI header (`ESC [` or
    /// `\x1b[`).
    ///
    /// For example, `csi:0m` can be sent to reset all styles of the current text.
    Csi(EncodedOs),
    /// Send an `ESC` sequence.
    Esc(EncodedOs),
    /// Send the specified text.
    ///
    /// Uses Zig string literal syntax. This is currently not validated.
    /// If the text is invalid (i.e. contains an invalid escape sequence),
    /// the error will currently only show up in logs.
    Text(EncodedOs),
    /// Send data to the pty depending on whether cursor key mode is enabled
    /// (`application`) or disabled (`normal`).
    CursorKey(CursorKey),
    /// Reset the terminal.
    ///
    /// This can fix a lot of issues when a running program puts the terminal
    /// into a broken state, equivalent to running the `reset` command.
    ///
    /// If you do this while in a TUI program such as vim, this may break
    /// the program. If you do this while in a shell, you may have to press
    /// enter after to get a new prompt.
    Reset,
    /// Copy the selected text to the clipboard.
    CopyToClipboard(CopyToClipboard),
    /// Paste the contents of the default clipboard.
    PasteFromClipboard,
    /// Paste the contents of the selection clipboard.
    PasteFromSelection,
    /// If there is a URL under the cursor, copy it to the default clipboard.
    CopyUrlToClipboard,
    /// Copy the terminal title to the clipboard. If the terminal title is not
    /// set or is empty this has no effect.
    CopyTitleToClipboard,
    /// Increase the font size by the specified amount in points (pt).
    ///
    /// For example, `increase_font_size:1.5` will increase the font size
    /// by 1.5 points.
    IncreaseFontSize(f32),
    /// Decrease the font size by the specified amount in points (pt).
    ///
    /// For example, `decrease_font_size:1.5` will decrease the font size
    /// by 1.5 points.
    DecreaseFontSize(f32),
    /// Reset the font size to the original configured size.
    ResetFontSize,
    /// Set the font size to the specified size in points (pt).
    ///
    /// For example, `set_font_size:14.5` will set the font size
    /// to 14.5 points.
    SetFontSize(f32),
    /// Start a search for the given text. If the text is empty, then
    /// the search is canceled. A canceled search will not disable any GUI
    /// elements showing search. For that, the explicit end_search binding
    /// should be used.
    ///
    /// If a previous search is active, it is replaced.
    Search(EncodedOs),
    /// Start a search for the current text selection. If there is no
    /// selection, this does nothing. If a search is already active, this
    /// changes the search terms.
    SearchSelection,
    /// Navigate the search results. If there is no active search, this
    /// is not performed.
    NavigateSearch(NavigateSearch),
    /// Start a search if it isn't started already. This doesn't set any
    /// search terms, but opens the UI for searching.
    StartSearch,
    /// End the current search if any and hide any GUI elements.
    EndSearch,
    /// Clear the screen and all scrollback.
    ClearScreen,
    /// Select all text on the screen.
    SelectAll,
    /// Scroll to the top of the screen.
    ScrollToTop,
    /// Scroll to the bottom of the screen.
    ScrollToBottom,
    /// Scroll to the selected text.
    ScrollToSelection,
    /// Scroll to the given absolute row in the screen with 0 being
    /// the first row.
    ScrollToRow(usize),
    /// Scroll the screen up by one page.
    ScrollPageUp,
    /// Scroll the screen down by one page.
    ScrollPageDown,
    /// Scroll the screen by the specified fraction of a page.
    ///
    /// Positive values scroll downwards, and negative values scroll upwards.
    ///
    /// For example, `scroll_page_fractional:0.5` would scroll the screen
    /// downwards by half a page, while `scroll_page_fractional:-1.5` would
    /// scroll it upwards by one and a half pages.
    ScrollPageFractional(f32),
    /// Scroll the screen by the specified amount of lines.
    ///
    /// Positive values scroll downwards, and negative values scroll upwards.
    ///
    /// For example, `scroll_page_lines:3` would scroll the screen downwards
    /// by 3 lines, while `scroll_page_lines:-10` would scroll it upwards by 10
    /// lines.
    ScrollPageLines(i16),
    /// Adjust the current selection in the given direction or position,
    /// relative to the cursor.
    ///
    /// WARNING: This does not create a new selection, and does nothing when
    /// there currently isn't one.
    ///
    /// Valid arguments are:
    ///
    ///   - `left`, `right`
    ///
    ///     Adjust the selection one cell to the left or right respectively.
    ///
    ///   - `up`, `down`
    ///
    ///     Adjust the selection one line upwards or downwards respectively.
    ///
    ///   - `page_up`, `page_down`
    ///
    ///     Adjust the selection one page upwards or downwards respectively.
    ///
    ///   - `home`, `end`
    ///
    ///     Adjust the selection to the top-left or the bottom-right corner
    ///     of the screen respectively.
    ///
    ///   - `beginning_of_line`, `end_of_line`
    ///
    ///     Adjust the selection to the beginning or the end of the line
    ///     respectively.
    ///
    AdjustSelection(AdjustSelection),
    /// Jump the viewport forward or back by the given number of prompts.
    ///
    /// Requires shell integration.
    ///
    /// Positive values scroll downwards, and negative values scroll upwards.
    JumpToPrompt(i16),
    /// Write the entire scrollback into a temporary file with the specified
    /// action. The action determines what to do with the filepath.
    ///
    /// Valid actions are:
    ///
    ///   - `copy`
    ///
    ///     Copy the file path into the clipboard.
    ///
    ///   - `paste`
    ///
    ///     Paste the file path into the terminal.
    ///
    ///   - `open`
    ///
    ///     Open the file in the default OS editor for text files.
    ///
    ///     The default OS editor is determined by using `open` on macOS
    ///     and `xdg-open` on Linux.
    ///
    WriteScrollbackFile(WriteScreen),
    /// Write the contents of the screen into a temporary file with the
    /// specified action.
    ///
    /// See `write_scrollback_file` for possible actions.
    WriteScreenFile(WriteScreen),
    /// Write the currently selected text into a temporary file with the
    /// specified action.
    ///
    /// See `write_scrollback_file` for possible actions.
    ///
    /// Does nothing when no text is selected.
    WriteSelectionFile(WriteScreen),
    /// Open a new window.
    ///
    /// If the application isn't currently focused,
    /// this will bring it to the front.
    NewWindow,
    /// Open a new tab.
    NewTab,
    /// Go to the previous tab.
    PreviousTab,
    /// Go to the next tab.
    NextTab,
    /// Go to the last tab.
    LastTab,
    /// Go to the tab with the specific index, starting from 1.
    ///
    /// If the tab number is higher than the number of tabs,
    /// this will go to the last tab.
    GotoTab(usize),
    /// Moves a tab by a relative offset.
    ///
    /// Positive values move the tab forwards, and negative values move it
    /// backwards. If the new position is out of bounds, it is wrapped around
    /// cyclically within the tab list.
    ///
    /// For example, `move_tab:1` moves the tab one position forwards, and if
    /// it was already the last tab in the list, it wraps around and becomes
    /// the first tab in the list. Likewise, `move_tab:-1` moves the tab one
    /// position backwards, and if it was the first tab, then it will become
    /// the last tab.
    MoveTab(isize),
    /// Toggle the tab overview.
    ///
    /// This is only supported on Linux and when the system's libadwaita
    /// version is 1.4 or newer. The current libadwaita version can be
    /// found by running `ghostty +version`.
    ToggleTabOverview,
    /// Change the title of the current focused surface via a pop-up prompt.
    PromptSurfaceTitle,
    /// Change the title of the current tab via a pop-up prompt. The
    /// title set via this prompt overrides any title set by the terminal
    /// and persists across focus changes within the tab.
    PromptTabTitle,
    /// Set the title for the current focused surface.
    ///
    /// If the title is empty, the surface title is reset to an empty title.
    SetSurfaceTitle(EncodedOs),
    /// Set the title for the current focused tab.
    ///
    /// If the title is empty, the tab title override is cleared.
    SetTabTitle(EncodedOs),
    /// Create a new split in the specified direction.
    ///
    /// Valid arguments:
    ///
    ///   - `right`, `down`, `left`, `up`
    ///
    ///     Creates a new split in the corresponding direction.
    ///
    ///   - `auto`
    ///
    ///     Creates a new split along the larger direction.
    ///     For example, if the parent split is currently wider than it is tall,
    ///     then a left-right split would be created, and vice versa.
    ///
    NewSplit(SplitDirection),
    /// Focus on a split either in the specified direction (`right`, `down`,
    /// `left` and `up`), or in the adjacent split in the order of creation
    /// (`previous` and `next`).
    GotoSplit(SplitFocusDirection),
    /// Focus on either the previous window or the next one ('previous', 'next')
    GotoWindow(GotoWindow),
    /// Zoom in or out of the current split.
    ///
    /// When a split is zoomed into, it will take up the entire space in
    /// the current tab, hiding other splits. The tab or tab bar would also
    /// reflect this by displaying an icon indicating the zoomed state.
    ToggleSplitZoom,
    /// Toggle read-only mode for the current surface.
    ///
    /// When a surface is in read-only mode:
    ///   - No input is sent to the PTY (mouse events, key encoding)
    ///   - Input can still be used at the terminal level to make selections,
    ///     copy/paste (keybinds), scroll, etc.
    ///   - Warn before quit is always enabled in this state even if an active
    ///     process is not running
    ToggleReadonly,
    /// Resize the current split in the specified direction and amount in
    /// pixels. The two arguments should be joined with a comma (`,`),
    /// like in `resize_split:up,10`.
    ResizeSplit(SplitResizeParameter),
    /// Equalize the size of all splits in the current window.
    EqualizeSplits,
    /// Reset the window to the default size. The "default size" is the
    /// size that a new window would be created with. This has no effect
    /// if the window is fullscreen.
    ///
    /// Only implemented on macOS.
    ResetWindowSize,
    /// Control the visibility of the terminal inspector.
    ///
    /// Valid arguments: `toggle`, `show`, `hide`.
    Inspector(InspectorMode),
    /// Show the GTK inspector.
    ///
    /// Has no effect on macOS.
    ShowGtkInspector,
    /// Show the on-screen keyboard if one is present.
    ///
    /// Only implemented on Linux (GTK). On GNOME, the "Screen Keyboard"
    /// accessibility feature must be turned on, which can be found under
    /// Settings > Accessibility > Typing. Other platforms are as of now
    /// untested.
    ShowOnScreenKeyboard,
    /// Open the configuration file in the default OS editor.
    ///
    /// If your default OS editor isn't configured then this will fail.
    /// Currently, any failures to open the configuration will show up only in
    /// the logs.
    OpenConfig,
    /// Reload the configuration.
    ///
    /// The exact meaning depends on the app runtime in use, but this usually
    /// involves re-reading the configuration file and applying any changes
    /// Note that not all changes can be applied at runtime.
    ReloadConfig,
    /// Close the current "surface", whether that is a window, tab, split, etc.
    ///
    /// This might trigger a close confirmation popup, depending on the value
    /// of the `confirm-close-surface` configuration setting.
    CloseSurface,
    /// Close the current tab and all splits therein, close all other tabs, or
    /// close every tab to the right of the current one depending on the mode.
    ///
    /// If the mode is not specified, defaults to closing the current tab.
    ///
    /// This might trigger a close confirmation popup, depending on the value
    /// of the `confirm-close-surface` configuration setting.
    CloseTab(CloseTabMode),
    /// Close the current window and all tabs and splits therein.
    ///
    /// This might trigger a close confirmation popup, depending on the value
    /// of the `confirm-close-surface` configuration setting.
    CloseWindow,
    /// Close all windows.
    ///
    /// WARNING: This action has been deprecated and has no effect on either
    /// Linux or macOS. Users are instead encouraged to use `all:close_window`
    /// instead.
    CloseAllWindows,
    /// Maximize or unmaximize the current window.
    ///
    /// This has no effect on macOS as it does not have the concept of
    /// maximized windows.
    ToggleMaximize,
    /// Fullscreen or unfullscreen the current window.
    ToggleFullscreen,
    /// Toggle window decorations (titlebar, buttons, etc.) for the current window.
    ///
    /// Only implemented on Linux.
    ToggleWindowDecorations,
    /// Toggle whether the terminal window should always float on top of other
    /// windows even when unfocused.
    ///
    /// Terminal windows always start as normal (not float-on-top) windows.
    ///
    /// Only implemented on macOS.
    ToggleWindowFloatOnTop,
    /// Toggle secure input mode.
    ///
    /// This is used to prevent apps from monitoring your keyboard input
    /// when entering passwords or other sensitive information.
    ///
    /// This applies to the entire application, not just the focused terminal.
    /// You must manually untoggle it or quit Ghostty entirely to disable it.
    ///
    /// Only implemented on macOS, as this uses a built-in system API.
    ToggleSecureInput,
    /// Toggle mouse reporting on or off.
    ///
    /// When mouse reporting is disabled, mouse events will not be reported to
    /// terminal applications even if they request it. This allows you to always
    /// use the mouse for selection and other terminal UI interactions without
    /// applications capturing mouse input.
    ///
    /// This can also be controlled via the `mouse-reporting` configuration
    /// option.
    ToggleMouseReporting,
    /// Toggle the command palette.
    ///
    /// The command palette is a popup that lets you see what actions
    /// you can perform, their associated keybindings (if any), a search bar
    /// to filter the actions, and the ability to then execute the action.
    ///
    /// This requires libadwaita 1.5 or newer on Linux. The current libadwaita
    /// version can be found by running `ghostty +version`.
    ToggleCommandPalette,
    /// Toggle the quick terminal.
    ///
    /// The quick terminal, also known as the "Quake-style" or drop-down
    /// terminal, is a terminal window that appears on demand from a keybinding,
    /// often sliding in from a screen edge such as the top. This is useful for
    /// quick access to a terminal without having to open a new window or tab.
    ///
    /// The terminal state is preserved between appearances, so showing the
    /// quick terminal after it was already hidden would display the same
    /// window instead of creating a new one.
    ///
    /// As quick terminals are often useful when other windows are currently
    /// focused, they are best used with *global* keybinds. For example, one
    /// can define the following key bind to toggle the quick terminal from
    /// anywhere within the system by pressing `` Cmd+` ``:
    ///
    /// ```ini
    /// keybind = global:cmd+backquote=toggle_quick_terminal
    /// ```
    ///
    /// The quick terminal has some limitations:
    ///
    ///   - Only one quick terminal instance can exist at a time.
    ///
    ///   - Unlike normal terminal windows, the quick terminal will not be
    ///     restored when the application is restarted on systems that support
    ///     window restoration like macOS.
    ///
    ///   - On Linux, the quick terminal is only supported on Wayland and not
    ///     X11, and only on Wayland compositors that support the `wlr-layer-shell-v1`
    ///     protocol. In practice, this means that only GNOME users would not be
    ///     able to use this feature.
    ///
    ///   - On Linux, slide-in animations are only supported on KDE, and when
    ///     the "Sliding Popups" KWin plugin is enabled.
    ///
    ///     If you do not have this plugin enabled, open System Settings > Apps
    ///     & Windows > Window Management > Desktop Effects, and enable the
    ///     plugin in the plugin list. Ghostty would then need to be restarted
    ///     fully for this to take effect.
    ///
    ///   - Quick terminal tabs are only supported on Linux and not on macOS.
    ///     This is because tabs on macOS require a title bar.
    ///
    ///   - On macOS, a fullscreened quick terminal will always be in non-native
    ///     fullscreen mode. This is a requirement due to how the quick terminal
    ///     is rendered.
    ///
    /// See the various configurations for the quick terminal in the
    /// configuration file to customize its behavior.
    ToggleQuickTerminal,
    /// Show or hide all windows. If all windows become shown, we also ensure
    /// Ghostty becomes focused. When hiding all windows, focus is yielded
    /// to the next application as determined by the OS.
    ///
    /// Note: When the focused surface is fullscreen, this method does nothing.
    ///
    /// Only implemented on macOS.
    ToggleVisibility,
    /// Toggle the window background opacity between transparent and opaque.
    ///
    /// This does nothing when `background-opacity` is set to 1 or above.
    ///
    /// When `background-opacity` is less than 1, this action will either make
    /// the window transparent or not depending on its current transparency state.
    ///
    /// Only implemented on macOS.
    ToggleBackgroundOpacity,
    /// Check for updates.
    ///
    /// Only implemented on macOS.
    CheckForUpdates,
    /// Undo the last undoable action for the focused surface or terminal,
    /// if possible. This can undo actions such as closing tabs or
    /// windows.
    ///
    /// Not every action in Ghostty can be undone or redone. The list
    /// of actions support undo/redo is currently limited to:
    ///
    ///   - New window, close window
    ///   - New tab, close tab
    ///   - New split, close split
    ///
    /// All actions are only undoable/redoable for a limited time.
    /// For example, restoring a closed split can only be done for
    /// some number of seconds since the split was closed. The exact
    /// amount is configured with the `undo-timeout` configuration settings.
    ///
    /// The undo/redo actions being limited ensures that there is
    /// bounded memory usage over time, closed surfaces don't continue running
    /// in the background indefinitely, and the keybinds become available
    /// for terminal applications to use.
    ///
    /// Only implemented on macOS.
    Undo,
    /// Redo the last undoable action for the focused surface or terminal,
    /// if possible. See "undo" for more details on what can and cannot
    /// be undone or redone.
    Redo,
    /// End the currently active key sequence, if any, and flush the
    /// keys up to this point to the terminal, excluding the key that
    /// triggered this action.
    ///
    /// For example: `ctrl+w>escape=end_key_sequence` would encode
    /// `ctrl+w` to the terminal and exit the key sequence.
    ///
    /// Normally, an invalid sequence will reset the key sequence and
    /// flush all data including the invalid key. This action allows
    /// you to flush only the prior keys, which is useful when you want
    /// to bind something like a control key (`ctrl+w`) but not send
    /// additional inputs.
    EndKeySequence,
    /// Activate a named key table (see `keybind` configuration documentation).
    /// The named key table will remain active until `deactivate_key_table`
    /// is called. If you want a one-shot key table activation, use the
    /// `activate_key_table_once` action instead.
    ///
    /// If the named key table does not exist, this action has no effect
    /// and performable will report false.
    ///
    /// If the named key table is already the currently active key table,
    /// this action has no effect and performable will report false.
    ActivateKeyTable(EncodedOs),
    /// Same as activate_key_table, but the key table will only be active
    /// until the first valid keybinding from that table is used (including
    /// any defined `catch_all` bindings).
    ///
    /// The "once" check is only done if this is the currently active
    /// key table. If another key table is activated later, then this
    /// table will remain active until it pops back out to being the
    /// active key table.
    ActivateKeyTableOnce(EncodedOs),
    /// Deactivate the currently active key table, if any. The next most
    /// recently activated key table (if any) will become active again.
    /// If no key table is active, this action has no effect.
    DeactivateKeyTable,
    /// Deactivate all active key tables. If no active key table exists,
    /// this will report performable as false.
    DeactivateAllKeyTables,
    /// Quit Ghostty.
    Quit,
    /// Crash Ghostty in the desired thread for the focused surface.
    ///
    /// WARNING: This is a hard crash (panic) and data can be lost.
    ///
    /// The purpose of this action is to test crash handling. For some
    /// users, it may be useful to test crash reporting functionality in
    /// order to determine if it all works as expected.
    ///
    /// The value determines the crash location:
    ///
    ///   - `main`
    ///
    ///     Crash on the main (GUI) thread.
    ///
    ///   - `io`
    ///
    ///     Crash on the IO thread for the focused surface.
    ///
    ///   - `render`
    ///
    ///     Crash on the render thread for the focused surface.
    ///
    Crash(CrashThread),
}

/// The tag of an [`Action`] (Zig `Action.Key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ActionKey {
    Ignore,
    Unbind,
    Csi,
    Esc,
    Text,
    CursorKey,
    Reset,
    CopyToClipboard,
    PasteFromClipboard,
    PasteFromSelection,
    CopyUrlToClipboard,
    CopyTitleToClipboard,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    SetFontSize,
    Search,
    SearchSelection,
    NavigateSearch,
    StartSearch,
    EndSearch,
    ClearScreen,
    SelectAll,
    ScrollToTop,
    ScrollToBottom,
    ScrollToSelection,
    ScrollToRow,
    ScrollPageUp,
    ScrollPageDown,
    ScrollPageFractional,
    ScrollPageLines,
    AdjustSelection,
    JumpToPrompt,
    WriteScrollbackFile,
    WriteScreenFile,
    WriteSelectionFile,
    NewWindow,
    NewTab,
    PreviousTab,
    NextTab,
    LastTab,
    GotoTab,
    MoveTab,
    ToggleTabOverview,
    PromptSurfaceTitle,
    PromptTabTitle,
    SetSurfaceTitle,
    SetTabTitle,
    NewSplit,
    GotoSplit,
    GotoWindow,
    ToggleSplitZoom,
    ToggleReadonly,
    ResizeSplit,
    EqualizeSplits,
    ResetWindowSize,
    Inspector,
    ShowGtkInspector,
    ShowOnScreenKeyboard,
    OpenConfig,
    ReloadConfig,
    CloseSurface,
    CloseTab,
    CloseWindow,
    CloseAllWindows,
    ToggleMaximize,
    ToggleFullscreen,
    ToggleWindowDecorations,
    ToggleWindowFloatOnTop,
    ToggleSecureInput,
    ToggleMouseReporting,
    ToggleCommandPalette,
    ToggleQuickTerminal,
    ToggleVisibility,
    ToggleBackgroundOpacity,
    CheckForUpdates,
    Undo,
    Redo,
    EndKeySequence,
    ActivateKeyTable,
    ActivateKeyTableOnce,
    DeactivateKeyTable,
    DeactivateAllKeyTables,
    Quit,
    Crash,
}

impl ActionKey {
    /// All action keys in declaration order.
    pub const ALL: &'static [ActionKey] = &[
        ActionKey::Ignore,
        ActionKey::Unbind,
        ActionKey::Csi,
        ActionKey::Esc,
        ActionKey::Text,
        ActionKey::CursorKey,
        ActionKey::Reset,
        ActionKey::CopyToClipboard,
        ActionKey::PasteFromClipboard,
        ActionKey::PasteFromSelection,
        ActionKey::CopyUrlToClipboard,
        ActionKey::CopyTitleToClipboard,
        ActionKey::IncreaseFontSize,
        ActionKey::DecreaseFontSize,
        ActionKey::ResetFontSize,
        ActionKey::SetFontSize,
        ActionKey::Search,
        ActionKey::SearchSelection,
        ActionKey::NavigateSearch,
        ActionKey::StartSearch,
        ActionKey::EndSearch,
        ActionKey::ClearScreen,
        ActionKey::SelectAll,
        ActionKey::ScrollToTop,
        ActionKey::ScrollToBottom,
        ActionKey::ScrollToSelection,
        ActionKey::ScrollToRow,
        ActionKey::ScrollPageUp,
        ActionKey::ScrollPageDown,
        ActionKey::ScrollPageFractional,
        ActionKey::ScrollPageLines,
        ActionKey::AdjustSelection,
        ActionKey::JumpToPrompt,
        ActionKey::WriteScrollbackFile,
        ActionKey::WriteScreenFile,
        ActionKey::WriteSelectionFile,
        ActionKey::NewWindow,
        ActionKey::NewTab,
        ActionKey::PreviousTab,
        ActionKey::NextTab,
        ActionKey::LastTab,
        ActionKey::GotoTab,
        ActionKey::MoveTab,
        ActionKey::ToggleTabOverview,
        ActionKey::PromptSurfaceTitle,
        ActionKey::PromptTabTitle,
        ActionKey::SetSurfaceTitle,
        ActionKey::SetTabTitle,
        ActionKey::NewSplit,
        ActionKey::GotoSplit,
        ActionKey::GotoWindow,
        ActionKey::ToggleSplitZoom,
        ActionKey::ToggleReadonly,
        ActionKey::ResizeSplit,
        ActionKey::EqualizeSplits,
        ActionKey::ResetWindowSize,
        ActionKey::Inspector,
        ActionKey::ShowGtkInspector,
        ActionKey::ShowOnScreenKeyboard,
        ActionKey::OpenConfig,
        ActionKey::ReloadConfig,
        ActionKey::CloseSurface,
        ActionKey::CloseTab,
        ActionKey::CloseWindow,
        ActionKey::CloseAllWindows,
        ActionKey::ToggleMaximize,
        ActionKey::ToggleFullscreen,
        ActionKey::ToggleWindowDecorations,
        ActionKey::ToggleWindowFloatOnTop,
        ActionKey::ToggleSecureInput,
        ActionKey::ToggleMouseReporting,
        ActionKey::ToggleCommandPalette,
        ActionKey::ToggleQuickTerminal,
        ActionKey::ToggleVisibility,
        ActionKey::ToggleBackgroundOpacity,
        ActionKey::CheckForUpdates,
        ActionKey::Undo,
        ActionKey::Redo,
        ActionKey::EndKeySequence,
        ActionKey::ActivateKeyTable,
        ActionKey::ActivateKeyTableOnce,
        ActionKey::DeactivateKeyTable,
        ActionKey::DeactivateAllKeyTables,
        ActionKey::Quit,
        ActionKey::Crash,
    ];

    /// The action name as used in keybind configuration.
    pub fn name(self) -> &'static str {
        match self {
            ActionKey::Ignore => "ignore",
            ActionKey::Unbind => "unbind",
            ActionKey::Csi => "csi",
            ActionKey::Esc => "esc",
            ActionKey::Text => "text",
            ActionKey::CursorKey => "cursor_key",
            ActionKey::Reset => "reset",
            ActionKey::CopyToClipboard => "copy_to_clipboard",
            ActionKey::PasteFromClipboard => "paste_from_clipboard",
            ActionKey::PasteFromSelection => "paste_from_selection",
            ActionKey::CopyUrlToClipboard => "copy_url_to_clipboard",
            ActionKey::CopyTitleToClipboard => "copy_title_to_clipboard",
            ActionKey::IncreaseFontSize => "increase_font_size",
            ActionKey::DecreaseFontSize => "decrease_font_size",
            ActionKey::ResetFontSize => "reset_font_size",
            ActionKey::SetFontSize => "set_font_size",
            ActionKey::Search => "search",
            ActionKey::SearchSelection => "search_selection",
            ActionKey::NavigateSearch => "navigate_search",
            ActionKey::StartSearch => "start_search",
            ActionKey::EndSearch => "end_search",
            ActionKey::ClearScreen => "clear_screen",
            ActionKey::SelectAll => "select_all",
            ActionKey::ScrollToTop => "scroll_to_top",
            ActionKey::ScrollToBottom => "scroll_to_bottom",
            ActionKey::ScrollToSelection => "scroll_to_selection",
            ActionKey::ScrollToRow => "scroll_to_row",
            ActionKey::ScrollPageUp => "scroll_page_up",
            ActionKey::ScrollPageDown => "scroll_page_down",
            ActionKey::ScrollPageFractional => "scroll_page_fractional",
            ActionKey::ScrollPageLines => "scroll_page_lines",
            ActionKey::AdjustSelection => "adjust_selection",
            ActionKey::JumpToPrompt => "jump_to_prompt",
            ActionKey::WriteScrollbackFile => "write_scrollback_file",
            ActionKey::WriteScreenFile => "write_screen_file",
            ActionKey::WriteSelectionFile => "write_selection_file",
            ActionKey::NewWindow => "new_window",
            ActionKey::NewTab => "new_tab",
            ActionKey::PreviousTab => "previous_tab",
            ActionKey::NextTab => "next_tab",
            ActionKey::LastTab => "last_tab",
            ActionKey::GotoTab => "goto_tab",
            ActionKey::MoveTab => "move_tab",
            ActionKey::ToggleTabOverview => "toggle_tab_overview",
            ActionKey::PromptSurfaceTitle => "prompt_surface_title",
            ActionKey::PromptTabTitle => "prompt_tab_title",
            ActionKey::SetSurfaceTitle => "set_surface_title",
            ActionKey::SetTabTitle => "set_tab_title",
            ActionKey::NewSplit => "new_split",
            ActionKey::GotoSplit => "goto_split",
            ActionKey::GotoWindow => "goto_window",
            ActionKey::ToggleSplitZoom => "toggle_split_zoom",
            ActionKey::ToggleReadonly => "toggle_readonly",
            ActionKey::ResizeSplit => "resize_split",
            ActionKey::EqualizeSplits => "equalize_splits",
            ActionKey::ResetWindowSize => "reset_window_size",
            ActionKey::Inspector => "inspector",
            ActionKey::ShowGtkInspector => "show_gtk_inspector",
            ActionKey::ShowOnScreenKeyboard => "show_on_screen_keyboard",
            ActionKey::OpenConfig => "open_config",
            ActionKey::ReloadConfig => "reload_config",
            ActionKey::CloseSurface => "close_surface",
            ActionKey::CloseTab => "close_tab",
            ActionKey::CloseWindow => "close_window",
            ActionKey::CloseAllWindows => "close_all_windows",
            ActionKey::ToggleMaximize => "toggle_maximize",
            ActionKey::ToggleFullscreen => "toggle_fullscreen",
            ActionKey::ToggleWindowDecorations => "toggle_window_decorations",
            ActionKey::ToggleWindowFloatOnTop => "toggle_window_float_on_top",
            ActionKey::ToggleSecureInput => "toggle_secure_input",
            ActionKey::ToggleMouseReporting => "toggle_mouse_reporting",
            ActionKey::ToggleCommandPalette => "toggle_command_palette",
            ActionKey::ToggleQuickTerminal => "toggle_quick_terminal",
            ActionKey::ToggleVisibility => "toggle_visibility",
            ActionKey::ToggleBackgroundOpacity => "toggle_background_opacity",
            ActionKey::CheckForUpdates => "check_for_updates",
            ActionKey::Undo => "undo",
            ActionKey::Redo => "redo",
            ActionKey::EndKeySequence => "end_key_sequence",
            ActionKey::ActivateKeyTable => "activate_key_table",
            ActionKey::ActivateKeyTableOnce => "activate_key_table_once",
            ActionKey::DeactivateKeyTable => "deactivate_key_table",
            ActionKey::DeactivateAllKeyTables => "deactivate_all_key_tables",
            ActionKey::Quit => "quit",
            ActionKey::Crash => "crash",
        }
    }

    /// The Zig `help_strings.KeybindAction` string: [`ActionKey::docs`]
    /// without the single trailing newline every doc literal carries. (A
    /// trailing empty `///` line in the Zig source is a second `\n`.)
    #[must_use]
    pub fn help_string(self) -> &'static str {
        let d = self.docs();
        d.strip_suffix('\n').unwrap_or(d)
    }

    /// The doc comment of the action (Zig `help_strings.KeybindAction`, generated
    /// at build time from the `///` docs of [`Action`] by `ghostty-helpgen`),
    /// each literal ending in one extra newline.
    pub fn docs(self) -> &'static str {
        include!(concat!(env!("OUT_DIR"), "/keybind_action_docs.rs"))
    }
}

impl Action {
    /// The tag of this action.
    pub fn key(&self) -> ActionKey {
        match self {
            Action::Ignore => ActionKey::Ignore,
            Action::Unbind => ActionKey::Unbind,
            Action::Csi(_) => ActionKey::Csi,
            Action::Esc(_) => ActionKey::Esc,
            Action::Text(_) => ActionKey::Text,
            Action::CursorKey(_) => ActionKey::CursorKey,
            Action::Reset => ActionKey::Reset,
            Action::CopyToClipboard(_) => ActionKey::CopyToClipboard,
            Action::PasteFromClipboard => ActionKey::PasteFromClipboard,
            Action::PasteFromSelection => ActionKey::PasteFromSelection,
            Action::CopyUrlToClipboard => ActionKey::CopyUrlToClipboard,
            Action::CopyTitleToClipboard => ActionKey::CopyTitleToClipboard,
            Action::IncreaseFontSize(_) => ActionKey::IncreaseFontSize,
            Action::DecreaseFontSize(_) => ActionKey::DecreaseFontSize,
            Action::ResetFontSize => ActionKey::ResetFontSize,
            Action::SetFontSize(_) => ActionKey::SetFontSize,
            Action::Search(_) => ActionKey::Search,
            Action::SearchSelection => ActionKey::SearchSelection,
            Action::NavigateSearch(_) => ActionKey::NavigateSearch,
            Action::StartSearch => ActionKey::StartSearch,
            Action::EndSearch => ActionKey::EndSearch,
            Action::ClearScreen => ActionKey::ClearScreen,
            Action::SelectAll => ActionKey::SelectAll,
            Action::ScrollToTop => ActionKey::ScrollToTop,
            Action::ScrollToBottom => ActionKey::ScrollToBottom,
            Action::ScrollToSelection => ActionKey::ScrollToSelection,
            Action::ScrollToRow(_) => ActionKey::ScrollToRow,
            Action::ScrollPageUp => ActionKey::ScrollPageUp,
            Action::ScrollPageDown => ActionKey::ScrollPageDown,
            Action::ScrollPageFractional(_) => ActionKey::ScrollPageFractional,
            Action::ScrollPageLines(_) => ActionKey::ScrollPageLines,
            Action::AdjustSelection(_) => ActionKey::AdjustSelection,
            Action::JumpToPrompt(_) => ActionKey::JumpToPrompt,
            Action::WriteScrollbackFile(_) => ActionKey::WriteScrollbackFile,
            Action::WriteScreenFile(_) => ActionKey::WriteScreenFile,
            Action::WriteSelectionFile(_) => ActionKey::WriteSelectionFile,
            Action::NewWindow => ActionKey::NewWindow,
            Action::NewTab => ActionKey::NewTab,
            Action::PreviousTab => ActionKey::PreviousTab,
            Action::NextTab => ActionKey::NextTab,
            Action::LastTab => ActionKey::LastTab,
            Action::GotoTab(_) => ActionKey::GotoTab,
            Action::MoveTab(_) => ActionKey::MoveTab,
            Action::ToggleTabOverview => ActionKey::ToggleTabOverview,
            Action::PromptSurfaceTitle => ActionKey::PromptSurfaceTitle,
            Action::PromptTabTitle => ActionKey::PromptTabTitle,
            Action::SetSurfaceTitle(_) => ActionKey::SetSurfaceTitle,
            Action::SetTabTitle(_) => ActionKey::SetTabTitle,
            Action::NewSplit(_) => ActionKey::NewSplit,
            Action::GotoSplit(_) => ActionKey::GotoSplit,
            Action::GotoWindow(_) => ActionKey::GotoWindow,
            Action::ToggleSplitZoom => ActionKey::ToggleSplitZoom,
            Action::ToggleReadonly => ActionKey::ToggleReadonly,
            Action::ResizeSplit(_) => ActionKey::ResizeSplit,
            Action::EqualizeSplits => ActionKey::EqualizeSplits,
            Action::ResetWindowSize => ActionKey::ResetWindowSize,
            Action::Inspector(_) => ActionKey::Inspector,
            Action::ShowGtkInspector => ActionKey::ShowGtkInspector,
            Action::ShowOnScreenKeyboard => ActionKey::ShowOnScreenKeyboard,
            Action::OpenConfig => ActionKey::OpenConfig,
            Action::ReloadConfig => ActionKey::ReloadConfig,
            Action::CloseSurface => ActionKey::CloseSurface,
            Action::CloseTab(_) => ActionKey::CloseTab,
            Action::CloseWindow => ActionKey::CloseWindow,
            Action::CloseAllWindows => ActionKey::CloseAllWindows,
            Action::ToggleMaximize => ActionKey::ToggleMaximize,
            Action::ToggleFullscreen => ActionKey::ToggleFullscreen,
            Action::ToggleWindowDecorations => ActionKey::ToggleWindowDecorations,
            Action::ToggleWindowFloatOnTop => ActionKey::ToggleWindowFloatOnTop,
            Action::ToggleSecureInput => ActionKey::ToggleSecureInput,
            Action::ToggleMouseReporting => ActionKey::ToggleMouseReporting,
            Action::ToggleCommandPalette => ActionKey::ToggleCommandPalette,
            Action::ToggleQuickTerminal => ActionKey::ToggleQuickTerminal,
            Action::ToggleVisibility => ActionKey::ToggleVisibility,
            Action::ToggleBackgroundOpacity => ActionKey::ToggleBackgroundOpacity,
            Action::CheckForUpdates => ActionKey::CheckForUpdates,
            Action::Undo => ActionKey::Undo,
            Action::Redo => ActionKey::Redo,
            Action::EndKeySequence => ActionKey::EndKeySequence,
            Action::ActivateKeyTable(_) => ActionKey::ActivateKeyTable,
            Action::ActivateKeyTableOnce(_) => ActionKey::ActivateKeyTableOnce,
            Action::DeactivateKeyTable => ActionKey::DeactivateKeyTable,
            Action::DeactivateAllKeyTables => ActionKey::DeactivateAllKeyTables,
            Action::Quit => ActionKey::Quit,
            Action::Crash(_) => ActionKey::Crash,
        }
    }

    /// Parse the action given its name, the optional parameter (text after the first colon).
    fn parse_named(name: &str, param: Option<&str>) -> Result<Action, Error> {
        match name {
            "ignore" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Ignore)
            }
            "unbind" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Unbind)
            }
            "csi" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::Csi(p.into()))
            }
            "esc" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::Esc(p.into()))
            }
            "text" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::Text(p.into()))
            }
            "cursor_key" => Err(Error::InvalidAction),
            "reset" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Reset)
            }
            "copy_to_clipboard" => {
                let Some(p) = param else {
                    return Ok(Action::CopyToClipboard(CopyToClipboard::DEFAULT));
                };
                Ok(Action::CopyToClipboard(CopyToClipboard::parse_param(p)?))
            }
            "paste_from_clipboard" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::PasteFromClipboard)
            }
            "paste_from_selection" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::PasteFromSelection)
            }
            "copy_url_to_clipboard" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CopyUrlToClipboard)
            }
            "copy_title_to_clipboard" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CopyTitleToClipboard)
            }
            "increase_font_size" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::IncreaseFontSize(f32::parse_param(p)?))
            }
            "decrease_font_size" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::DecreaseFontSize(f32::parse_param(p)?))
            }
            "reset_font_size" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ResetFontSize)
            }
            "set_font_size" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::SetFontSize(f32::parse_param(p)?))
            }
            "search" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::Search(p.into()))
            }
            "search_selection" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::SearchSelection)
            }
            "navigate_search" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::NavigateSearch(NavigateSearch::parse_param(p)?))
            }
            "start_search" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::StartSearch)
            }
            "end_search" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::EndSearch)
            }
            "clear_screen" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ClearScreen)
            }
            "select_all" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::SelectAll)
            }
            "scroll_to_top" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ScrollToTop)
            }
            "scroll_to_bottom" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ScrollToBottom)
            }
            "scroll_to_selection" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ScrollToSelection)
            }
            "scroll_to_row" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::ScrollToRow(usize::parse_param(p)?))
            }
            "scroll_page_up" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ScrollPageUp)
            }
            "scroll_page_down" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ScrollPageDown)
            }
            "scroll_page_fractional" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::ScrollPageFractional(f32::parse_param(p)?))
            }
            "scroll_page_lines" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::ScrollPageLines(i16::parse_param(p)?))
            }
            "adjust_selection" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::AdjustSelection(AdjustSelection::parse_param(p)?))
            }
            "jump_to_prompt" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::JumpToPrompt(i16::parse_param(p)?))
            }
            "write_scrollback_file" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::WriteScrollbackFile(WriteScreen::parse_param(p)?))
            }
            "write_screen_file" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::WriteScreenFile(WriteScreen::parse_param(p)?))
            }
            "write_selection_file" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::WriteSelectionFile(WriteScreen::parse_param(p)?))
            }
            "new_window" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::NewWindow)
            }
            "new_tab" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::NewTab)
            }
            "previous_tab" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::PreviousTab)
            }
            "next_tab" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::NextTab)
            }
            "last_tab" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::LastTab)
            }
            "goto_tab" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::GotoTab(usize::parse_param(p)?))
            }
            "move_tab" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::MoveTab(isize::parse_param(p)?))
            }
            "toggle_tab_overview" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleTabOverview)
            }
            "prompt_surface_title" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::PromptSurfaceTitle)
            }
            "prompt_tab_title" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::PromptTabTitle)
            }
            "set_surface_title" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::SetSurfaceTitle(p.into()))
            }
            "set_tab_title" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::SetTabTitle(p.into()))
            }
            "new_split" => {
                let Some(p) = param else {
                    return Ok(Action::NewSplit(SplitDirection::DEFAULT));
                };
                Ok(Action::NewSplit(SplitDirection::parse_param(p)?))
            }
            "goto_split" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::GotoSplit(SplitFocusDirection::parse_param(p)?))
            }
            "goto_window" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::GotoWindow(GotoWindow::parse_param(p)?))
            }
            "toggle_split_zoom" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleSplitZoom)
            }
            "toggle_readonly" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleReadonly)
            }
            "resize_split" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::ResizeSplit(SplitResizeParameter::parse_param(p)?))
            }
            "equalize_splits" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::EqualizeSplits)
            }
            "reset_window_size" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ResetWindowSize)
            }
            "inspector" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::Inspector(InspectorMode::parse_param(p)?))
            }
            "show_gtk_inspector" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ShowGtkInspector)
            }
            "show_on_screen_keyboard" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ShowOnScreenKeyboard)
            }
            "open_config" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::OpenConfig)
            }
            "reload_config" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ReloadConfig)
            }
            "close_surface" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CloseSurface)
            }
            "close_tab" => {
                let Some(p) = param else {
                    return Ok(Action::CloseTab(CloseTabMode::DEFAULT));
                };
                Ok(Action::CloseTab(CloseTabMode::parse_param(p)?))
            }
            "close_window" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CloseWindow)
            }
            "close_all_windows" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CloseAllWindows)
            }
            "toggle_maximize" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleMaximize)
            }
            "toggle_fullscreen" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleFullscreen)
            }
            "toggle_window_decorations" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleWindowDecorations)
            }
            "toggle_window_float_on_top" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleWindowFloatOnTop)
            }
            "toggle_secure_input" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleSecureInput)
            }
            "toggle_mouse_reporting" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleMouseReporting)
            }
            "toggle_command_palette" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleCommandPalette)
            }
            "toggle_quick_terminal" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleQuickTerminal)
            }
            "toggle_visibility" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleVisibility)
            }
            "toggle_background_opacity" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::ToggleBackgroundOpacity)
            }
            "check_for_updates" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::CheckForUpdates)
            }
            "undo" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Undo)
            }
            "redo" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Redo)
            }
            "end_key_sequence" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::EndKeySequence)
            }
            "activate_key_table" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::ActivateKeyTable(p.into()))
            }
            "activate_key_table_once" => {
                let p = param.ok_or(Error::InvalidFormat)?;
                Ok(Action::ActivateKeyTableOnce(p.into()))
            }
            "deactivate_key_table" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::DeactivateKeyTable)
            }
            "deactivate_all_key_tables" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::DeactivateAllKeyTables)
            }
            "quit" => {
                if param.is_some() {
                    return Err(Error::InvalidFormat);
                }
                Ok(Action::Quit)
            }
            "crash" => {
                let Some(p) = param else {
                    return Err(Error::InvalidFormat);
                };
                Ok(Action::Crash(CrashThread::parse_param(p)?))
            }
            _ => Err(Error::InvalidAction),
        }
    }

    /// Format only the name and parameter pieces. Used by Display.
    fn fmt_into(&self, out: &mut String) {
        out.push_str(self.key().name());
        match self {
            Action::Ignore
            | Action::Unbind
            | Action::Reset
            | Action::PasteFromClipboard
            | Action::PasteFromSelection
            | Action::CopyUrlToClipboard
            | Action::CopyTitleToClipboard
            | Action::ResetFontSize
            | Action::SearchSelection
            | Action::StartSearch
            | Action::EndSearch
            | Action::ClearScreen
            | Action::SelectAll
            | Action::ScrollToTop
            | Action::ScrollToBottom
            | Action::ScrollToSelection
            | Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::NewWindow
            | Action::NewTab
            | Action::PreviousTab
            | Action::NextTab
            | Action::LastTab
            | Action::ToggleTabOverview
            | Action::PromptSurfaceTitle
            | Action::PromptTabTitle
            | Action::ToggleSplitZoom
            | Action::ToggleReadonly
            | Action::EqualizeSplits
            | Action::ResetWindowSize
            | Action::ShowGtkInspector
            | Action::ShowOnScreenKeyboard
            | Action::OpenConfig
            | Action::ReloadConfig
            | Action::CloseSurface
            | Action::CloseWindow
            | Action::CloseAllWindows
            | Action::ToggleMaximize
            | Action::ToggleFullscreen
            | Action::ToggleWindowDecorations
            | Action::ToggleWindowFloatOnTop
            | Action::ToggleSecureInput
            | Action::ToggleMouseReporting
            | Action::ToggleCommandPalette
            | Action::ToggleQuickTerminal
            | Action::ToggleVisibility
            | Action::ToggleBackgroundOpacity
            | Action::CheckForUpdates
            | Action::Undo
            | Action::Redo
            | Action::EndKeySequence
            | Action::DeactivateKeyTable
            | Action::DeactivateAllKeyTables
            | Action::Quit => {}
            Action::Csi(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::Esc(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::Text(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::CursorKey(_) => panic!("formatting not supported"),
            Action::CopyToClipboard(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::IncreaseFontSize(v) => {
                out.push(':');
                out.push_str(&ghostty_os::zig_fmt::format_float_d(*v));
            }
            Action::DecreaseFontSize(v) => {
                out.push(':');
                out.push_str(&ghostty_os::zig_fmt::format_float_d(*v));
            }
            Action::SetFontSize(v) => {
                out.push(':');
                out.push_str(&ghostty_os::zig_fmt::format_float_d(*v));
            }
            Action::Search(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::NavigateSearch(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::ScrollToRow(v) => {
                out.push(':');
                out.push_str(&v.to_string());
            }
            Action::ScrollPageFractional(v) => {
                out.push(':');
                out.push_str(&ghostty_os::zig_fmt::format_float_d(*v));
            }
            Action::ScrollPageLines(v) => {
                out.push(':');
                out.push_str(&v.to_string());
            }
            Action::AdjustSelection(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::JumpToPrompt(v) => {
                out.push(':');
                out.push_str(&v.to_string());
            }
            Action::WriteScrollbackFile(v) => {
                out.push(':');
                v.format_into(out);
            }
            Action::WriteScreenFile(v) => {
                out.push(':');
                v.format_into(out);
            }
            Action::WriteSelectionFile(v) => {
                out.push(':');
                v.format_into(out);
            }
            Action::GotoTab(v) => {
                out.push(':');
                out.push_str(&v.to_string());
            }
            Action::MoveTab(v) => {
                out.push(':');
                out.push_str(&v.to_string());
            }
            Action::SetSurfaceTitle(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::SetTabTitle(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::NewSplit(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::GotoSplit(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::GotoWindow(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::ResizeSplit(v) => {
                out.push(':');
                v.format_into(out);
            }
            Action::Inspector(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::CloseTab(v) => {
                out.push(':');
                out.push_str(v.name());
            }
            Action::ActivateKeyTable(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::ActivateKeyTableOnce(v) => {
                out.push(':');
                string_escape(&v.to_bytes(), out);
            }
            Action::Crash(v) => {
                out.push(':');
                out.push_str(v.name());
            }
        }
    }

    pub fn hash_into<H: std::hash::Hasher>(&self, h: &mut H) {
        use std::hash::Hash;
        self.key().hash(h);
        match self {
            Action::Ignore
            | Action::Unbind
            | Action::Reset
            | Action::PasteFromClipboard
            | Action::PasteFromSelection
            | Action::CopyUrlToClipboard
            | Action::CopyTitleToClipboard
            | Action::ResetFontSize
            | Action::SearchSelection
            | Action::StartSearch
            | Action::EndSearch
            | Action::ClearScreen
            | Action::SelectAll
            | Action::ScrollToTop
            | Action::ScrollToBottom
            | Action::ScrollToSelection
            | Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::NewWindow
            | Action::NewTab
            | Action::PreviousTab
            | Action::NextTab
            | Action::LastTab
            | Action::ToggleTabOverview
            | Action::PromptSurfaceTitle
            | Action::PromptTabTitle
            | Action::ToggleSplitZoom
            | Action::ToggleReadonly
            | Action::EqualizeSplits
            | Action::ResetWindowSize
            | Action::ShowGtkInspector
            | Action::ShowOnScreenKeyboard
            | Action::OpenConfig
            | Action::ReloadConfig
            | Action::CloseSurface
            | Action::CloseWindow
            | Action::CloseAllWindows
            | Action::ToggleMaximize
            | Action::ToggleFullscreen
            | Action::ToggleWindowDecorations
            | Action::ToggleWindowFloatOnTop
            | Action::ToggleSecureInput
            | Action::ToggleMouseReporting
            | Action::ToggleCommandPalette
            | Action::ToggleQuickTerminal
            | Action::ToggleVisibility
            | Action::ToggleBackgroundOpacity
            | Action::CheckForUpdates
            | Action::Undo
            | Action::Redo
            | Action::EndKeySequence
            | Action::DeactivateKeyTable
            | Action::DeactivateAllKeyTables
            | Action::Quit => {}
            Action::Csi(v) => v.hash(h),
            Action::Esc(v) => v.hash(h),
            Action::Text(v) => v.hash(h),
            Action::CursorKey(v) => v.hash(h),
            Action::CopyToClipboard(v) => v.hash(h),
            Action::IncreaseFontSize(v) => v.to_bits().hash(h),
            Action::DecreaseFontSize(v) => v.to_bits().hash(h),
            Action::SetFontSize(v) => v.to_bits().hash(h),
            Action::Search(v) => v.hash(h),
            Action::NavigateSearch(v) => v.hash(h),
            Action::ScrollToRow(v) => v.hash(h),
            Action::ScrollPageFractional(v) => v.to_bits().hash(h),
            Action::ScrollPageLines(v) => v.hash(h),
            Action::AdjustSelection(v) => v.hash(h),
            Action::JumpToPrompt(v) => v.hash(h),
            Action::WriteScrollbackFile(v) => v.hash(h),
            Action::WriteScreenFile(v) => v.hash(h),
            Action::WriteSelectionFile(v) => v.hash(h),
            Action::GotoTab(v) => v.hash(h),
            Action::MoveTab(v) => v.hash(h),
            Action::SetSurfaceTitle(v) => v.hash(h),
            Action::SetTabTitle(v) => v.hash(h),
            Action::NewSplit(v) => v.hash(h),
            Action::GotoSplit(v) => v.hash(h),
            Action::GotoWindow(v) => v.hash(h),
            Action::ResizeSplit(v) => v.hash(h),
            Action::Inspector(v) => v.hash(h),
            Action::CloseTab(v) => v.hash(h),
            Action::ActivateKeyTable(v) => v.hash(h),
            Action::ActivateKeyTableOnce(v) => v.hash(h),
            Action::Crash(v) => v.hash(h),
        }
    }

    /// Returns the scope of an action.
    pub fn scope(&self) -> Scope {
        self.key().scope()
    }
}

impl ActionKey {
    /// Returns the scope of an action.
    pub fn scope(self) -> Scope {
        match self {
            ActionKey::Ignore
            | ActionKey::Unbind
            | ActionKey::OpenConfig
            | ActionKey::ReloadConfig
            | ActionKey::CloseAllWindows
            | ActionKey::Quit
            | ActionKey::ToggleQuickTerminal
            | ActionKey::ToggleVisibility
            | ActionKey::CheckForUpdates
            | ActionKey::ShowGtkInspector
            | ActionKey::NewWindow
            | ActionKey::Undo
            | ActionKey::Redo => Scope::App,
            ActionKey::Csi
            | ActionKey::Esc
            | ActionKey::Text
            | ActionKey::CursorKey
            | ActionKey::Search
            | ActionKey::NavigateSearch
            | ActionKey::SearchSelection
            | ActionKey::StartSearch
            | ActionKey::EndSearch
            | ActionKey::Reset
            | ActionKey::CopyToClipboard
            | ActionKey::CopyUrlToClipboard
            | ActionKey::CopyTitleToClipboard
            | ActionKey::PasteFromClipboard
            | ActionKey::PasteFromSelection
            | ActionKey::IncreaseFontSize
            | ActionKey::DecreaseFontSize
            | ActionKey::ResetFontSize
            | ActionKey::SetFontSize
            | ActionKey::PromptSurfaceTitle
            | ActionKey::PromptTabTitle
            | ActionKey::SetSurfaceTitle
            | ActionKey::SetTabTitle
            | ActionKey::ClearScreen
            | ActionKey::SelectAll
            | ActionKey::ScrollToTop
            | ActionKey::ScrollToBottom
            | ActionKey::ScrollToSelection
            | ActionKey::ScrollToRow
            | ActionKey::ScrollPageUp
            | ActionKey::ScrollPageDown
            | ActionKey::ScrollPageFractional
            | ActionKey::ScrollPageLines
            | ActionKey::AdjustSelection
            | ActionKey::JumpToPrompt
            | ActionKey::WriteScrollbackFile
            | ActionKey::WriteScreenFile
            | ActionKey::WriteSelectionFile
            | ActionKey::CloseSurface
            | ActionKey::CloseTab
            | ActionKey::CloseWindow
            | ActionKey::ToggleMaximize
            | ActionKey::ToggleFullscreen
            | ActionKey::ToggleWindowDecorations
            | ActionKey::ToggleWindowFloatOnTop
            | ActionKey::ToggleSecureInput
            | ActionKey::ToggleMouseReporting
            | ActionKey::ToggleCommandPalette
            | ActionKey::ToggleBackgroundOpacity
            | ActionKey::ShowOnScreenKeyboard
            | ActionKey::ResetWindowSize
            | ActionKey::ActivateKeyTable
            | ActionKey::ActivateKeyTableOnce
            | ActionKey::DeactivateKeyTable
            | ActionKey::DeactivateAllKeyTables
            | ActionKey::EndKeySequence
            | ActionKey::Crash
            | ActionKey::NewTab
            | ActionKey::PreviousTab
            | ActionKey::NextTab
            | ActionKey::LastTab
            | ActionKey::GotoTab
            | ActionKey::MoveTab
            | ActionKey::ToggleTabOverview
            | ActionKey::NewSplit
            | ActionKey::GotoSplit
            | ActionKey::GotoWindow
            | ActionKey::ToggleSplitZoom
            | ActionKey::ToggleReadonly
            | ActionKey::ResizeSplit
            | ActionKey::EqualizeSplits
            | ActionKey::Inspector => Scope::Surface,
        }
    }

    /// Look up an action key by name.
    pub fn from_name(s: &str) -> Option<ActionKey> {
        match s {
            "ignore" => Some(ActionKey::Ignore),
            "unbind" => Some(ActionKey::Unbind),
            "csi" => Some(ActionKey::Csi),
            "esc" => Some(ActionKey::Esc),
            "text" => Some(ActionKey::Text),
            "cursor_key" => Some(ActionKey::CursorKey),
            "reset" => Some(ActionKey::Reset),
            "copy_to_clipboard" => Some(ActionKey::CopyToClipboard),
            "paste_from_clipboard" => Some(ActionKey::PasteFromClipboard),
            "paste_from_selection" => Some(ActionKey::PasteFromSelection),
            "copy_url_to_clipboard" => Some(ActionKey::CopyUrlToClipboard),
            "copy_title_to_clipboard" => Some(ActionKey::CopyTitleToClipboard),
            "increase_font_size" => Some(ActionKey::IncreaseFontSize),
            "decrease_font_size" => Some(ActionKey::DecreaseFontSize),
            "reset_font_size" => Some(ActionKey::ResetFontSize),
            "set_font_size" => Some(ActionKey::SetFontSize),
            "search" => Some(ActionKey::Search),
            "search_selection" => Some(ActionKey::SearchSelection),
            "navigate_search" => Some(ActionKey::NavigateSearch),
            "start_search" => Some(ActionKey::StartSearch),
            "end_search" => Some(ActionKey::EndSearch),
            "clear_screen" => Some(ActionKey::ClearScreen),
            "select_all" => Some(ActionKey::SelectAll),
            "scroll_to_top" => Some(ActionKey::ScrollToTop),
            "scroll_to_bottom" => Some(ActionKey::ScrollToBottom),
            "scroll_to_selection" => Some(ActionKey::ScrollToSelection),
            "scroll_to_row" => Some(ActionKey::ScrollToRow),
            "scroll_page_up" => Some(ActionKey::ScrollPageUp),
            "scroll_page_down" => Some(ActionKey::ScrollPageDown),
            "scroll_page_fractional" => Some(ActionKey::ScrollPageFractional),
            "scroll_page_lines" => Some(ActionKey::ScrollPageLines),
            "adjust_selection" => Some(ActionKey::AdjustSelection),
            "jump_to_prompt" => Some(ActionKey::JumpToPrompt),
            "write_scrollback_file" => Some(ActionKey::WriteScrollbackFile),
            "write_screen_file" => Some(ActionKey::WriteScreenFile),
            "write_selection_file" => Some(ActionKey::WriteSelectionFile),
            "new_window" => Some(ActionKey::NewWindow),
            "new_tab" => Some(ActionKey::NewTab),
            "previous_tab" => Some(ActionKey::PreviousTab),
            "next_tab" => Some(ActionKey::NextTab),
            "last_tab" => Some(ActionKey::LastTab),
            "goto_tab" => Some(ActionKey::GotoTab),
            "move_tab" => Some(ActionKey::MoveTab),
            "toggle_tab_overview" => Some(ActionKey::ToggleTabOverview),
            "prompt_surface_title" => Some(ActionKey::PromptSurfaceTitle),
            "prompt_tab_title" => Some(ActionKey::PromptTabTitle),
            "set_surface_title" => Some(ActionKey::SetSurfaceTitle),
            "set_tab_title" => Some(ActionKey::SetTabTitle),
            "new_split" => Some(ActionKey::NewSplit),
            "goto_split" => Some(ActionKey::GotoSplit),
            "goto_window" => Some(ActionKey::GotoWindow),
            "toggle_split_zoom" => Some(ActionKey::ToggleSplitZoom),
            "toggle_readonly" => Some(ActionKey::ToggleReadonly),
            "resize_split" => Some(ActionKey::ResizeSplit),
            "equalize_splits" => Some(ActionKey::EqualizeSplits),
            "reset_window_size" => Some(ActionKey::ResetWindowSize),
            "inspector" => Some(ActionKey::Inspector),
            "show_gtk_inspector" => Some(ActionKey::ShowGtkInspector),
            "show_on_screen_keyboard" => Some(ActionKey::ShowOnScreenKeyboard),
            "open_config" => Some(ActionKey::OpenConfig),
            "reload_config" => Some(ActionKey::ReloadConfig),
            "close_surface" => Some(ActionKey::CloseSurface),
            "close_tab" => Some(ActionKey::CloseTab),
            "close_window" => Some(ActionKey::CloseWindow),
            "close_all_windows" => Some(ActionKey::CloseAllWindows),
            "toggle_maximize" => Some(ActionKey::ToggleMaximize),
            "toggle_fullscreen" => Some(ActionKey::ToggleFullscreen),
            "toggle_window_decorations" => Some(ActionKey::ToggleWindowDecorations),
            "toggle_window_float_on_top" => Some(ActionKey::ToggleWindowFloatOnTop),
            "toggle_secure_input" => Some(ActionKey::ToggleSecureInput),
            "toggle_mouse_reporting" => Some(ActionKey::ToggleMouseReporting),
            "toggle_command_palette" => Some(ActionKey::ToggleCommandPalette),
            "toggle_quick_terminal" => Some(ActionKey::ToggleQuickTerminal),
            "toggle_visibility" => Some(ActionKey::ToggleVisibility),
            "toggle_background_opacity" => Some(ActionKey::ToggleBackgroundOpacity),
            "check_for_updates" => Some(ActionKey::CheckForUpdates),
            "undo" => Some(ActionKey::Undo),
            "redo" => Some(ActionKey::Redo),
            "end_key_sequence" => Some(ActionKey::EndKeySequence),
            "activate_key_table" => Some(ActionKey::ActivateKeyTable),
            "activate_key_table_once" => Some(ActionKey::ActivateKeyTableOnce),
            "deactivate_key_table" => Some(ActionKey::DeactivateKeyTable),
            "deactivate_all_key_tables" => Some(ActionKey::DeactivateAllKeyTables),
            "quit" => Some(ActionKey::Quit),
            "crash" => Some(ActionKey::Crash),
            _ => None,
        }
    }
}
