//! VT event stream inspector (ZIG/src/inspector/widgets/termio.zig).

use std::{collections::VecDeque, convert::Infallible, fmt::Write as _};

use dear_imgui_rs::{
    FocusedFlags,
    Key,
    SelectableFlags,
    TableColumnFlags,
    TableColumnWidth,
    TableFlags,
    Ui,
    WindowFlags,
};
use ghostty_terminal::terminal::{ScrollingRegion, Terminal};
use ghostty_vt::{
    color::Palette,
    osc::{self, Command as OscCommand},
    parser::Action as ParserAction,
    stream::{Action as StreamAction, Handler, Stream as VtStream},
};

use super::screen::{CursorSnapshot, cursor_style, cursor_table};

/// The maximum number of recorded events (see the key event ring).
pub const MAX_EVENTS: usize = 100;

/// The tags of `Parser.Action` (used for filtering).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTag {
    Print,
    Execute,
    CsiDispatch,
    EscDispatch,
    OscDispatch,
    DcsHook,
    DcsPut,
    DcsUnhook,
    ApcStart,
    ApcPut,
    ApcEnd,
}

impl ActionTag {
    pub const ALL: [ActionTag; 11] = [
        ActionTag::Print,
        ActionTag::Execute,
        ActionTag::CsiDispatch,
        ActionTag::EscDispatch,
        ActionTag::OscDispatch,
        ActionTag::DcsHook,
        ActionTag::DcsPut,
        ActionTag::DcsUnhook,
        ActionTag::ApcStart,
        ActionTag::ApcPut,
        ActionTag::ApcEnd,
    ];

    pub fn of(action: &ParserAction) -> ActionTag {
        match action {
            ParserAction::Print(_) => ActionTag::Print,
            ParserAction::Execute(_) => ActionTag::Execute,
            ParserAction::CsiDispatch(_) => ActionTag::CsiDispatch,
            ParserAction::EscDispatch(_) => ActionTag::EscDispatch,
            ParserAction::OscDispatch(_) => ActionTag::OscDispatch,
            ParserAction::DcsHook(_) => ActionTag::DcsHook,
            ParserAction::DcsPut(_) => ActionTag::DcsPut,
            ParserAction::DcsUnhook => ActionTag::DcsUnhook,
            ParserAction::ApcStart => ActionTag::ApcStart,
            ParserAction::ApcPut(_) => ActionTag::ApcPut,
            ParserAction::ApcEnd => ActionTag::ApcEnd,
        }
    }

    pub fn name(self) -> String {
        crate::keys::tag_name(&self)
    }
}

/// A simple set of excluded action tags (Zig: `std.EnumSet`).
#[derive(Debug, Clone, Default)]
pub struct ActionTagSet(Vec<ActionTag>);

impl ActionTagSet {
    pub fn contains(&self, tag: ActionTag) -> bool {
        self.0.contains(&tag)
    }

    pub fn insert(&mut self, tag: ActionTag) {
        if !self.contains(tag) {
            self.0.push(tag);
        }
    }

    pub fn remove(&mut self, tag: ActionTag) {
        self.0.retain(|t| *t != tag);
    }
}

/// Port of `ImGuiTextFilter` (build + pass filter): comma separated terms,
/// a term starting with `-` excludes, matching is ASCII case insensitive.
#[derive(Debug, Clone, Default)]
pub struct TextFilter {
    pub input: String,
    /// `(exclude, lowercase term)` in order.
    filters: Vec<(bool, String)>,
    count_grep: usize,
}

impl TextFilter {
    /// Re-parse the filter expression (`ImGuiTextFilter::Build`).
    pub fn build(&mut self) {
        self.filters.clear();
        self.count_grep = 0;
        for part in self.input.split(',') {
            let part = part.trim_matches(|c| c == ' ' || c == '\t');
            if part.is_empty() {
                continue;
            }
            self.filters
                .push((part.starts_with('-'), part.to_ascii_lowercase()));
            // `-` alone is a no-op term, like in ImGui.
            if !part.starts_with('-') {
                self.count_grep += 1;
            }
        }
    }

    /// `ImGuiTextFilter::PassFilter`.
    pub fn pass_filter(&self, text: &str) -> bool {
        if self.filters.is_empty() {
            return true;
        }

        let lower = text.to_ascii_lowercase();
        for (exclude, term) in &self.filters {
            if *exclude {
                // Subtract hyphen
                let needle = &term[1..];
                if !needle.is_empty() && lower.contains(needle) {
                    return false;
                }
            } else if lower.contains(term.as_str()) {
                return true;
            }
        }

        // Implicit * grep
        self.count_grep == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Print,
    Execute,
    Csi,
    Esc,
    Osc,
    Dcs,
    Apc,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Print => "print",
            Kind::Execute => "execute",
            Kind::Csi => "csi",
            Kind::Esc => "esc",
            Kind::Osc => "osc",
            Kind::Dcs => "dcs",
            Kind::Apc => "apc",
        }
    }
}

/// VT event. This is just how we store internal events.
#[derive(Debug, Clone)]
pub struct VtEvent {
    /// Sequence number, just monotonically increasing and wrapping if
    /// it ever overflows. It gives us a nice way to visualize progress.
    pub seq: usize,

    /// Kind of event, for filtering
    pub kind: Kind,

    /// The description of the raw event in a more human-friendly format.
    /// For example for control sequences this is the full sequence but
    /// control characters are replaced with human-readable names, e.g.
    /// 0x07 (bell) becomes BEL.
    pub raw_description: String,

    /// Various metadata at the time of the event (before processing).
    pub cursor: CursorSnapshot,
    pub scrolling_region: ScrollingRegion,
    pub metadata: Vec<(String, String)>,

    /// imgui selection state
    pub imgui_selected: bool,
}

impl VtEvent {
    /// Initialize the event information for the given parser action.
    pub fn new(
        cursor: CursorSnapshot,
        scrolling_region: ScrollingRegion,
        action: &ParserAction,
    ) -> Self {
        let mut md = Vec::new();
        let desc = encode_action(&mut md, action);

        let kind = match action {
            ParserAction::Print(_) => Kind::Print,
            ParserAction::Execute(_) => Kind::Execute,
            ParserAction::CsiDispatch(_) => Kind::Csi,
            ParserAction::EscDispatch(_) => Kind::Esc,
            ParserAction::OscDispatch(_) => Kind::Osc,
            ParserAction::DcsHook(_) | ParserAction::DcsPut(_) | ParserAction::DcsUnhook => {
                Kind::Dcs
            }
            ParserAction::ApcStart | ParserAction::ApcPut(_) | ParserAction::ApcEnd => Kind::Apc,
        };

        VtEvent {
            seq: 1,
            kind,
            raw_description: desc,
            cursor,
            scrolling_region,
            metadata: md,
            imgui_selected: false,
        }
    }

    /// Returns true if the event passes the given filter.
    pub fn pass_filter(&self, filter: &TextFilter) -> bool {
        // Check our main string
        if filter.pass_filter(&self.raw_description) {
            return true;
        }

        // We also check all metadata keys and values
        self.metadata
            .iter()
            .any(|(k, v)| filter.pass_filter(k) || filter.pass_filter(v))
    }
}

/// Encode a parser action as a string that we show in the logs.
fn encode_action(md: &mut Vec<(String, String)>, action: &ParserAction) -> String {
    match action {
        ParserAction::Print(ch) => encode_print(*ch),
        ParserAction::Execute(ch) => encode_execute(*ch),
        ParserAction::CsiDispatch(v) => encode_csi(v),
        ParserAction::EscDispatch(v) => encode_esc(*v),
        ParserAction::OscDispatch(v) => encode_osc(md, v),
        other => format_action(other),
    }
}

/// Mirrors `Parser.Action.format` for the actions that are not specially
/// encoded.
fn format_action(action: &ParserAction) -> String {
    let (tag, value) = match action {
        ParserAction::DcsHook(d) => (
            "dcs_hook",
            format!(
                "{{ intermediates = {:?}, params = {:?}, final = {} }}",
                d.intermediates(),
                d.params(),
                d.final_
            ),
        ),
        ParserAction::DcsPut(b) => ("dcs_put", format!("0x{b:x}")),
        ParserAction::DcsUnhook => ("dcs_unhook", "void".to_owned()),
        ParserAction::ApcStart => ("apc_start", "void".to_owned()),
        ParserAction::ApcPut(b) => ("apc_put", format!("0x{b:x}")),
        ParserAction::ApcEnd => ("apc_end", "void".to_owned()),
        // Handled by the specific encoders.
        _ => ("unknown", String::new()),
    };
    format!("parser.Parser.Action{{ .{tag} = {value} }}")
}

fn encode_print(ch: u32) -> String {
    let c = char::from_u32(ch).unwrap_or(char::REPLACEMENT_CHARACTER);
    format!("'{c}' (U+{ch:X})")
}

fn encode_execute(ch: u8) -> String {
    let name = match ch {
        0x00 => "NUL",
        0x01 => "SOH",
        0x02 => "STX",
        0x03 => "ETX",
        0x04 => "EOT",
        0x05 => "ENQ",
        0x06 => "ACK",
        0x07 => "BEL",
        0x08 => "BS",
        0x09 => "HT",
        0x0A => "LF",
        0x0B => "VT",
        0x0C => "FF",
        0x0D => "CR",
        0x0E => "SO",
        0x0F => "SI",
        _ => "?",
    };
    format!("{name} (0x{ch:X})")
}

fn encode_csi(csi: &ghostty_vt::parser::Csi) -> String {
    let mut out = String::new();
    for v in csi.intermediates() {
        let _ = write!(out, "{} ", char::from(*v));
    }
    for (i, v) in csi.params().iter().enumerate() {
        if i != 0 {
            out.push(';');
        }
        let _ = write!(out, "{v}");
    }
    if !csi.intermediates().is_empty() || !csi.params().is_empty() {
        out.push(' ');
    }
    out.push(char::from(csi.final_));
    out
}

fn encode_esc(esc: ghostty_vt::parser::Esc) -> String {
    let mut out = String::new();
    for v in esc.intermediates() {
        let _ = write!(out, "{} ", char::from(*v));
    }
    out.push(char::from(esc.final_));
    out
}

fn bytes(v: &[u8]) -> String {
    String::from_utf8_lossy(v).into_owned()
}

fn terminator_name(t: osc::Terminator) -> &'static str {
    match t {
        osc::Terminator::St => "st",
        osc::Terminator::Bel => "bel",
    }
}

fn optional_bool(v: Option<bool>) -> String {
    match v {
        Some(true) => "true".to_owned(),
        Some(false) => "false".to_owned(),
        None => "(unset)".to_owned(),
    }
}

fn encode_osc(md: &mut Vec<(String, String)>, cmd: &OscCommand) -> String {
    // The description is just the tag
    let out = format!("{} ", crate::keys::tag_name(&cmd.key()));
    encode_osc_metadata(md, cmd);
    out
}

/// Add the OSC fields to the metadata (Zig: `encodeMetadata`, which walks
/// the fields of the command union with comptime reflection).
fn encode_osc_metadata(md: &mut Vec<(String, String)>, cmd: &OscCommand) {
    let mut put = |k: &str, v: String| md.push((k.to_owned(), v));
    match cmd {
        OscCommand::Invalid | OscCommand::HyperlinkEnd | OscCommand::ConemuWaitInput => {}
        OscCommand::ChangeWindowTitle(v)
        | OscCommand::ChangeWindowIcon(v)
        | OscCommand::ConemuShowMessageBox(v)
        | OscCommand::ConemuGuimacro(v)
        | OscCommand::ConemuRunProcess(v)
        | OscCommand::ConemuOutputEnvironmentVariable(v)
        | OscCommand::ConemuComment(v) => put("data", bytes(v)),
        OscCommand::SemanticPrompt(v) => {
            put("action", crate::keys::tag_name(&v.action));
            put("options_unvalidated", bytes(&v.options_unvalidated));
        }
        OscCommand::ClipboardContents { kind, data } => {
            put("kind", kind.to_string());
            put("data", bytes(data));
        }
        OscCommand::ReportPwd { value } | OscCommand::MouseShape { value } => {
            put("value", bytes(value));
        }
        OscCommand::ColorOperation {
            op,
            requests: _,
            terminator,
        } => {
            // `Operation` tags are named `osc_4`, `osc_104`, ...
            let dbg = format!("{op:?}");
            let name = dbg
                .strip_prefix("Osc")
                .map_or_else(|| crate::keys::snake_case(&dbg), |n| format!("osc_{n}"));
            put("op", name);
            put(
                "requests",
                "array_list.Aligned(terminal.osc.parsers.color.Request,null)".to_owned(),
            );
            put("terminator", terminator_name(*terminator).to_owned());
        }
        OscCommand::KittyColorProtocol(v) => {
            put(
                "list",
                "array_list.Aligned(terminal.kitty.color.OSC.Request,null)".to_owned(),
            );
            put("terminator", terminator_name(v.terminator).to_owned());
        }
        OscCommand::ShowDesktopNotification { title, body } => {
            put("title", bytes(title));
            put("body", bytes(body));
        }
        OscCommand::HyperlinkStart { id, uri } => {
            put(
                "id",
                id.as_deref().map_or_else(|| "(unset)".to_owned(), bytes),
            );
            put("uri", bytes(uri));
        }
        OscCommand::ConemuSleep { duration_ms } => put("duration_ms", duration_ms.to_string()),
        OscCommand::ConemuChangeTabTitle(v) => match v {
            osc::ConemuChangeTabTitle::Reset => put("data", "reset".to_owned()),
            osc::ConemuChangeTabTitle::Value(s) => put("value", bytes(s)),
        },
        OscCommand::ConemuProgressReport(v) => {
            put("state", crate::keys::tag_name(&v.state));
            put(
                "progress",
                v.progress
                    .map_or_else(|| "(unset)".to_owned(), |p| p.to_string()),
            );
        }
        OscCommand::ConemuXtermEmulation { keyboard, output } => {
            put("keyboard", optional_bool(*keyboard));
            put("output", optional_bool(*output));
        }
        OscCommand::KittyTextSizing(v) => {
            put("scale", v.scale.to_string());
            put("width", v.width.to_string());
            put("numerator", v.numerator.to_string());
            put("denominator", v.denominator.to_string());
            put("valign", crate::keys::tag_name(&v.valign));
            put("halign", crate::keys::tag_name(&v.halign));
            put("text", bytes(&v.text));
        }
        OscCommand::KittyClipboardProtocol(v) => {
            put("metadata", bytes(&v.metadata));
            put(
                "payload",
                v.payload
                    .as_deref()
                    .map_or_else(|| "(unset)".to_owned(), bytes),
            );
            put("terminator", terminator_name(v.terminator).to_owned());
        }
        OscCommand::ContextSignal(v) => {
            put("action", crate::keys::tag_name(&v.action));
            put("id", bytes(&v.id));
            put("metadata", bytes(&v.metadata));
        }
    }
}

/// The capture state of the VT handler: the terminal state at the time of
/// the events.
#[derive(Debug, Clone)]
struct CaptureState {
    cursor: CursorSnapshot,
    scrolling_region: ScrollingRegion,
}

/// Our VT stream handler for the Stream widget.
#[derive(Debug)]
pub struct VtHandler {
    /// The capture state, must be set before use. If `None`, then
    /// events are dropped.
    state: Option<CaptureState>,

    /// The event ring to write events to.
    pub events: VecDeque<VtEvent>,

    /// True to pause this artificially.
    pub paused: bool,

    /// Current sequence number
    pub current_seq: usize,

    /// Exclude certain actions by tag.
    pub filter_exclude: ActionTagSet,
    pub filter_text: TextFilter,
}

impl VtHandler {
    fn new() -> Self {
        let mut filter_exclude = ActionTagSet::default();
        filter_exclude.insert(ActionTag::Print);
        Self {
            state: None,
            events: VecDeque::with_capacity(MAX_EVENTS),
            paused: false,
            current_seq: 1,
            filter_exclude,
            filter_text: TextFilter::default(),
        }
    }
}

impl Handler for VtHandler {
    type Error = Infallible;

    fn vt(&mut self, _action: StreamAction<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// This is called with every single terminal action.
    fn vt_raw(&mut self, action: &ParserAction) -> Result<bool, Self::Error> {
        let Some(state) = self.state.clone() else {
            return Ok(true);
        };

        // We always increment the sequence number, even if we're paused or
        // filter out the event. This helps show the user that there is a gap
        // between events and roughly how large that gap was.
        let seq = self.current_seq;
        self.current_seq = self.current_seq.wrapping_add(1);

        // If we're manually paused, we ignore all events.
        if self.paused {
            return Ok(true);
        }

        // We ignore certain action types that are too noisy.
        if matches!(action, ParserAction::DcsPut(_) | ParserAction::ApcPut(_)) {
            return Ok(true);
        }

        // If we requested a specific type to be ignored, ignore it.
        // We return true because we did "handle" it by ignoring it.
        if self.filter_exclude.contains(ActionTag::of(action)) {
            return Ok(true);
        }

        // Build our event
        let mut ev = VtEvent::new(state.cursor, state.scrolling_region, action);
        ev.seq = seq;

        // Check if the event passes the filter
        if !ev.pass_filter(&self.filter_text) {
            return Ok(true);
        }

        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(ev);

        // Do NOT skip it, because we want to record more information
        // about this event.
        Ok(false)
    }
}

/// Enum representing keyboard navigation actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Down,
    None,
    Up,
}

/// Helper function to check keyboard state and determine navigation action.
fn get_key_action(ui: &Ui) -> KeyAction {
    let keys = [
        (Key::J, KeyAction::Down),
        (Key::DownArrow, KeyAction::Down),
        (Key::K, KeyAction::Up),
        (Key::UpArrow, KeyAction::Up),
    ];

    for (key, action) in keys {
        if ui.is_key_pressed(key) {
            return action;
        }
    }
    KeyAction::None
}

/// VT event stream inspector widget.
pub struct Stream {
    pub parser_stream: VtStream<VtHandler>,

    /// The currently selected event sequence number for keyboard navigation
    pub selected_event_seq: Option<u32>,

    /// Flag indicating whether we need to scroll to the selected item
    pub need_scroll_to_selected: bool,

    /// Flag indicating whether the selection was made by keyboard
    pub is_keyboard_selection: bool,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("events", &self.parser_stream.handler.events.len())
            .field("selected_event_seq", &self.selected_event_seq)
            .finish_non_exhaustive()
    }
}

impl Default for Stream {
    fn default() -> Self {
        Self::new()
    }
}

impl Stream {
    pub fn new() -> Self {
        Self {
            parser_stream: VtStream::new_alloc(VtHandler::new()),
            selected_event_seq: None,
            need_scroll_to_selected: false,
            is_keyboard_selection: false,
        }
    }

    pub fn events(&self) -> &VecDeque<VtEvent> {
        &self.parser_stream.handler.events
    }

    pub fn record_pty_read(&mut self, t: &Terminal, data: &[u8]) {
        self.parser_stream.handler.state = Some(CaptureState {
            cursor: CursorSnapshot::new(&t.screens.active().cursor),
            scrolling_region: t.scrolling_region,
        });
        let res = self.parser_stream.next_slice(data);
        self.parser_stream.handler.state = None;
        if let Err(err) = res {
            log::warn!("error recording pty read in inspector err={err:?}");
        }
    }

    pub fn draw(&mut self, ui: &Ui, palette: &Palette) {
        let popup_filter = "Filter";
        let handler = &mut self.parser_stream.handler;

        // Controls
        {
            let pause_play = if handler.paused {
                "Resume##pause_play"
            } else {
                "Pause##pause_play"
            };
            if ui.button(pause_play) {
                handler.paused = !handler.paused;
            }

            let inner = ui.clone_style().item_inner_spacing()[0];
            ui.same_line_with_spacing(0.0, inner);
            if ui.button("Filter") {
                ui.open_popup(popup_filter);
            }

            if !handler.events.is_empty() {
                ui.same_line_with_spacing(0.0, inner);
                if ui.button("Clear") {
                    handler.events.clear();
                    handler.current_seq = 1;
                }
            }
        }

        // Events Table
        if handler.events.is_empty() {
            ui.text("Waiting for events...");
        }

        {
            ui.separator();

            let table = ui.begin_table_with_flags(
                "table_vt_events",
                3,
                TableFlags::ROW_BG | TableFlags::BORDERS,
            );
            if let Some(_table) = table {
                ui.table_setup_column(
                    "Seq",
                    TableColumnFlags::NONE,
                    Some(TableColumnWidth::Fixed(0.0)),
                );
                ui.table_setup_column(
                    "Kind",
                    TableColumnFlags::NONE,
                    Some(TableColumnWidth::Fixed(0.0)),
                );
                ui.table_setup_column(
                    "Description",
                    TableColumnFlags::NONE,
                    Some(TableColumnWidth::Stretch(0.0)),
                );

                // Handle keyboard navigation when window is focused
                if ui.is_window_focused_with_flags(FocusedFlags::ROOT_AND_CHILD_WINDOWS) {
                    let key_pressed = get_key_action(ui);
                    let events = &handler.events;

                    match key_pressed {
                        KeyAction::None => {}
                        KeyAction::Up | KeyAction::Down => {
                            // If no event is selected, select the first/last event based on direction
                            if let Some(selected) = self.selected_event_seq {
                                // Find next/previous event based on current selection
                                // (iterating newest to oldest).
                                match key_pressed {
                                    KeyAction::Down => {
                                        let mut found = false;
                                        for ev in events.iter().rev() {
                                            if found {
                                                self.selected_event_seq = Some(ev.seq as u32);
                                                break;
                                            }
                                            if ev.seq as u32 == selected {
                                                found = true;
                                            }
                                        }
                                    }
                                    KeyAction::Up => {
                                        let mut prev_ev: Option<&VtEvent> = None;
                                        for ev in events.iter().rev() {
                                            if ev.seq as u32 == selected
                                                && let Some(prev) = prev_ev
                                            {
                                                self.selected_event_seq = Some(prev.seq as u32);
                                                break;
                                            }
                                            prev_ev = Some(ev);
                                        }
                                    }
                                    KeyAction::None => {}
                                }
                            } else if !events.is_empty() {
                                // `.forward` iterator for up, `.reverse` for down.
                                let first = if key_pressed == KeyAction::Up {
                                    events.front()
                                } else {
                                    events.back()
                                };
                                if let Some(ev) = first {
                                    self.selected_event_seq = Some(ev.seq as u32);
                                }
                            }

                            // Mark that we need to scroll to the newly selected item
                            self.need_scroll_to_selected = true;
                            self.is_keyboard_selection = true;
                        }
                    }
                }

                for (i, ev) in handler.events.iter_mut().enumerate().rev() {
                    // Need to push an ID so that our selectable is unique.
                    let _id = ui.push_id(i as i32);

                    ui.table_next_row();
                    ui.table_next_column();

                    // Store the previous selection state to detect changes
                    let was_selected = ev.imgui_selected;

                    // Update selection state based on keyboard navigation
                    if let Some(seq) = self.selected_event_seq {
                        ev.imgui_selected = ev.seq as u32 == seq;
                    }

                    // Handle selectable widget
                    if ui
                        .selectable_config("##select")
                        .selected(ev.imgui_selected)
                        .flags(SelectableFlags::SPAN_ALL_COLUMNS)
                        .build()
                    {
                        ev.imgui_selected = !ev.imgui_selected;
                        // If selection state changed, update keyboard navigation state
                        if ev.imgui_selected != was_selected {
                            self.selected_event_seq = if ev.imgui_selected {
                                Some(ev.seq as u32)
                            } else {
                                None
                            };
                            self.is_keyboard_selection = false;
                        }
                    }

                    ui.same_line();
                    ui.text(ev.seq.to_string());
                    ui.table_next_column();
                    ui.text(ev.kind.name());
                    ui.table_next_column();
                    ui.text(&ev.raw_description);

                    // If the event is selected, we render info about it. For now
                    // we put this in the last column because that's the widest and
                    // imgui has no way to make a column span.
                    if ev.imgui_selected {
                        {
                            cursor_table(ui, &ev.cursor);
                            cursor_style(ui, &ev.cursor, Some(palette));

                            if let Some(_details) =
                                ui.begin_table_with_flags("details", 2, TableFlags::NONE)
                            {
                                ui.table_next_row();
                                {
                                    ui.table_set_column_index(0);
                                    ui.text("Scroll Region");
                                }
                                {
                                    ui.table_set_column_index(1);
                                    ui.text(format!(
                                        "T={} B={} L={} R={}",
                                        ev.scrolling_region.top,
                                        ev.scrolling_region.bottom,
                                        ev.scrolling_region.left,
                                        ev.scrolling_region.right,
                                    ));
                                }

                                for (key, value) in &ev.metadata {
                                    ui.table_next_row();
                                    ui.table_next_column();
                                    ui.text(key);
                                    ui.table_next_column();
                                    ui.text(value);
                                }
                            }
                        }

                        // If this is the selected event and scrolling is needed, scroll to it
                        if self.need_scroll_to_selected && self.is_keyboard_selection {
                            ui.set_scroll_here_y(0.5);
                            self.need_scroll_to_selected = false;
                        }
                    }
                }
            }
        } // table

        if let Some(_popup) = ui
            .begin_modal_popup_config(popup_filter)
            .flags(WindowFlags::ALWAYS_AUTO_RESIZE)
            .begin()
        {
            ui.text("Changed filter settings will only affect future events.");

            ui.separator();

            if let Some(_t) = ui.begin_table_with_flags("table_filter_kind", 3, TableFlags::NONE) {
                for tag in ActionTag::ALL {
                    if matches!(tag, ActionTag::ApcPut | ActionTag::DcsPut) {
                        continue;
                    }

                    ui.table_next_column();
                    let mut value = !handler.filter_exclude.contains(tag);
                    if ui.checkbox(tag.name(), &mut value) {
                        if value {
                            handler.filter_exclude.remove(tag);
                        } else {
                            handler.filter_exclude.insert(tag);
                        }
                    }
                }
            } // Filter kind table

            ui.separator();

            ui.text(
                "Filter by string. Empty displays all, \"abc\" finds lines\n\
                 containing \"abc\", \"abc,xyz\" finds lines containing \"abc\"\n\
                 or \"xyz\", \"-abc\" excludes lines containing \"abc\".",
            );
            if ui
                .input_text("##filter_text", &mut handler.filter_text.input)
                .build()
            {
                handler.filter_text.build();
            }

            ui.separator();
            if ui.button("Close") {
                ui.close_current_popup();
            }
        } // filter popup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(s: &str) -> TextFilter {
        let mut f = TextFilter {
            input: s.to_owned(),
            ..TextFilter::default()
        };
        f.build();
        f
    }

    #[test]
    fn text_filter() {
        assert!(filter("").pass_filter("anything"));
        assert!(filter("abc").pass_filter("xxABCxx"));
        assert!(!filter("abc").pass_filter("xyz"));
        assert!(filter("abc,xyz").pass_filter("xyz"));
        assert!(!filter("-abc").pass_filter("zabcz"));
        assert!(filter("-abc").pass_filter("zzz"));
        assert!(filter("abc,-abc").pass_filter("abc"));
        assert!(!filter("-abc,abc").pass_filter("abc"));
        assert!(filter("-").pass_filter("abc"));
    }

    #[test]
    fn encode_execute_names() {
        assert_eq!(encode_execute(0x07), "BEL (0x7)");
        assert_eq!(encode_execute(0x0A), "LF (0xA)");
        assert_eq!(encode_execute(0x1f), "? (0x1F)");
    }

    #[test]
    fn encode_print_format() {
        assert_eq!(encode_print(u32::from('a')), "'a' (U+61)");
    }

    #[test]
    fn encode_csi_and_esc() {
        let mut stream = VtStream::new_alloc(VtHandler::new());
        stream.handler.state = Some(CaptureState {
            cursor: CursorSnapshot {
                x: 0,
                y: 0,
                pending_wrap: false,
                protected: false,
                style: ghostty_terminal::style::Style::default(),
                hyperlink_uri: None,
            },
            scrolling_region: ScrollingRegion {
                top: 0,
                bottom: 23,
                left: 0,
                right: 79,
            },
        });
        stream.handler.filter_exclude = ActionTagSet::default();
        // Like Zig, `vt_raw` only sees the parser actions of escape
        // sequences: printable text and C0 controls in the ground state are
        // dispatched directly to the handler.
        stream
            .next_slice(b"a\x07\x1b[1;2H\x1bM\x1b]0;title\x07")
            .unwrap();
        let descs: Vec<_> = stream
            .handler
            .events
            .iter()
            .map(|e| (e.seq, e.kind.name(), e.raw_description.clone()))
            .collect();
        assert_eq!(descs[0], (1, "csi", "1;2 H".to_owned()));
        assert_eq!(descs[1], (2, "esc", "M".to_owned()));
        assert_eq!(descs[2].1, "osc");
        assert_eq!(descs[2].2, "change_window_title ");
        assert_eq!(
            stream.handler.events[2].metadata,
            vec![("data".to_owned(), "title".to_owned())]
        );
    }

    #[test]
    fn paused_still_increments_seq_and_drops() {
        let mut stream = VtStream::new_alloc(VtHandler::new());
        stream.handler.state = None;
        stream.next_slice(b"\x07").unwrap();
        // No capture state: events dropped and the sequence is untouched.
        assert!(stream.handler.events.is_empty());
        assert_eq!(stream.handler.current_seq, 1);
    }
}
