//! tmux control mode viewer (port of `tmux/viewer.zig`).
//!
//! A viewer is a tmux control mode client that attempts to create a remote
//! view of a tmux session, including providing the ability to send new
//! input to the session.
//!
//! This struct helps move through a state machine of connecting to a tmux
//! session, negotiating capabilities, listing window state, etc.
//!
//! ## Viewer Lifecycle
//!
//! ```text
//! DCS 1000p received by host
//!   -> startup_block     wait for the initial %begin/%end block (response to
//!                        the initial command, e.g. "attach -t 0")
//!   -> startup_session   wait for %session-changed to get the session ID
//!   -> command_queue     main operating state: process commands sequentially
//!                        and handle notifications
//!        tmux_version    query tmux version
//!        list_windows    get all windows in the session
//!        sync layouts    parse layouts; for each new pane queue
//!                        pane_history/pane_visible (primary, alternate)
//!        pane_state      cursor position/style, modes, tabs, scroll region
//!   -> ready: %output, %layout-change, %session-changed etc. are handled live.
//! ```
//!
//! ## Error Handling
//!
//! At any point, if an unrecoverable error occurs or tmux sends `%exit`,
//! the viewer transitions to the `defunct` state and emits an `Exit` action.
//!
//! ## Session Changes
//!
//! When `%session-changed` is received during `command_queue` state, the
//! viewer resets itself completely: clears all windows/panes, emits an
//! empty windows action, and restarts the `list_windows` flow for the new
//! session.
#![allow(
    clippy::assert_is_empty,
    clippy::assigning_clones,
    clippy::collapsible_if,
    clippy::enum_glob_use,
    clippy::manual_assert_eq,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::type_complexity,
    clippy::unnecessary_wraps
)]

// TODO(zig): A list of TODOs from the original:
// - We need to make startup more robust so session and block can happen
//   out of order.
// - We need to ignore `output` for panes that aren't yet initialized
//   (until capture-panes are complete).
// - We should note what the active window pane is on the tmux side;
//   we can use this at least for initial focus.

// NOTE: There is some fragility here that can possibly break if tmux
// changes their implementation. In particular, the order of notifications
// and assurances about what is sent when are based on reading the tmux
// source code as of Dec, 2025. These aren't documented as fixed.

use std::collections::VecDeque;

use ghostty_vt::{modes::Mode, tmux::control::Notification};

use super::{
    layout::{self, Content, Layout},
    output::{self, Variable},
};
use crate::{
    cursor::Style as CursorStyle,
    page,
    point::{Coordinate, Point},
    screen_set::Key as ScreenKey,
    size::CellCountInt,
    terminal::{Options as TerminalOptions, Terminal},
};

/// The initial capacity of the command queue.
const COMMAND_QUEUE_INITIAL: usize = 8;

/// Errors that can occur while processing tmux input. These never escape
/// [`Viewer::next`]; they make the viewer go defunct (or are logged).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Format output parse failure.
    #[error("format parse: {0}")]
    Output(#[from] output::ParseError),
    /// Layout parse failure.
    #[error("layout parse: {0}")]
    Layout(#[from] layout::ParseError),
    /// Terminal/page failure.
    #[error("terminal: {0:?}")]
    Terminal(page::Error),
    /// VT stream failure.
    #[error("stream: {0}")]
    Stream(String),
    /// Window/pane dimension doesn't fit a cell count.
    #[error("dimension overflow")]
    Overflow,
}

impl From<page::Error> for Error {
    fn from(e: page::Error) -> Self {
        Error::Terminal(e)
    }
}

/// An action the caller must perform as the result of an input.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Tmux has closed the control mode connection, we should end
    /// our viewer session in some way.
    Exit,

    /// Send a command to tmux, e.g. `list-windows`. The caller should not
    /// worry about parsing this or reading what command it is; just send it
    /// to tmux as-is. This includes the trailing newline.
    Command(Vec<u8>),

    /// Windows changed. This may add, remove or change windows. The caller
    /// is responsible for diffing the new window list against the prior one.
    /// For a given Viewer, window IDs are stable (and tmux never reuses
    /// window IDs within a server process lifetime as of Dec 2025).
    Windows(Vec<Window>),
}

impl Action {
    #[cfg(test)]
    fn tag(&self) -> &'static str {
        match self {
            Action::Exit => "exit",
            Action::Command(_) => "command",
            Action::Windows(_) => "windows",
        }
    }
}

/// Input to the viewer.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Data from tmux was received that needs to be processed.
    Tmux(Notification),
}

/// A tmux window.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// Window ID.
    pub id: usize,
    /// Width in cells.
    pub width: usize,
    /// Height in cells.
    pub height: usize,
    /// Parsed layout.
    pub layout: Layout,
}

/// A tmux pane mirrored in a local terminal.
#[derive(Debug)]
pub struct Pane {
    /// The terminal holding the pane contents.
    pub terminal: Terminal,
}

/// Insertion-ordered map of pane ID to pane (Zig `AutoArrayHashMapUnmanaged`).
#[derive(Debug, Default)]
pub struct PanesMap {
    entries: Vec<(usize, Pane)>,
}

impl PanesMap {
    /// Number of panes.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
    /// True if the pane ID exists.
    pub fn contains(&self, id: usize) -> bool {
        self.entries.iter().any(|(k, _)| *k == id)
    }
    /// Get a pane.
    pub fn get(&self, id: usize) -> Option<&Pane> {
        self.entries.iter().find(|(k, _)| *k == id).map(|(_, v)| v)
    }
    /// Get a pane mutably.
    pub fn get_mut(&mut self, id: usize) -> Option<&mut Pane> {
        self.entries
            .iter_mut()
            .find(|(k, _)| *k == id)
            .map(|(_, v)| v)
    }
    /// Pane IDs in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().map(|(k, _)| *k)
    }
    fn take(&mut self, id: usize) -> Option<Pane> {
        let i = self.entries.iter().position(|(k, _)| *k == id)?;
        Some(self.entries.remove(i).1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// We start in this state just after receiving the initial DCS 1000p
    /// opening sequence. We wait for an initial begin/end block that is
    /// guaranteed to be sent by tmux for the initial control mode command.
    StartupBlock,
    /// After receiving the initial block, we wait for a session-changed
    /// notification to record the initial session ID.
    StartupSession,
    /// Tmux has closed the control mode connection.
    Defunct,
    /// We're sitting on the command queue waiting for command output in the
    /// order provided in `command_queue`.
    ///
    /// Precondition: if the queue is non-empty, the first command has
    /// already been sent to tmux (via a `Command` action). The next output
    /// is assumed to be the result of this command. Any transition INTO this
    /// state must send a command action for the first queued command.
    CommandQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CapturePane {
    id: usize,
    screen_key: ScreenKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// List all windows so we can sync our window state.
    ListWindows,
    /// Capture history for the given pane ID.
    PaneHistory(CapturePane),
    /// Capture visible area for the given pane ID.
    PaneVisible(CapturePane),
    /// Capture the pane terminal state as best we can.
    PaneState,
    /// Get the tmux server version.
    TmuxVersion,
    /// User command provided by the user (opaque).
    #[allow(dead_code)]
    User(Vec<u8>),
}

/// Format strings used for commands in our viewer.
struct Format {
    /// The variables included in this format, in order.
    vars: &'static [Variable],
    /// The delimiter between variables. Must be a character guaranteed to
    /// not appear in any variable output.
    delim: char,
}

impl Format {
    const LIST_PANES: Format = Format {
        delim: ';',
        vars: &[
            Variable::PaneId,
            // Cursor position & appearance
            Variable::CursorX,
            Variable::CursorY,
            Variable::CursorFlag,
            Variable::CursorShape,
            Variable::CursorColour,
            Variable::CursorBlinking,
            // Alternate screen
            Variable::AlternateOn,
            Variable::AlternateSavedX,
            Variable::AlternateSavedY,
            // Terminal modes
            Variable::InsertFlag,
            Variable::WrapFlag,
            Variable::KeypadFlag,
            Variable::KeypadCursorFlag,
            Variable::OriginFlag,
            // Mouse modes
            Variable::MouseAllFlag,
            Variable::MouseAnyFlag,
            Variable::MouseButtonFlag,
            Variable::MouseStandardFlag,
            Variable::MouseUtf8Flag,
            Variable::MouseSgrFlag,
            // Focus & special features
            Variable::FocusFlag,
            Variable::BracketedPaste,
            // Scroll region
            Variable::ScrollRegionUpper,
            Variable::ScrollRegionLower,
            // Tab stops
            Variable::PaneTabs,
        ],
    };

    const LIST_WINDOWS: Format = Format {
        delim: ' ',
        vars: &[
            Variable::SessionId,
            Variable::WindowId,
            Variable::WindowWidth,
            Variable::WindowHeight,
            Variable::WindowLayout,
        ],
    };

    const TMUX_VERSION: Format = Format {
        delim: ' ',
        vars: &[Variable::Version],
    };

    fn format_string(&self) -> String {
        output::format(self.vars, self.delim)
    }

    fn parse(&self, s: &str) -> Result<output::FormatValues, output::ParseError> {
        output::parse_format_struct(self.vars, s, self.delim)
    }
}

impl Command {
    /// Format the command into the command that should be executed by
    /// tmux. Trailing newlines are appended so this can be sent as-is.
    fn format_command(&self) -> Vec<u8> {
        let alt = |k: ScreenKey| if k == ScreenKey::Alternate { "-a " } else { "" };
        match self {
            Command::ListWindows => format!(
                "list-windows -F '{}'\n",
                Format::LIST_WINDOWS.format_string()
            )
            .into_bytes(),
            // -p = output to stdout instead of buffer
            // -e = output escape sequences for SGR
            // -a = capture alternate screen (only valid for alternate)
            // -q = quiet, don't error if alternate screen doesn't exist
            // -S - = start at the top of history ("-")
            // -E -1 = end at the last line of history (1 before the
            //   visible area is -1).
            // -t %{d} = target a specific pane ID
            Command::PaneHistory(cap) => format!(
                "capture-pane -p -e -q {}-S - -E -1 -t %{}\n",
                alt(cap.screen_key),
                cap.id
            )
            .into_bytes(),
            // (no -S/-E = capture visible area only)
            Command::PaneVisible(cap) => format!(
                "capture-pane -p -e -q {}-t %{}\n",
                alt(cap.screen_key),
                cap.id
            )
            .into_bytes(),
            Command::PaneState => {
                format!("list-panes -F '{}'\n", Format::LIST_PANES.format_string()).into_bytes()
            }
            Command::TmuxVersion => format!(
                "display-message -p '{}'\n",
                Format::TMUX_VERSION.format_string()
            )
            .into_bytes(),
            Command::User(v) => v.clone(),
        }
    }
}

fn trim_bytes<'a>(s: &'a [u8], set: &[u8]) -> &'a [u8] {
    let mut s = s;
    while let [first, rest @ ..] = s {
        if set.contains(first) {
            s = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if set.contains(last) {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn lossy(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

fn to_cell(v: usize) -> Option<CellCountInt> {
    CellCountInt::try_from(v).ok()
}

/// A tmux control mode viewer. See the module docs.
#[derive(Debug)]
pub struct Viewer {
    state: State,

    /// The current session ID we're attached to.
    pub session_id: usize,

    /// The tmux server version string (e.g., "3.5a"). We capture this on
    /// startup because it will allow us to change behavior between versions
    /// as necessary.
    pub tmux_version: String,

    /// The list of commands we've sent that we want to send and wait for a
    /// response for. We only send one command at a time just to avoid any
    /// possible confusion around ordering.
    command_queue: VecDeque<Command>,

    /// The windows in the current session.
    pub windows: Vec<Window>,

    /// The panes in the current session, mapped by pane ID.
    pub panes: PanesMap,
}

impl Default for Viewer {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewer {
    /// Initialize a new viewer.
    pub fn new() -> Viewer {
        Viewer {
            state: State::StartupBlock,
            // The default value here is meaningless. We don't get started
            // until we receive a session-changed notification which will
            // set this to a real value.
            session_id: 0,
            tmux_version: String::new(),
            command_queue: VecDeque::with_capacity(COMMAND_QUEUE_INITIAL),
            windows: Vec::new(),
            panes: PanesMap::default(),
        }
    }

    /// True if the command queue has no pending commands.
    pub fn command_queue_is_empty(&self) -> bool {
        self.command_queue.is_empty()
    }

    /// Number of queued commands (including the in-flight one).
    pub fn command_queue_len(&self) -> usize {
        self.command_queue.len()
    }

    /// True once tmux has exited or an unrecoverable error occurred.
    pub fn is_defunct(&self) -> bool {
        self.state == State::Defunct
    }

    /// Send in an input event (such as a tmux protocol notification) and
    /// process it. The returned list is a set of actions to take as a result
    /// of the input prior to the next input. This list may be empty.
    ///
    /// This never fails; on error the viewer goes defunct.
    pub fn next(&mut self, input: Input) -> Vec<Action> {
        match input {
            Input::Tmux(n) => self.next_tmux(n),
        }
    }

    fn next_tmux(&mut self, n: Notification) -> Vec<Action> {
        match self.state {
            State::Defunct => {
                log::info!("received notification in defunct state, ignoring");
                Vec::new()
            }
            State::StartupBlock => self.next_startup_block(n),
            State::StartupSession => self.next_startup_session(n),
            State::CommandQueue => self.next_command(n),
        }
    }

    fn next_startup_block(&mut self, n: Notification) -> Vec<Action> {
        debug_assert!(self.state == State::StartupBlock);
        match n {
            // Only sent by the DCS parser when we first get DCS 1000p, it
            // should never reach us here.
            Notification::Enter => unreachable!("enter is handled before the viewer"),

            // Semantically fine to handle without issue.
            Notification::Exit => self.defunct(),

            // Any begin and end (even error) is fine! Now we wait for
            // session-changed to get the initial session ID.
            Notification::BlockEnd(_) | Notification::BlockErr(_) => {
                self.state = State::StartupSession;
                Vec::new()
            }

            _ => Vec::new(),
        }
    }

    fn next_startup_session(&mut self, n: Notification) -> Vec<Action> {
        debug_assert!(self.state == State::StartupSession);
        match n {
            Notification::Enter => unreachable!("enter is handled before the viewer"),
            Notification::Exit => self.defunct(),
            Notification::SessionChanged { id, .. } => {
                self.session_id = id;
                self.enter_command_queue(vec![Command::TmuxVersion, Command::ListWindows])
            }
            _ => Vec::new(),
        }
    }

    fn next_command(&mut self, n: Notification) -> Vec<Action> {
        // We have to be in a command queue, but the command queue MAY be
        // empty. If it is empty, then received_command_output will handle it
        // by ignoring any command output. That's okay!
        debug_assert!(self.state == State::CommandQueue);

        let mut actions: Vec<Action> = Vec::new();

        // Track whether the in-flight command slot is available. Starts true
        // if queue is empty (no command in flight). Set to true when a
        // command completes (block_end/block_err) or the queue is reset
        // (session_changed).
        let mut command_consumed = self.command_queue.is_empty();

        match n {
            Notification::Enter => unreachable!("enter is handled before the viewer"),
            Notification::Exit => return self.defunct(),

            Notification::BlockEnd(content) | Notification::BlockErr(content) => {
                // (is_err is only used for logging in Zig)
                if self
                    .received_command_output(&mut actions, &content)
                    .is_err()
                {
                    log::warn!("failed to process command output, becoming defunct");
                    return self.defunct();
                }
                // Command is consumed since a block end/err is the output
                // from a command.
                command_consumed = true;
            }

            Notification::Output { pane_id, data } => {
                if let Err(err) = self.received_output(pane_id, &data) {
                    log::warn!("failed to process output for pane id={pane_id}: {err}");
                }
            }

            // Session changed means we switched to a different tmux session.
            // We need to reset our state and start fresh with list-windows.
            Notification::SessionChanged { id, .. } => {
                self.session_changed(&mut actions, id);
                // Command is consumed because session_changed resets our
                // entire viewer.
                command_consumed = true;
            }

            // Layout changed of a single window.
            Notification::LayoutChange {
                window_id, layout, ..
            } => {
                if self
                    .layout_changed(&mut actions, window_id, &layout)
                    .is_err()
                {
                    // Note: in the future, we can probably handle a failure
                    // here with a fallback to remove this one window, list
                    // windows again, and try again.
                    log::warn!("failed to handle layout change, becoming defunct");
                    return self.defunct();
                }
            }

            // A window was added to this session.
            Notification::WindowAdd { id } => self.window_add(id),

            // The active pane changed. We handle our own focus.
            Notification::WindowPaneChanged { .. } => {}

            // A session was created or destroyed. If it was our own session
            // we will get an exit notification very soon.
            Notification::SessionsChanged => {}

            // We don't use window names for anything, currently.
            Notification::WindowRenamed { .. } => {}

            // For other clients. For us, we'll get `exit` or
            // `session_changed`, respectively.
            Notification::ClientDetached { .. } | Notification::ClientSessionChanged { .. } => {}

            // TODO(integration): the Rust control parser may expose more
            // notification kinds than the Zig switch handled.
            #[allow(unreachable_patterns)]
            _ => {}
        }

        // After processing commands, we add our next command to execute if
        // we have one. We do this last because command processing may itself
        // queue more commands. We only emit a command if a prior command was
        // consumed (or never existed).
        if self.state == State::CommandQueue && command_consumed {
            if let Some(next_command) = self.command_queue.front() {
                debug_assert!(!actions.iter().any(|a| matches!(a, Action::Command(_))));
                actions.push(Action::Command(next_command.format_command()));
            }
        }

        actions
    }

    /// When the layout changes for a single window, a pane may be added or
    /// removed that we've never seen, in addition to the layout itself
    /// physically changing.
    ///
    /// Similar to list-windows except we expect the window to already
    /// exist. We update the layout, init any new panes, queue commands to
    /// capture them, and prune removed panes.
    fn layout_changed(
        &mut self,
        actions: &mut Vec<Action>,
        window_id: usize,
        layout_str: &[u8],
    ) -> Result<(), Error> {
        // Find the window this layout change is for.
        let Some(window) = self.windows.iter_mut().find(|w| w.id == window_id) else {
            log::info!("layout change for unknown window id={window_id}");
            return Ok(());
        };

        window.layout = match Layout::parse_with_checksum(layout_str) {
            Ok(l) => l,
            Err(e) => {
                log::info!(
                    "failed to parse window layout id={} layout={}",
                    window_id,
                    lossy(layout_str)
                );
                return Err(e.into());
            }
        };

        // Our initial action is to definitely let the caller know that some
        // windows changed.
        actions.push(Action::Windows(self.windows.clone()));

        // Sync up our panes
        self.sync_layouts(None)
    }

    /// When a window is added to the session, we refresh our window list
    /// to get the new window's information.
    fn window_add(&mut self, _window_id: usize) {
        self.queue_commands([Command::ListWindows]);
    }

    /// Sync panes with the layouts of `new_windows` (or of `self.windows`
    /// if `None`), then replace our window list if `new_windows` is given.
    fn sync_layouts(&mut self, new_windows: Option<Vec<Window>>) -> Result<(), Error> {
        // Collect the pane IDs (and sizes) in layout order.
        fn collect(layout: &Layout, out: &mut Vec<(usize, usize, usize)>) {
            match &layout.content {
                Content::Horizontal(c) | Content::Vertical(c) => {
                    for l in c {
                        collect(l, out);
                    }
                }
                Content::Pane(id) => {
                    if !out.iter().any(|(i, _, _)| i == id) {
                        out.push((*id, layout.width, layout.height));
                    }
                }
            }
        }
        let mut wanted: Vec<(usize, usize, usize)> = Vec::new();
        for w in new_windows.as_deref().unwrap_or(&self.windows) {
            collect(&w.layout, &mut wanted);
        }

        // Create the new terminals first so failure leaves us untouched.
        // TODO(zig): We need to gracefully handle overflow of our max
        // cols/width here. In practice we shouldn't hit this.
        let mut created: Vec<(usize, Pane)> = Vec::new();
        for &(id, w, h) in &wanted {
            if self.panes.contains(id) {
                continue;
            }
            let cols = to_cell(w).ok_or(Error::Overflow)?;
            let rows = to_cell(h).ok_or(Error::Overflow)?;
            let terminal = Terminal::new(TerminalOptions::new(cols, rows))?;
            created.push((id, Pane { terminal }));
        }

        // Get our list of added panes and setup our command queue to
        // populate them.
        let mut added = false;
        for (id, _) in &created {
            added = true;
            let id = *id;
            self.queue_commands([
                Command::PaneHistory(CapturePane {
                    id,
                    screen_key: ScreenKey::Primary,
                }),
                Command::PaneVisible(CapturePane {
                    id,
                    screen_key: ScreenKey::Primary,
                }),
                Command::PaneHistory(CapturePane {
                    id,
                    screen_key: ScreenKey::Alternate,
                }),
                Command::PaneVisible(CapturePane {
                    id,
                    screen_key: ScreenKey::Alternate,
                }),
            ]);
        }
        // If we added any panes, then we also want to resync the pane state
        // (terminal modes and cursor positions and so on).
        if added {
            self.queue_commands([Command::PaneState]);
        }

        // No more errors after this point.
        if let Some(w) = new_windows {
            self.windows = w;
        }

        // Replace our panes (old panes not in the layout are dropped).
        let mut old = std::mem::take(&mut self.panes);
        let mut entries = Vec::with_capacity(wanted.len());
        for (id, _, _) in wanted {
            let pane = match old.take(id) {
                Some(p) => p,
                None => {
                    let i = created.iter().position(|(k, _)| *k == id);
                    created.remove(i.expect("created pane")).1
                }
            };
            entries.push((id, pane));
        }
        self.panes = PanesMap { entries };
        Ok(())
    }

    /// When a session changes, we have to basically reset our whole state.
    /// To do this, we emit an empty windows event (so callers can clear all
    /// windows), reset ourself, and start all over.
    fn session_changed(&mut self, actions: &mut Vec<Action>, session_id: usize) {
        debug_assert!(actions.is_empty());

        // Build up a new viewer. It's the easiest way to reset ourselves.
        let mut replacement = Viewer::new();
        actions.push(Action::Windows(Vec::new()));

        // Setup our command queue and put ourselves in the command queue
        // state.
        replacement.queue_commands([Command::ListWindows]);
        replacement.state = State::CommandQueue;

        // Transfer preserved version to replacement
        replacement.tmux_version = self.tmux_version.clone();

        *self = replacement;

        // Set our session ID and jump directly to the list
        self.session_id = session_id;
        debug_assert!(self.state == State::CommandQueue);
    }

    fn received_command_output(
        &mut self,
        actions: &mut Vec<Action>,
        content: &[u8],
    ) -> Result<(), Error> {
        // Get the command we're expecting output for.
        let Some(command) = self.command_queue.pop_front() else {
            // If we have no pending commands, this is unexpected.
            log::info!("unexpected block output");
            return Ok(());
        };

        match command {
            Command::User(_) => Ok(()),
            Command::PaneState => self.received_pane_state(content),
            Command::ListWindows => self.received_list_windows(actions, content),
            Command::PaneHistory(cap) => {
                self.received_pane_history(cap.screen_key, cap.id, content)
            }
            Command::PaneVisible(cap) => {
                self.received_pane_visible(cap.screen_key, cap.id, content)
            }
            Command::TmuxVersion => self.received_tmux_version(content),
        }
    }

    fn received_tmux_version(&mut self, content: &[u8]) -> Result<(), Error> {
        let line = trim_bytes(content, b" \t\r\n");
        if line.is_empty() {
            return Ok(());
        }
        let line_str = String::from_utf8_lossy(line);
        let data = Format::TMUX_VERSION.parse(&line_str).inspect_err(|_| {
            log::info!("failed to parse tmux version: {line_str}");
        })?;
        self.tmux_version = data.str(Variable::Version).to_owned();
        Ok(())
    }

    fn received_list_windows(
        &mut self,
        actions: &mut Vec<Action>,
        content: &[u8],
    ) -> Result<(), Error> {
        // This stores our new window state from this list-windows output.
        let mut windows: Vec<Window> = Vec::new();

        for line_raw in content.split(|&c| c == b'\n') {
            let line = trim_bytes(line_raw, b" \t\r");
            if line.is_empty() {
                continue;
            }
            let line = String::from_utf8_lossy(line);
            let data = Format::LIST_WINDOWS.parse(&line).inspect_err(|_| {
                log::info!("failed to parse list-windows line: {line}");
            })?;

            // Parse the layout
            let layout_str = data.str(Variable::WindowLayout);
            let layout = Layout::parse_with_checksum(layout_str.as_bytes()).inspect_err(|_| {
                log::info!(
                    "failed to parse window layout id={} layout={}",
                    data.usize(Variable::WindowId),
                    layout_str
                );
            })?;

            windows.push(Window {
                id: data.usize(Variable::WindowId),
                width: data.usize(Variable::WindowWidth),
                height: data.usize(Variable::WindowHeight),
                layout,
            });
        }

        // Setup our windows action so the caller can process GUI window
        // changes.
        actions.push(Action::Windows(windows.clone()));

        // Sync up our layouts. This will populate unknown panes, prune, etc.
        // On failure, drop the action we just added (Zig shrinks the list).
        if let Err(e) = self.sync_layouts(Some(windows)) {
            actions.pop();
            return Err(e);
        }
        Ok(())
    }

    fn received_pane_state(&mut self, content: &[u8]) -> Result<(), Error> {
        for line_raw in content.split(|&c| c == b'\n') {
            let line = trim_bytes(line_raw, b" \t\r");
            if line.is_empty() {
                continue;
            }
            let line = String::from_utf8_lossy(line);
            let data = Format::LIST_PANES.parse(&line).inspect_err(|_| {
                log::info!("failed to parse list-panes line: {line}");
            })?;

            // Get the pane for this ID
            let pane_id = data.usize(Variable::PaneId);
            let Some(pane) = self.panes.get_mut(pane_id) else {
                log::info!("received pane state for untracked pane id={pane_id}");
                continue;
            };
            let t = &mut pane.terminal;

            // Determine which screen to use based on alternate_on
            let screen_key = if data.bool(Variable::AlternateOn) {
                ScreenKey::Alternate
            } else {
                ScreenKey::Primary
            };

            // Set cursor position on the appropriate screen (tmux uses 0-based)
            if let Some(screen) = t.screens.get_mut(screen_key) {
                'cursor: {
                    let Some(cursor_x) = to_cell(data.usize(Variable::CursorX)) else {
                        break 'cursor;
                    };
                    let Some(cursor_y) = to_cell(data.usize(Variable::CursorY)) else {
                        break 'cursor;
                    };
                    if cursor_x >= screen.pages.cols || cursor_y >= screen.pages.rows {
                        break 'cursor;
                    }
                    screen.cursor_absolute(cursor_x, cursor_y);
                }

                // Set cursor shape on this screen
                match data.str(Variable::CursorShape) {
                    "block" => screen.cursor.cursor_style = CursorStyle::Block,
                    "underline" => screen.cursor.cursor_style = CursorStyle::Underline,
                    "bar" => screen.cursor.cursor_style = CursorStyle::Bar,
                    // "default" or unknown: leave as-is
                    _ => {}
                }
            }

            // Set alternate screen saved cursor position
            if let Some(alt_screen) = t.screens.get_mut(ScreenKey::Alternate) {
                'cursor: {
                    let Some(alt_x) = to_cell(data.usize(Variable::AlternateSavedX)) else {
                        break 'cursor;
                    };
                    let Some(alt_y) = to_cell(data.usize(Variable::AlternateSavedY)) else {
                        break 'cursor;
                    };
                    // If our coordinates are outside our screen we ignore it.
                    // tmux actually sends MAX_INT for when there isn't a set
                    // cursor position, so this isn't theoretical.
                    if alt_x >= alt_screen.pages.cols || alt_y >= alt_screen.pages.rows {
                        break 'cursor;
                    }
                    alt_screen.cursor_absolute(alt_x, alt_y);
                }
            }

            // Cursor visibility / blinking
            t.modes
                .set(Mode::CursorVisible, data.bool(Variable::CursorFlag));
            t.modes
                .set(Mode::CursorBlinking, data.bool(Variable::CursorBlinking));

            // Terminal modes
            t.modes.set(Mode::Insert, data.bool(Variable::InsertFlag));
            t.modes.set(Mode::Wraparound, data.bool(Variable::WrapFlag));
            t.modes
                .set(Mode::KeypadKeys, data.bool(Variable::KeypadFlag));
            t.modes
                .set(Mode::CursorKeys, data.bool(Variable::KeypadCursorFlag));
            t.modes.set(Mode::Origin, data.bool(Variable::OriginFlag));

            // Mouse modes
            t.modes
                .set(Mode::MouseEventAny, data.bool(Variable::MouseAllFlag));
            t.modes
                .set(Mode::MouseEventButton, data.bool(Variable::MouseAnyFlag));
            t.modes
                .set(Mode::MouseEventNormal, data.bool(Variable::MouseButtonFlag));
            t.modes
                .set(Mode::MouseEventX10, data.bool(Variable::MouseStandardFlag));
            t.modes
                .set(Mode::MouseFormatUtf8, data.bool(Variable::MouseUtf8Flag));
            t.modes
                .set(Mode::MouseFormatSgr, data.bool(Variable::MouseSgrFlag));

            // Focus and bracketed paste
            t.modes
                .set(Mode::FocusEvent, data.bool(Variable::FocusFlag));
            t.modes
                .set(Mode::BracketedPaste, data.bool(Variable::BracketedPaste));

            // Scroll region (tmux uses 0-based values)
            if let (Some(top), Some(bottom)) = (
                to_cell(data.usize(Variable::ScrollRegionUpper)),
                to_cell(data.usize(Variable::ScrollRegionLower)),
            ) {
                t.scrolling_region.top = top;
                t.scrolling_region.bottom = bottom;
            }

            // Tab stops - parse comma-separated list and set
            t.tabstops.reset(0); // Clear all tabstops first
            let tabs = data.str(Variable::PaneTabs);
            if !tabs.is_empty() {
                for tab_str in tabs.split(',') {
                    let Ok(col) = ghostty_zigfmt::parse_int::<usize>(tab_str.as_bytes(), 10) else {
                        continue;
                    };
                    let Some(col_cell) = to_cell(col) else {
                        continue;
                    };
                    if col_cell >= t.cols {
                        continue;
                    }
                    t.tabstops.set(usize::from(col_cell));
                }
            }
        }
        Ok(())
    }

    fn received_pane_history(
        &mut self,
        screen_key: ScreenKey,
        id: usize,
        content: &[u8],
    ) -> Result<(), Error> {
        let Some(pane) = self.panes.get_mut(id) else {
            log::info!("received pane history for untracked pane id={id}");
            return Ok(());
        };
        let t = &mut pane.terminal;
        t.switch_screen(screen_key)?;

        // Send the data as-is into a VT stream. This will populate the
        // active area too so it won't be exactly correct but we'll get the
        // active contents soon.
        {
            let mut stream = t.vt_stream();
            stream.next_slice(content).map_err(|e| {
                log::info!("failed to process pane history for pane id={id}: {e:?}");
                Error::Stream(format!("{e:?}"))
            })?;
        }

        // Populate the active area to be empty since this is only history.
        // We'll fill it with blanks and move the cursor to the top-left.
        t.carriage_return();
        for _ in 0..t.rows {
            t.index()?;
        }
        t.set_cursor_pos(1, 1);

        // Our active area should be empty
        if cfg!(debug_assertions) {
            let s = t.screens.active();
            let dump = s.dump_string_alloc(Point::Active(Coordinate::default()))?;
            debug_assert!(dump.is_empty());
        }
        Ok(())
    }

    fn received_pane_visible(
        &mut self,
        screen_key: ScreenKey,
        id: usize,
        content: &[u8],
    ) -> Result<(), Error> {
        let Some(pane) = self.panes.get_mut(id) else {
            log::info!("received pane visible for untracked pane id={id}");
            return Ok(());
        };
        let t = &mut pane.terminal;
        t.switch_screen(screen_key)?;

        // Erase the active area and reset the cursor to the top-left before
        // writing the visible content.
        t.erase_display(ghostty_vt::csi::EraseDisplay::Complete, false);
        t.set_cursor_pos(1, 1);

        let mut stream = t.vt_stream();
        stream.next_slice(content).map_err(|e| {
            log::info!("failed to process pane visible for pane id={id}: {e:?}");
            Error::Stream(format!("{e:?}"))
        })
    }

    fn received_output(&mut self, id: usize, data: &[u8]) -> Result<(), Error> {
        let Some(pane) = self.panes.get_mut(id) else {
            log::info!("received output for untracked pane id={id}");
            return Ok(());
        };
        let mut stream = pane.terminal.vt_stream();
        stream.next_slice(data).map_err(|e| {
            log::info!("failed to process output for pane id={id}: {e:?}");
            Error::Stream(format!("{e:?}"))
        })
    }

    /// Enters the command queue state from any other state, queueing the
    /// commands and returning an action to execute the first command.
    fn enter_command_queue(&mut self, commands: Vec<Command>) -> Vec<Action> {
        debug_assert!(self.state != State::CommandQueue);
        debug_assert!(!commands.is_empty());

        let action = Action::Command(commands[0].format_command());
        self.command_queue.extend(commands);
        self.state = State::CommandQueue;
        vec![action]
    }

    /// Queue multiple commands to execute. This doesn't return actions
    /// because the command_queue state will automatically send the next
    /// command when it receives output.
    fn queue_commands(&mut self, commands: impl IntoIterator<Item = Command>) {
        self.command_queue.extend(commands);
    }

    fn defunct(&mut self) -> Vec<Action> {
        self.state = State::Defunct;
        vec![Action::Exit]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(n: Notification) -> Input {
        Input::Tmux(n)
    }
    fn block_end(s: &str) -> Input {
        n(Notification::BlockEnd(s.as_bytes().to_vec()))
    }
    fn session_changed(id: usize, name: &str) -> Input {
        n(Notification::SessionChanged {
            id,
            name: name.as_bytes().to_vec(),
        })
    }
    fn layout_change_in(window_id: usize, layout: &str) -> Input {
        n(Notification::LayoutChange {
            window_id,
            layout: layout.as_bytes().to_vec(),
            visible_layout: layout.as_bytes().to_vec(),
            raw_flags: b"*".to_vec(),
        })
    }

    /// Run a step, check commands end in newline (common mistake).
    fn run(v: &mut Viewer, input: Input) -> Vec<Action> {
        let actions = v.next(input);
        for a in &actions {
            if let Action::Command(c) = a {
                assert!(c.ends_with(b"\n"));
            }
        }
        actions
    }

    fn has_tag(actions: &[Action], tag: &str) -> bool {
        actions.iter().any(|a| a.tag() == tag)
    }

    fn commands(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Command(c) => Some(String::from_utf8_lossy(c).into_owned()),
                _ => None,
            })
            .collect()
    }

    fn contains_command(actions: &[Action], prefix: &str) {
        assert!(
            commands(actions).iter().any(|c| c.starts_with(prefix)),
            "no command starting with {prefix:?} in {actions:?}"
        );
    }

    /// Asserts exactly one command that targets `-t %id` and `-a` or not.
    fn check_capture(actions: &[Action], id: usize, alt: bool) {
        contains_command(actions, "capture-pane");
        let cmds = commands(actions);
        assert!(!cmds.is_empty());
        for c in cmds {
            assert!(c.contains(&format!("-t %{id}")), "{c}");
            assert_eq!(c.contains("-a"), alt, "{c}");
        }
    }

    fn dump(v: &Viewer, pane: usize, pt: Point) -> String {
        v.panes
            .get(pane)
            .unwrap()
            .terminal
            .screens
            .active()
            .dump_string_alloc(pt)
            .unwrap()
    }
    fn history() -> Point {
        Point::History(Coordinate::default())
    }
    fn active() -> Point {
        Point::Active(Coordinate::default())
    }

    /// Common startup through the first list-windows output.
    fn startup(v: &mut Viewer, session: usize, name: &str, window_line: &str) -> Vec<Action> {
        run(v, block_end(""));
        let a = run(v, session_changed(session, name));
        contains_command(&a, "display-message");
        let a = run(v, block_end("3.5a"));
        contains_command(&a, "list-windows");
        run(v, block_end(window_line))
    }

    #[test]
    fn immediate_exit() {
        let mut v = Viewer::new();
        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
        let a = run(&mut v, n(Notification::Exit));
        assert_eq!(a.len(), 0);
    }

    #[test]
    fn session_changed_resets_state() {
        let mut v = Viewer::new();
        run(&mut v, block_end(""));
        let a = run(&mut v, session_changed(1, "first"));
        contains_command(&a, "display-message");
        let a = run(&mut v, block_end("3.5a"));
        contains_command(&a, "list-windows");
        let a = run(
            &mut v,
            block_end("$1 @0 83 44 027b,83x44,0,0[83x20,0,0,0,83x23,0,21,1]"),
        );
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert_eq!(v.session_id, 1);
        assert_eq!(v.windows.len(), 1);
        assert_eq!(v.panes.count(), 2);
        assert_eq!(v.tmux_version, "3.5a");

        // Now session changes - should reset everything but keep version
        let a = run(&mut v, session_changed(2, "second"));
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        contains_command(&a, "list-windows");
        assert_eq!(v.session_id, 2);
        assert!(
            a.iter()
                .any(|x| matches!(x, Action::Windows(w) if w.is_empty()))
        );
        assert_eq!(v.windows.len(), 0);
        assert_eq!(v.panes.count(), 0);
        assert_eq!(v.tmux_version, "3.5a");

        // Same pane IDs 0,1 - they should be re-created since old panes were cleared
        let a = run(
            &mut v,
            block_end("$2 @1 83 44 027b,83x44,0,0[83x20,0,0,0,83x23,0,21,1]"),
        );
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert_eq!(v.session_id, 2);
        assert_eq!(v.windows.len(), 1);
        assert_eq!(v.windows[0].id, 1);
        assert_eq!(v.panes.count(), 2);
        assert_eq!(v.tmux_version, "3.5a");
    }

    #[test]
    fn initial_flow() {
        let mut v = Viewer::new();
        run(&mut v, block_end(""));
        let a = run(&mut v, session_changed(1, "test"));
        contains_command(&a, "display-message");
        let a = run(&mut v, block_end("3.5a"));
        contains_command(&a, "list-windows");
        let a = run(
            &mut v,
            block_end("$0 @0 83 44 027b,83x44,0,0[83x20,0,0,0,83x23,0,21,1]"),
        );
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        check_capture(&a, 0, false);

        let a = run(&mut v, block_end("Hello, world!"));
        check_capture(&a, 0, false);
        assert_eq!(dump(&v, 0, history()), "Hello, world!");
        assert_eq!(dump(&v, 0, active()), "");

        let a = run(&mut v, block_end(""));
        check_capture(&a, 0, true);
        let a = run(&mut v, block_end(""));
        check_capture(&a, 0, true);
        let a = run(&mut v, block_end(""));
        check_capture(&a, 1, false);
        let a = run(&mut v, block_end(""));
        check_capture(&a, 1, false);
        let a = run(&mut v, block_end(""));
        check_capture(&a, 1, true);
        let a = run(&mut v, block_end(""));
        check_capture(&a, 1, true);
        run(&mut v, block_end(""));

        let a = run(
            &mut v,
            n(Notification::Output {
                pane_id: 0,
                data: b"new output".to_vec(),
            }),
        );
        assert_eq!(a.len(), 0);
        assert!(dump(&v, 0, active()).contains("new output"));

        let a = run(
            &mut v,
            n(Notification::Output {
                pane_id: 999,
                data: b"ignored".to_vec(),
            }),
        );
        assert_eq!(a.len(), 0);

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn layout_change() {
        let mut v = Viewer::new();
        let a = startup(&mut v, 1, "test", "$0 @0 83 44 b7dd,83x44,0,0,0");
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert_eq!(v.windows.len(), 1);
        assert_eq!(v.panes.count(), 1);
        assert!(v.panes.contains(0));

        // Complete all capture-pane commands for pane 0 plus pane_state
        for _ in 0..5 {
            run(&mut v, block_end(""));
        }

        // Now send a layout_change that splits into two panes
        let a = run(
            &mut v,
            layout_change_in(0, "e07b,83x44,0,0[83x22,0,0,0,83x21,0,23,2]"),
        );
        assert!(has_tag(&a, "windows"));
        assert_eq!(v.windows.len(), 1);
        assert_eq!(v.panes.count(), 2);
        assert!(v.panes.contains(0));
        assert!(v.panes.contains(2));
        // Commands queued for the new pane (4 capture-pane + 1 pane_state)
        assert_eq!(v.command_queue_len(), 5);

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn layout_change_does_not_return_command_when_queue_not_empty() {
        let mut v = Viewer::new();
        let a = startup(&mut v, 1, "test", "$0 @0 83 44 b7dd,83x44,0,0,0");
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert!(!v.command_queue_is_empty());

        // Do NOT complete capture-pane commands - queue still has commands.
        let a = run(
            &mut v,
            layout_change_in(0, "e07b,83x44,0,0[83x22,0,0,0,83x21,0,23,2]"),
        );
        assert!(has_tag(&a, "windows"));
        assert_eq!(v.panes.count(), 2);
        assert!(!has_tag(&a, "command"));

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn layout_change_returns_command_when_queue_was_empty() {
        let mut v = Viewer::new();
        let a = startup(&mut v, 1, "test", "$0 @0 83 44 b7dd,83x44,0,0,0");
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));

        // Complete all capture-pane commands for pane 0
        for _ in 0..4 {
            run(&mut v, block_end(""));
        }
        // Queue should now be empty
        run(&mut v, block_end(""));
        assert!(v.command_queue_is_empty());

        let a = run(
            &mut v,
            layout_change_in(0, "e07b,83x44,0,0[83x22,0,0,0,83x21,0,23,2]"),
        );
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert_eq!(v.panes.count(), 2);
        assert!(!v.command_queue_is_empty());

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn window_add_queues_list_windows_when_queue_empty() {
        let mut v = Viewer::new();
        let a = startup(&mut v, 1, "test", "$0 @0 83 44 b7dd,83x44,0,0,0");
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        for _ in 0..4 {
            run(&mut v, block_end(""));
        }
        run(&mut v, block_end(""));
        assert!(v.command_queue_is_empty());

        let a = run(&mut v, n(Notification::WindowAdd { id: 1 }));
        contains_command(&a, "list-windows");
        assert!(!v.command_queue_is_empty());
        assert_eq!(v.command_queue_len(), 1);

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn window_add_queues_list_windows_when_queue_not_empty() {
        let mut v = Viewer::new();
        let a = startup(&mut v, 1, "test", "$0 @0 83 44 b7dd,83x44,0,0,0");
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert!(!v.command_queue_is_empty());

        let a = run(&mut v, n(Notification::WindowAdd { id: 1 }));
        assert!(!has_tag(&a, "command"));
        assert!(!v.command_queue_is_empty());

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }

    #[test]
    fn two_pane_flow_with_pane_state() {
        let mut v = Viewer::new();
        run(&mut v, block_end(""));
        let a = run(&mut v, session_changed(0, "0"));
        contains_command(&a, "display-message");
        assert_eq!(v.session_id, 0);
        let a = run(&mut v, block_end("3.5a"));
        contains_command(&a, "list-windows");

        let a = run(
            &mut v,
            block_end("$0 @0 165 79 ca97,165x79,0,0[165x40,0,0,0,165x38,0,41,4]"),
        );
        assert!(has_tag(&a, "windows") && has_tag(&a, "command"));
        assert_eq!(v.windows.len(), 1);
        let w = &v.windows[0];
        assert_eq!((w.id, w.width, w.height), (0, 165, 79));
        assert_eq!(v.panes.count(), 2);
        assert!(v.panes.contains(0));
        assert!(v.panes.contains(4));

        // pane 0 primary history / visible
        run(&mut v, block_end("prompt %\nprompt %"));
        run(&mut v, block_end("prompt %"));
        let h = dump(&v, 0, history());
        assert!(h.matches("prompt %").count() >= 2, "{h:?}");
        assert_eq!(dump(&v, 0, active()), "prompt %");

        // pane 0 alternate history/visible (empty)
        run(&mut v, block_end(""));
        run(&mut v, block_end(""));
        // pane 4 primary history / visible
        run(&mut v, block_end("prompt %"));
        run(&mut v, block_end("prompt %"));
        assert_eq!(dump(&v, 4, history()), "prompt %");
        assert!(dump(&v, 4, active()).starts_with("prompt %"));
        // pane 4 alternate (empty)
        run(&mut v, block_end(""));
        run(&mut v, block_end(""));

        // list-panes output with terminal state
        let tabs = "8,16,24,32,40,48,56,64,72,80,88,96,104,112,120,128,136,144,152,160";
        let state = format!(
            "%0;42;0;1;;;;0;4294967295;4294967295;0;1;0;0;0;0;0;0;0;0;0;;;0;39;{tabs}\n\
             %4;10;5;1;;;;0;4294967295;4294967295;0;1;0;0;0;0;0;0;0;0;0;;;0;37;{tabs}"
        );
        run(&mut v, block_end(&state));

        for (id, x, y) in [(0usize, 42u16, 0u16), (4, 10, 5)] {
            let t = &v.panes.get(id).unwrap().terminal;
            let screen = t.screens.get(ScreenKey::Primary).unwrap();
            assert_eq!(screen.cursor.x, x);
            assert_eq!(screen.cursor.y, y);
            assert!(t.modes.get(Mode::CursorVisible));
            assert!(t.modes.get(Mode::Wraparound));
            assert!(!t.modes.get(Mode::Insert));
            assert!(!t.modes.get(Mode::Origin));
            assert!(!t.modes.get(Mode::KeypadKeys));
            assert!(!t.modes.get(Mode::CursorKeys));
        }

        let a = run(&mut v, n(Notification::Exit));
        assert!(has_tag(&a, "exit"));
    }
}
