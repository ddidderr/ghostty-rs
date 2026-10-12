//! Command palette entries (ZIG/src/input/command.zig).

use std::sync::OnceLock;

use crate::binding::{
    Action,
    CloseTabMode,
    CopyToClipboard,
    GotoWindow,
    InspectorMode,
    NavigateSearch,
    SplitDirection,
    SplitFocusDirection,
    WriteScreen,
    WriteScreenAction,
    WriteScreenFormat,
};

/// A command is a named binding action that can be executed from
/// something like a command palette.
///
/// A command must be associated with a binding; all commands can be
/// mapped to traditional `keybind` configurations. This restriction
/// makes it so that there is nothing special about commands and likewise
/// it makes it trivial and consistent to define custom commands.
///
/// For apprt implementers: a command palette doesn't have to make use
/// of all the fields here. We try to provide as much information as
/// possible to make it easier to implement a command palette in the way
/// that makes the most sense for the application.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub action: Action,
    pub title: ghostty_os::osbytes::EncodedOs,
    pub description: ghostty_os::osbytes::EncodedOs,
}

impl Command {
    pub fn new(action: Action, title: &str, description: &str) -> Command {
        Command {
            action,
            title: title.into(),
            description: description.into(),
        }
    }

    pub fn equal(&self, other: &Command) -> bool {
        if self.action.hash() != other.action.hash() {
            return false;
        }
        self.title == other.title && self.description == other.description
    }

    /// Implements a comparison function for sorting. The sorting is defined
    /// by Ghostty to be what we prefer. If a caller wants some other
    /// sorting, they should do it themselves.
    pub fn less_than(lhs: &Command, rhs: &Command) -> bool {
        cmp_ignore_case(&lhs.title, &rhs.title) == std::cmp::Ordering::Less
    }

    /// The `ghostty_command_s` equivalent: (action key name, action string,
    /// title, description).
    pub fn cval(&self) -> CommandC {
        CommandC {
            action_key: self.action.name().to_string(),
            action: self.action.to_string(),
            title: self.title.clone(),
            description: self.description.clone(),
        }
    }
}

/// Owned form of `Command.C`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandC {
    pub action_key: String,
    pub action: String,
    pub title: ghostty_os::osbytes::EncodedOs,
    pub description: ghostty_os::osbytes::EncodedOs,
}

/// `std.ascii.orderIgnoreCase`
fn cmp_ignore_case(
    a: &ghostty_os::osbytes::EncodedOs,
    b: &ghostty_os::osbytes::EncodedOs,
) -> std::cmp::Ordering {
    let a = a.to_bytes().into_iter().map(|c| c.to_ascii_lowercase());
    let b = b.to_bytes().into_iter().map(|c| c.to_ascii_lowercase());
    a.cmp(b)
}

/// The default command palette entries, sorted by title (ignoring case).
pub fn defaults() -> &'static [Command] {
    static DEFAULTS: OnceLock<Vec<Command>> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let mut result = default_commands();
        result.sort_by(|a, b| cmp_ignore_case(&a.title, &b.title));
        result
    })
}

/// Defaults in C-compatible form.
pub fn defaults_c() -> Vec<CommandC> {
    defaults().iter().map(Command::cval).collect()
}

// The set of commands associated with each action key by default. Not all
// actions should have commands. As a general guideline, an action should have
// a command only if it is useful and reasonable to appear in a command
// palette. Actions without commands (parameterized actions without obvious
// values, ignore/unbind, etc.) are: csi, esc, cursor_key, set_font_size,
// set_surface_title, set_tab_title, search, scroll_to_row,
// scroll_page_fractional, scroll_page_lines, adjust_selection, jump_to_prompt,
// write_scrollback_file, goto_tab, resize_split, activate_key_table,
// activate_key_table_once, deactivate_key_table, deactivate_all_key_tables,
// end_key_sequence, crash, toggle_command_palette, toggle_quick_terminal,
// toggle_visibility, previous_tab, next_tab, last_tab, ignore, unbind.
fn default_commands() -> Vec<Command> {
    vec![
        Command::new(
            Action::Reset,
            "Reset Terminal",
            "Reset the terminal to a clean state.",
        ),
        Command::new(
            Action::CopyToClipboard(CopyToClipboard::Mixed),
            "Copy to Clipboard",
            "Copy the selected text to the clipboard in both plain and styled formats.",
        ),
        Command::new(
            Action::CopyToClipboard(CopyToClipboard::Plain),
            "Copy Selection as Plain Text to Clipboard",
            "Copy the selected text as plain text to the clipboard.",
        ),
        Command::new(
            Action::CopyToClipboard(CopyToClipboard::Vt),
            "Copy Selection as ANSI Sequences to Clipboard",
            "Copy the selected text as ANSI escape sequences to the clipboard.",
        ),
        Command::new(
            Action::CopyToClipboard(CopyToClipboard::Html),
            "Copy Selection as HTML to Clipboard",
            "Copy the selected text as HTML to the clipboard.",
        ),
        Command::new(
            Action::CopyUrlToClipboard,
            "Copy URL to Clipboard",
            "Copy the URL under the cursor to the clipboard.",
        ),
        Command::new(
            Action::CopyTitleToClipboard,
            "Copy Terminal Title to Clipboard",
            "Copy the terminal title to the clipboard. If the terminal title is not set this has no effect.",
        ),
        Command::new(
            Action::PasteFromClipboard,
            "Paste from Clipboard",
            "Paste the contents of the main clipboard.",
        ),
        Command::new(
            Action::PasteFromSelection,
            "Paste from Selection",
            "Paste the contents of the selection clipboard.",
        ),
        Command::new(
            Action::StartSearch,
            "Start Search",
            "Start a search if one isn't already active.",
        ),
        Command::new(
            Action::SearchSelection,
            "Search Selection",
            "Start a search for the current text selection.",
        ),
        Command::new(
            Action::EndSearch,
            "End Search",
            "End the current search if any and hide any GUI elements.",
        ),
        Command::new(
            Action::NavigateSearch(NavigateSearch::Next),
            "Next Search Result",
            "Navigate to the next search result, if any.",
        ),
        Command::new(
            Action::NavigateSearch(NavigateSearch::Previous),
            "Previous Search Result",
            "Navigate to the previous search result, if any.",
        ),
        Command::new(
            Action::IncreaseFontSize(1.0),
            "Increase Font Size",
            "Increase the font size by 1 point.",
        ),
        Command::new(
            Action::DecreaseFontSize(1.0),
            "Decrease Font Size",
            "Decrease the font size by 1 point.",
        ),
        Command::new(
            Action::ResetFontSize,
            "Reset Font Size",
            "Reset the font size to the default.",
        ),
        Command::new(
            Action::ClearScreen,
            "Clear Screen",
            "Clear the screen and scrollback.",
        ),
        Command::new(
            Action::SelectAll,
            "Select All",
            "Select all text on the screen.",
        ),
        Command::new(
            Action::ScrollToTop,
            "Scroll to Top",
            "Scroll to the top of the screen.",
        ),
        Command::new(
            Action::ScrollToBottom,
            "Scroll to Bottom",
            "Scroll to the bottom of the screen.",
        ),
        Command::new(
            Action::ScrollToSelection,
            "Scroll to Selection",
            "Scroll to the selected text.",
        ),
        Command::new(
            Action::ScrollPageUp,
            "Scroll Page Up",
            "Scroll the screen up by a page.",
        ),
        Command::new(
            Action::ScrollPageDown,
            "Scroll Page Down",
            "Scroll the screen down by a page.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen::COPY),
            "Copy Screen to Temporary File and Copy Path",
            "Copy the screen contents to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen::PASTE),
            "Copy Screen to Temporary File and Paste Path",
            "Copy the screen contents to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen::OPEN),
            "Copy Screen to Temporary File and Open",
            "Copy the screen contents to a temporary file and open it.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Copy,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Screen as HTML to Temporary File and Copy Path",
            "Copy the screen contents as HTML to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Paste,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Screen as HTML to Temporary File and Paste Path",
            "Copy the screen contents as HTML to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Open,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Screen as HTML to Temporary File and Open",
            "Copy the screen contents as HTML to a temporary file and open it.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Copy,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Screen as ANSI Sequences to Temporary File and Copy Path",
            "Copy the screen contents as ANSI escape sequences to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Paste,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Screen as ANSI Sequences to Temporary File and Paste Path",
            "Copy the screen contents as ANSI escape sequences to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteScreenFile(WriteScreen {
                action: WriteScreenAction::Open,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Screen as ANSI Sequences to Temporary File and Open",
            "Copy the screen contents as ANSI escape sequences to a temporary file and open it.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen::COPY),
            "Copy Selection to Temporary File and Copy Path",
            "Copy the selection contents to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen::PASTE),
            "Copy Selection to Temporary File and Paste Path",
            "Copy the selection contents to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen::OPEN),
            "Copy Selection to Temporary File and Open",
            "Copy the selection contents to a temporary file and open it.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Copy,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Selection as HTML to Temporary File and Copy Path",
            "Copy the selection contents as HTML to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Paste,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Selection as HTML to Temporary File and Paste Path",
            "Copy the selection contents as HTML to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Open,
                emit: WriteScreenFormat::Html,
            }),
            "Copy Selection as HTML to Temporary File and Open",
            "Copy the selection contents as HTML to a temporary file and open it.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Copy,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Selection as ANSI Sequences to Temporary File and Copy Path",
            "Copy the selection contents as ANSI escape sequences to a temporary file and copy the path to the clipboard.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Paste,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Selection as ANSI Sequences to Temporary File and Paste Path",
            "Copy the selection contents as ANSI escape sequences to a temporary file and paste the path to the file.",
        ),
        Command::new(
            Action::WriteSelectionFile(WriteScreen {
                action: WriteScreenAction::Open,
                emit: WriteScreenFormat::Vt,
            }),
            "Copy Selection as ANSI Sequences to Temporary File and Open",
            "Copy the selection contents as ANSI escape sequences to a temporary file and open it.",
        ),
        Command::new(Action::NewWindow, "New Window", "Open a new window."),
        Command::new(Action::NewTab, "New Tab", "Open a new tab."),
        Command::new(
            Action::MoveTab(-1),
            "Move Tab Left",
            "Move the current tab to the left.",
        ),
        Command::new(
            Action::MoveTab(1),
            "Move Tab Right",
            "Move the current tab to the right.",
        ),
        Command::new(
            Action::ToggleTabOverview,
            "Toggle Tab Overview",
            "Toggle the tab overview.",
        ),
        Command::new(
            Action::PromptSurfaceTitle,
            "Change Terminal Title…",
            "Prompt for a new title for the current terminal.",
        ),
        Command::new(
            Action::PromptTabTitle,
            "Change Tab Title…",
            "Prompt for a new title for the current tab.",
        ),
        Command::new(
            Action::NewSplit(SplitDirection::Left),
            "Split Left",
            "Split the terminal to the left.",
        ),
        Command::new(
            Action::NewSplit(SplitDirection::Right),
            "Split Right",
            "Split the terminal to the right.",
        ),
        Command::new(
            Action::NewSplit(SplitDirection::Up),
            "Split Up",
            "Split the terminal up.",
        ),
        Command::new(
            Action::NewSplit(SplitDirection::Down),
            "Split Down",
            "Split the terminal down.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Previous),
            "Focus Split: Previous",
            "Focus the previous split, if any.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Next),
            "Focus Split: Next",
            "Focus the next split, if any.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Left),
            "Focus Split: Left",
            "Focus the split to the left, if it exists.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Right),
            "Focus Split: Right",
            "Focus the split to the right, if it exists.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Up),
            "Focus Split: Up",
            "Focus the split above, if it exists.",
        ),
        Command::new(
            Action::GotoSplit(SplitFocusDirection::Down),
            "Focus Split: Down",
            "Focus the split below, if it exists.",
        ),
        Command::new(
            Action::GotoWindow(GotoWindow::Previous),
            "Focus Window: Previous",
            "Focus the previous window, if any.",
        ),
        Command::new(
            Action::GotoWindow(GotoWindow::Next),
            "Focus Window: Next",
            "Focus the next window, if any.",
        ),
        Command::new(
            Action::ToggleSplitZoom,
            "Toggle Split Zoom",
            "Toggle the zoom state of the current split.",
        ),
        Command::new(
            Action::ToggleReadonly,
            "Toggle Read-Only Mode",
            "Toggle read-only mode for the current surface.",
        ),
        Command::new(
            Action::EqualizeSplits,
            "Equalize Splits",
            "Equalize the size of all splits.",
        ),
        Command::new(
            Action::ResetWindowSize,
            "Reset Window Size",
            "Reset the window size to the default.",
        ),
        Command::new(
            Action::Inspector(InspectorMode::Toggle),
            "Toggle Inspector",
            "Toggle the inspector.",
        ),
        Command::new(
            Action::ShowGtkInspector,
            "Show the GTK Inspector",
            "Show the GTK inspector.",
        ),
        Command::new(
            Action::ShowOnScreenKeyboard,
            "Show On-Screen Keyboard",
            "Show the on-screen keyboard if present.",
        ),
        Command::new(Action::OpenConfig, "Open Config", "Open the config file."),
        Command::new(
            Action::ReloadConfig,
            "Reload Config",
            "Reload the config file.",
        ),
        Command::new(
            Action::CloseSurface,
            "Close Terminal",
            "Close the current terminal.",
        ),
        Command::new(
            Action::CloseTab(CloseTabMode::This),
            "Close Tab",
            "Close the current tab.",
        ),
        Command::new(
            Action::CloseTab(CloseTabMode::Other),
            "Close Other Tabs",
            "Close all tabs in this window except the current one.",
        ),
        Command::new(
            Action::CloseTab(CloseTabMode::Right),
            "Close Tabs to the Right",
            "Close all tabs to the right of the current one.",
        ),
        Command::new(
            Action::CloseWindow,
            "Close Window",
            "Close the current window.",
        ),
        Command::new(
            Action::CloseAllWindows,
            "Close All Windows",
            "Close all windows.",
        ),
        Command::new(
            Action::ToggleMaximize,
            "Toggle Maximize",
            "Toggle the maximized state of the current window.",
        ),
        Command::new(
            Action::ToggleFullscreen,
            "Toggle Fullscreen",
            "Toggle the fullscreen state of the current window.",
        ),
        Command::new(
            Action::ToggleWindowDecorations,
            "Toggle Window Decorations",
            "Toggle the window decorations.",
        ),
        Command::new(
            Action::ToggleWindowFloatOnTop,
            "Toggle Float on Top",
            "Toggle the float on top state of the current window.",
        ),
        Command::new(
            Action::ToggleSecureInput,
            "Toggle Secure Input",
            "Toggle secure input mode.",
        ),
        Command::new(
            Action::ToggleMouseReporting,
            "Toggle Mouse Reporting",
            "Toggle whether mouse events are reported to terminal applications.",
        ),
        Command::new(
            Action::ToggleBackgroundOpacity,
            "Toggle Background Opacity",
            "Toggle the background opacity of a window that started transparent.",
        ),
        Command::new(
            Action::CheckForUpdates,
            "Check for Updates",
            "Check for updates to the application.",
        ),
        Command::new(Action::Undo, "Undo", "Undo the last action."),
        Command::new(Action::Redo, "Redo", "Redo the last undone action."),
        Command::new(Action::Quit, "Quit", "Quit the application."),
        Command::new(
            Action::Text("👻".into()),
            "Ghostty",
            "Put a little Ghostty in your terminal.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_defaults() {
        // This just ensures that defaults is analyzed and works.
        assert_ne!(defaults().len(), 0);
        assert_eq!(defaults().len(), defaults_c().len());
    }

    #[test]
    fn command_defaults_sorted_and_roundtrip() {
        let d = defaults();
        for w in d.windows(2) {
            assert_ne!(
                cmp_ignore_case(&w[0].title, &w[1].title),
                std::cmp::Ordering::Greater
            );
        }
        // Round trip every default action through its string form.
        // (`text` is stored raw and escaped on format, so it is excluded.)
        for c in d.iter().filter(|c| !matches!(c.action, Action::Text(_))) {
            let s = c.action.to_string();
            assert_eq!(Action::parse(&s).expect("parse"), c.action, "{s}");
        }
    }
}
