//! The side-effecting stream handler (`termio/stream_handler.zig`).
//!
//! This is the handler used by termio for the VT [`Stream`]. It applies every
//! terminal-modifying action to the [`Terminal`] (the same behaviour as
//! `ghostty_terminal::stream_readonly`, plus the termio-specific extras such
//! as the default cursor style and the cursor-blink override) and additionally
//! handles everything that has side effects outside of the terminal:
//! query replies written back to the pty (through the termio mailbox),
//! surface notifications (title, pwd, clipboard, bell, progress, ...) and the
//! DCS/APC sub-protocols (XTGETTCAP, DECRQSS, kitty graphics).
//!
//! # Ownership / threading
//!
//! In Zig the handler holds pointers to the terminal and is wrapped by a
//! long-lived `Stream`. Here the long-lived part is [`TerminalStream`]
//! (the VT parser state plus [`HandlerState`]); on every
//! [`TerminalStream::process`] call a short-lived [`StreamHandler`] borrowing
//! the (locked) `Terminal` is built around it.
//!
//! The Zig handler may release the renderer mutex in the middle of an action
//! when a mailbox is full. That is impossible here (the guard is owned by the
//! caller), so a full mailbox instead spills into an in-order *outbox* inside
//! [`HandlerState`], which the caller flushes after unlocking
//! ([`TerminalStream::flush_outbox`]; `Termio::process_output` does this).

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{Arc, OnceLock},
};

use ghostty_apprt::{
    structs::Clipboard,
    surface::{
        DesktopNotification,
        Message as SurfaceMessage,
        ReportTitleStyle,
        SET_TITLE_LEN,
        WriteReq,
    },
};
use ghostty_config::types_b::{ClipboardAccess, OSCColorReportFormat};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_renderer::size::Size;
use ghostty_terminal::{
    cursor::Style as CursorStyle,
    terminal::{
        DeccolmMode,
        Error,
        MouseEvents,
        MouseFormat,
        MouseShiftCapture,
        ScrollViewport,
        SwitchScreenMode,
        Terminal,
    },
};
use ghostty_vt::{
    ansi::{CursorStyle as AnsiCursorStyle, DeviceAttributeReq, ModifyKeyFormat, ProtectedMode},
    apc,
    color::{Dynamic as DynamicColor, Rgb},
    csi,
    dcs,
    device_status,
    kitty::{self, key::SetMode as KittySetMode},
    modes::Mode,
    mouse_shape::MouseShape,
    osc::{
        ProgressReport,
        ProgressState,
        Terminator,
        color::{self as osc_color, Request, Target},
        semantic_prompt::Action as PromptAction,
    },
    parser::{Dcs, Parser},
    stream::{self, Action},
    utf8_decoder::Utf8Decoder,
};

use crate::{
    mailbox::Mailbox,
    message::{Message, SizeReport},
    sinks::SurfaceMailbox,
    terminfo,
    wakeup::Wakeup,
};

/// `build_config.version_string`.
pub const VERSION_STRING: &str = "1.3.1";

/// The long-lived state of the handler (everything but the terminal).
pub struct HandlerState {
    /// The cached size info (`size: *renderer.Size`). Kept up to date by
    /// `Termio::resize`.
    pub size: Size,

    /// Mailbox for data to the termio thread.
    pub termio_mailbox: Mailbox,

    /// Mailbox for the surface.
    pub surface_mailbox: Arc<dyn SurfaceMailbox>,

    /// A handle to wake up the renderer. This hints to the renderer that
    /// a repaint should happen.
    pub renderer_wakeup: Wakeup,

    /// The default cursor state. This is used with CSI q. This is
    /// set to true when we're currently in the default cursor state.
    pub default_cursor: bool,
    pub default_cursor_style: CursorStyle,
    pub default_cursor_blink: Option<bool>,

    /// The response to use for ENQ requests.
    pub enquiry_response: Vec<u8>,

    /// The color reporting format for OSC requests.
    pub osc_color_report_format: OSCColorReportFormat,

    /// The clipboard write access configuration.
    pub clipboard_write: ClipboardAccess,

    //---------------------------------------------------------------
    // Internal state
    /// The APC command handler maintains the APC state. APC is like
    /// CSI or OSC, but it is a private escape sequence that is used
    /// to send commands to the terminal emulator. This is used by
    /// the kitty graphics protocol.
    pub apc: apc::Handler,

    /// The DCS handler maintains DCS state. DCS is like CSI or OSC,
    /// but requires more stateful parsing. This is used by functionality
    /// such as XTGETTCAP.
    pub dcs: dcs::Handler,

    /// The tmux control mode viewer state (`tmux_viewer`). Dropping it is
    /// the equivalent of Zig's `viewer.deinit()` + `destroy`.
    pub tmux_viewer: Option<Box<ghostty_terminal::tmux::Viewer>>,

    /// This is set to true when a message was written to the termio
    /// mailbox. This can be used by callers to determine if they need
    /// to wake up the termio thread.
    pub termio_messaged: bool,

    /// This is set to true when we've seen a title escape sequence. We use
    /// this to determine if we need to default the window title.
    pub seen_title: bool,

    /// Messages that did not fit into a full mailbox (see module docs).
    outbox_termio: VecDeque<Message>,
    outbox_surface: VecDeque<SurfaceMessage>,
}

/// Messages that did not fit into a full mailbox, in order.
#[derive(Default)]
pub struct Outbox {
    termio: VecDeque<Message>,
    surface: VecDeque<SurfaceMessage>,
}

impl Outbox {
    pub fn is_empty(&self) -> bool {
        self.termio.is_empty() && self.surface.is_empty()
    }

    /// Deliver the messages, blocking until the receivers make room.
    pub fn deliver(self, termio: &Mailbox, surface: &dyn SurfaceMailbox) {
        let any_termio = !self.termio.is_empty();
        for msg in self.termio {
            termio.send(msg);
        }
        if any_termio {
            termio.notify();
        }
        for msg in self.surface {
            surface.push(msg, Timeout::Forever);
        }
    }
}

/// The inputs to build a [`HandlerState`].
pub struct HandlerOptions {
    pub size: Size,
    pub termio_mailbox: Mailbox,
    pub surface_mailbox: Arc<dyn SurfaceMailbox>,
    pub renderer_wakeup: Wakeup,
    pub default_cursor_style: CursorStyle,
    pub default_cursor_blink: Option<bool>,
    pub enquiry_response: Vec<u8>,
    pub osc_color_report_format: OSCColorReportFormat,
    pub clipboard_write: ClipboardAccess,
}

impl HandlerState {
    pub fn new(opts: HandlerOptions) -> Self {
        Self {
            size: opts.size,
            termio_mailbox: opts.termio_mailbox,
            surface_mailbox: opts.surface_mailbox,
            renderer_wakeup: opts.renderer_wakeup,
            default_cursor: true,
            default_cursor_style: opts.default_cursor_style,
            default_cursor_blink: opts.default_cursor_blink,
            enquiry_response: opts.enquiry_response,
            osc_color_report_format: opts.osc_color_report_format,
            clipboard_write: opts.clipboard_write,
            apc: apc::Handler::default(),
            dcs: dcs::Handler::default(),
            tmux_viewer: None,
            termio_messaged: false,
            seen_title: false,
            outbox_termio: VecDeque::new(),
            outbox_surface: VecDeque::new(),
        }
    }

    /// Queue a render with the renderer thread. The render isn't guaranteed
    /// to happen immediately but it will happen as soon as practical.
    pub fn queue_render(&self) {
        if let Err(err) = self.renderer_wakeup.notify() {
            log::warn!("failed to notify renderer err={err}");
        }
    }

    fn surface_message_writer(&mut self, msg: SurfaceMessage) {
        // See message_writer: preserve order if we already spilled.
        if !self.outbox_surface.is_empty() {
            self.outbox_surface.push_back(msg);
            return;
        }
        if self.surface_mailbox.push(msg.clone(), Timeout::Instant) == 0 {
            self.outbox_surface.push_back(msg);
        }
    }

    fn message_writer(&mut self, msg: Message) {
        self.termio_messaged = true;
        if !self.outbox_termio.is_empty() {
            self.outbox_termio.push_back(msg);
            return;
        }
        if let Err(msg) = self.termio_mailbox.try_send(msg) {
            // Make sure the IO thread is awake to make room.
            self.termio_mailbox.notify();
            self.outbox_termio.push_back(msg);
        }
    }

    /// True if there are messages that did not fit in a mailbox yet.
    pub fn has_outbox(&self) -> bool {
        !self.outbox_termio.is_empty() || !self.outbox_surface.is_empty()
    }

    /// Take the spilled messages out of the handler so they can be delivered
    /// after the locks are released (see [`Outbox::deliver`]).
    pub fn take_outbox(&mut self) -> Outbox {
        Outbox {
            termio: std::mem::take(&mut self.outbox_termio),
            surface: std::mem::take(&mut self.outbox_surface),
        }
    }

    /// Deliver the spilled messages, blocking until the receivers make room.
    /// MUST NOT be called with the terminal lock held (the receivers may
    /// need it to make progress).
    pub fn flush_outbox(&mut self) {
        let outbox = self.take_outbox();
        outbox.deliver(&self.termio_mailbox, &*self.surface_mailbox);
    }

    /// Whether messages were written to the termio mailbox since the last
    /// call (and reset the flag), i.e. whether the IO thread needs a wakeup.
    pub fn take_termio_messaged(&mut self) -> bool {
        std::mem::take(&mut self.termio_messaged)
    }
}

/// The long-lived stream wrapper (`StreamHandler.Stream` plus its handler
/// state): VT parser state that persists across reads.
pub struct TerminalStream {
    pub state: HandlerState,
    parser: Option<Parser>,
    utf8decoder: Utf8Decoder,
}

impl TerminalStream {
    pub fn new(state: HandlerState) -> Self {
        let mut parser = Parser::new();
        // `Stream.initAlloc`
        parser.osc_parser.alloc = true;
        Self {
            state,
            parser: Some(parser),
            utf8decoder: Utf8Decoder::new(),
        }
    }

    /// Feed bytes through the parser, applying the actions to `terminal`.
    /// Errors from the handler abort the call (Zig `nextSlice` error).
    pub fn process(&mut self, terminal: &mut Terminal, bytes: &[u8]) -> Result<(), Error> {
        let mut parser = self.parser.take().unwrap_or_else(|| {
            let mut p = Parser::new();
            p.osc_parser.alloc = true;
            p
        });
        let mut s = stream::Stream {
            handler: StreamHandler {
                terminal,
                st: &mut self.state,
            },
            parser: std::mem::take(&mut parser),
            utf8decoder: self.utf8decoder,
        };
        let res = s.next_slice(bytes);
        self.parser = Some(s.parser);
        self.utf8decoder = s.utf8decoder;
        res
    }

    /// Scalar variant of [`TerminalStream::process`] (Zig `next` byte by
    /// byte; used when an inspector needs to record each byte).
    pub fn process_byte(&mut self, terminal: &mut Terminal, byte: u8) -> Result<(), Error> {
        let mut s = stream::Stream {
            handler: StreamHandler {
                terminal,
                st: &mut self.state,
            },
            parser: self.parser.take().unwrap_or_default(),
            utf8decoder: self.utf8decoder,
        };
        let res = s.next(byte);
        self.parser = Some(s.parser);
        self.utf8decoder = s.utf8decoder;
        res
    }

    /// See [`HandlerState::flush_outbox`].
    pub fn flush_outbox(&mut self) {
        self.state.flush_outbox();
    }

    /// Change the configuration for this handler (`changeConfig`). Must be
    /// called with the terminal locked.
    pub fn change_config(
        &mut self,
        terminal: &mut Terminal,
        config: &crate::termio::DerivedConfig,
    ) {
        let st = &mut self.state;
        st.osc_color_report_format = config.osc_color_report_format;
        st.clipboard_write = config.clipboard_write;
        st.enquiry_response = config.enquiry_response.clone().into_bytes();
        st.default_cursor_style = config.cursor_style;
        st.default_cursor_blink = config.cursor_blink;

        let mut h = StreamHandler { terminal, st };
        // If our cursor is the default, then we update it immediately.
        if h.st.default_cursor {
            h.set_cursor_style_default();
        }

        // The config could have changed any of our colors so update mode 2031
        h.st.message_writer(Message::ColorSchemeReport { force: false });
    }
}

/// The handler (see module docs).
pub struct StreamHandler<'a> {
    pub terminal: &'a mut Terminal,
    pub st: &'a mut HandlerState,
}

/// All the xtgettcap responses for the ghostty terminfo (complete DCS
/// replies), keyed by the hex-encoded uppercase capability name.
fn xtgettcap_map() -> &'static std::collections::HashMap<String, Vec<u8>> {
    static MAP: OnceLock<std::collections::HashMap<String, Vec<u8>>> = OnceLock::new();
    MAP.get_or_init(|| {
        terminfo::ghostty::GHOSTTY
            .xtgettcap_map()
            .into_iter()
            .map(|(k, v)| (k, v.into_bytes()))
            .collect()
    })
}

impl StreamHandler<'_> {
    fn message_writer(&mut self, msg: Message) {
        self.st.message_writer(msg);
    }

    fn surface_message_writer(&mut self, msg: SurfaceMessage) {
        self.st.surface_message_writer(msg);
    }

    fn write_req(&mut self, data: &[u8]) {
        self.message_writer(Message::write_req(data));
    }

    fn horizontal_tab(&mut self, count: u16) {
        for _ in 0..count {
            let x = self.terminal.screen().cursor.x;
            self.terminal.horizontal_tab();
            if x == self.terminal.screen().cursor.x {
                break;
            }
        }
    }

    fn horizontal_tab_back(&mut self, count: u16) {
        for _ in 0..count {
            let x = self.terminal.screen().cursor.x;
            self.terminal.horizontal_tab_back();
            if x == self.terminal.screen().cursor.x {
                break;
            }
        }
    }

    fn full_reset(&mut self) {
        self.terminal.full_reset();
        self.set_mouse_shape(MouseShape::Text);

        // Reset resets our palette so we report it for mode 2031.
        self.message_writer(Message::ColorSchemeReport { force: false });

        // Clear the progress bar
        self.progress_report(ProgressReport {
            state: ProgressState::Remove,
            progress: None,
        });
    }

    fn request_mode(&mut self, mode: Mode) {
        let code = if self.terminal.modes.get(mode) { 1 } else { 2 };
        let resp = format!(
            "\x1B[{}{};{}$y",
            if mode.ansi() { "" } else { "?" },
            mode.value(),
            code
        );
        self.write_req(resp.as_bytes());
    }

    fn request_mode_unknown(&mut self, mode_raw: u16, ansi: bool) {
        let resp = format!("\x1B[{}{};0$y", if ansi { "" } else { "?" }, mode_raw);
        self.write_req(resp.as_bytes());
    }

    fn set_mode(&mut self, mode: Mode, enabled: bool) -> Result<(), Error> {
        // If we are setting cursor blinking, we ignore it if we have
        // a default cursor blink setting set. This is a really weird
        // behavior: there are two ways to set cursor blinks, DECSCUSR
        // (CSI _ q) and DEC mode 12. DECSCUSR has a way to revert to
        // the "default" (CSI 0 q) but mode 12 doesn't, so once a program
        // used mode 12 the blink could never be reset to the user's
        // `cursor-style-blink`. If the user specified an explicit default
        // we ignore mode 12 (DECSCUSR can still change it since programs
        // using it are well behaved and reset to the default on exit).
        if mode == Mode::CursorBlinking && self.st.default_cursor_blink.is_some() {
            return Ok(());
        }

        // We first always set the raw mode on our mode state.
        self.terminal.modes.set(mode, enabled);

        // And then some modes require additional processing.
        match mode {
            // Just noting here that autorepeat has no effect on
            // the terminal. xterm ignores this mode and so do we.
            Mode::Autorepeat => {}

            // Schedule a render since we changed colors
            Mode::ReverseColors => self.terminal.flags.dirty.reverse_colors = true,

            // Origin resets cursor pos. This is called whether or not
            // we're enabling or disabling origin mode and whether or
            // not the value changed.
            Mode::Origin => self.terminal.set_cursor_pos(1, 1),

            Mode::EnableLeftAndRightMargin => {
                if !enabled {
                    // When we disable left/right margin mode we need to
                    // reset the left/right margins.
                    self.terminal.scrolling_region.left = 0;
                    self.terminal.scrolling_region.right = self.terminal.cols - 1;
                }
            }

            Mode::AltScreenLegacy => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M47, enabled)?,
            Mode::AltScreen => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M1047, enabled)?,
            Mode::AltScreenSaveCursorClearEnter => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M1049, enabled)?,

            // Mode 1048 is xterm's conditional save cursor depending
            // on if alt screen is enabled or not (at the terminal emulator
            // level). Alt screen is always enabled for us so this just
            // does a save/restore cursor.
            Mode::SaveCursor => {
                if enabled {
                    self.terminal.save_cursor();
                } else {
                    self.terminal.restore_cursor();
                }
            }

            // Force resize back to the window size
            Mode::EnableMode3 => {
                let grid_size = self.st.size.grid();
                if let Err(err) = self.terminal.resize(grid_size.columns, grid_size.rows) {
                    log::error!("error updating terminal size: {err:?}");
                }
            }

            Mode::M132Column => self.terminal.deccolm(if enabled {
                DeccolmMode::Cols132
            } else {
                DeccolmMode::Cols80
            })?,

            // We need to start a timer to prevent the emulator being hung
            // forever.
            Mode::SynchronizedOutput => {
                if enabled {
                    self.message_writer(Message::StartSynchronizedOutput);
                }
            }

            Mode::Linefeed => self.message_writer(Message::LinefeedMode(enabled)),

            Mode::InBandSizeReports => {
                if enabled {
                    self.message_writer(Message::SizeReport(SizeReport::Mode2048));
                }
            }

            Mode::FocusEvent => {
                if enabled {
                    let focused = self.terminal.flags.focused;
                    self.message_writer(Message::Focused(focused));
                }
            }

            Mode::MouseEventX10 => self.set_mouse_event(enabled, MouseEvents::X10),
            Mode::MouseEventNormal => self.set_mouse_event(enabled, MouseEvents::Normal),
            Mode::MouseEventButton => self.set_mouse_event(enabled, MouseEvents::Button),
            Mode::MouseEventAny => self.set_mouse_event(enabled, MouseEvents::Any),

            Mode::MouseFormatUtf8 => self.set_mouse_format(enabled, MouseFormat::Utf8),
            Mode::MouseFormatSgr => self.set_mouse_format(enabled, MouseFormat::Sgr),
            Mode::MouseFormatUrxvt => self.set_mouse_format(enabled, MouseFormat::Urxvt),
            Mode::MouseFormatSgrPixels => self.set_mouse_format(enabled, MouseFormat::SgrPixels),

            _ => {}
        }
        Ok(())
    }

    fn set_mouse_event(&mut self, enabled: bool, ev: MouseEvents) {
        if enabled {
            self.terminal.flags.mouse_event = ev;
            self.set_mouse_shape(MouseShape::Default);
        } else {
            self.terminal.flags.mouse_event = MouseEvents::None;
            self.set_mouse_shape(MouseShape::Text);
        }
    }

    fn set_mouse_format(&mut self, enabled: bool, f: MouseFormat) {
        self.terminal.flags.mouse_format = if enabled { f } else { MouseFormat::X10 };
    }

    fn device_attributes(&mut self, req: DeviceAttributeReq) {
        // For the below, we quack as a VT220. We don't quack as
        // a 420 because we don't support DCS sequences.
        match req {
            DeviceAttributeReq::Primary => {
                // 62 = Level 2 conformance
                // 22 = Color text
                // 52 = Clipboard access
                let resp: &'static [u8] = if self.st.clipboard_write == ClipboardAccess::Deny {
                    b"\x1B[?62;22c"
                } else {
                    b"\x1B[?62;22;52c"
                };
                self.message_writer(Message::write_stable(resp));
            }
            DeviceAttributeReq::Secondary => {
                self.message_writer(Message::write_stable(b"\x1B[>1;10;0c"));
            }
            req @ DeviceAttributeReq::Tertiary => {
                log::warn!("unimplemented device attributes req: {req:?}");
            }
        }
    }

    fn device_status_report(&mut self, req: device_status::Request) {
        match req {
            device_status::Request::OperatingStatus => {
                self.message_writer(Message::write_stable(b"\x1B[0n"));
            }

            device_status::Request::CursorPosition => {
                let cursor = &self.terminal.screen().cursor;
                let (x, y) = if self.terminal.modes.get(Mode::Origin) {
                    (
                        cursor.x.saturating_sub(self.terminal.scrolling_region.left),
                        cursor.y.saturating_sub(self.terminal.scrolling_region.top),
                    )
                } else {
                    (cursor.x, cursor.y)
                };
                let resp = format!("\x1B[{};{}R", usize::from(y) + 1, usize::from(x) + 1);
                self.write_req(resp.as_bytes());
            }

            device_status::Request::ColorScheme => {
                self.message_writer(Message::ColorSchemeReport { force: true });
            }
        }
    }

    fn set_cursor_style_default(&mut self) {
        self.st.default_cursor = true;
        self.terminal.screen_mut().cursor.cursor_style = self.st.default_cursor_style;
        let blink = self.st.default_cursor_blink.unwrap_or(true);
        self.terminal.modes.set(Mode::CursorBlinking, blink);
    }

    fn set_cursor_style(&mut self, style: AnsiCursorStyle) {
        // Assume we're setting to a non-default.
        self.st.default_cursor = false;

        let (cs, blink) = match style {
            AnsiCursorStyle::Default => {
                self.set_cursor_style_default();
                return;
            }
            AnsiCursorStyle::BlinkingBlock => (CursorStyle::Block, true),
            AnsiCursorStyle::SteadyBlock => (CursorStyle::Block, false),
            AnsiCursorStyle::BlinkingUnderline => (CursorStyle::Underline, true),
            AnsiCursorStyle::SteadyUnderline => (CursorStyle::Underline, false),
            AnsiCursorStyle::BlinkingBar => (CursorStyle::Bar, true),
            AnsiCursorStyle::SteadyBar => (CursorStyle::Bar, false),
        };
        self.terminal.screen_mut().cursor.cursor_style = cs;
        self.terminal.modes.set(Mode::CursorBlinking, blink);
    }

    fn query_kitty_keyboard(&mut self) {
        log::debug!("querying kitty keyboard mode");
        let resp = format!(
            "\x1b[?{}u",
            self.terminal.screen().kitty_keyboard.current().int()
        );
        self.write_req(resp.as_bytes());
    }

    fn report_xtversion(&mut self) {
        log::debug!("reporting XTVERSION: ghostty {VERSION_STRING}");
        let resp = format!("\x1BP>|ghostty {VERSION_STRING}\x1B\\");
        self.write_req(resp.as_bytes());
    }

    //-------------------------------------------------------------------------
    // DCS / APC

    fn dcs_command(&mut self, cmd: dcs::Command) {
        match cmd {
            dcs::Command::Tmux(tmux) => {
                use ghostty_terminal::tmux::{
                    control::Notification,
                    viewer::{Action as ViewerAction, Input},
                };

                log::info!("tmux control mode event cmd={tmux:?}");

                match tmux {
                    Notification::Enter => {
                        // Setup our viewer state
                        assert!(self.st.tmux_viewer.is_none());
                        self.st.tmux_viewer = Some(Box::new(ghostty_terminal::tmux::Viewer::new()));
                        return;
                    }

                    Notification::Exit => {
                        // Free our viewer state if we have one
                        self.st.tmux_viewer = None;
                        return;
                    }

                    _ => {}
                }

                let Some(viewer) = self.st.tmux_viewer.as_mut() else {
                    // This can only really happen if we failed to
                    // initialize the viewer on enter.
                    log::info!("received tmux control mode command without viewer: {tmux:?}");
                    return;
                };

                let actions = viewer.next(Input::Tmux(tmux));
                for action in actions {
                    log::info!("tmux viewer action={action:?}");
                    match action {
                        // We ignore this because we will fully exit when
                        // our DCS connection ends. We may want to handle
                        // this in the future to notify our GUI we're
                        // disconnected though.
                        ViewerAction::Exit => {}

                        ViewerAction::Command(command) => {
                            assert_eq!(command.last(), Some(&b'\n'));
                            self.write_req(&command);
                        }

                        // TODO (Zig has the same TODO): windows are not
                        // surfaced to the GUI yet.
                        ViewerAction::Windows(_) => {}
                    }
                }
            }

            dcs::Command::Xtgettcap(mut gettcap) => {
                let map = xtgettcap_map();
                while let Some(key) = gettcap.next() {
                    let Ok(key) = std::str::from_utf8(key) else {
                        continue;
                    };
                    let Some(response) = map.get(key) else {
                        continue;
                    };
                    self.write_req(response);
                }
            }

            dcs::Command::Decrqss(decrqss) => {
                // Our response is valid if we have a response payload
                let payload: Option<String> = match decrqss {
                    // Invalid or unhandled request
                    dcs::Decrqss::None => None,

                    dcs::Decrqss::Sgr => {
                        let mut s = self.terminal.print_attributes();
                        s.push('m');
                        Some(s)
                    }

                    dcs::Decrqss::Decscusr => {
                        let blink = self.terminal.modes.get(Mode::CursorBlinking);
                        let style: u8 = match self.terminal.screen().cursor.cursor_style {
                            CursorStyle::Block => {
                                if blink {
                                    1
                                } else {
                                    2
                                }
                            }
                            CursorStyle::Underline => {
                                if blink {
                                    3
                                } else {
                                    4
                                }
                            }
                            CursorStyle::Bar => {
                                if blink {
                                    5
                                } else {
                                    6
                                }
                            }
                            // Below here, the cursor styles aren't represented by
                            // DECSCUSR so we map it to some other style.
                            CursorStyle::BlockHollow => {
                                if blink {
                                    1
                                } else {
                                    2
                                }
                            }
                        };
                        Some(format!("{style} q"))
                    }

                    dcs::Decrqss::Decstbm => Some(format!(
                        "{};{}r",
                        u32::from(self.terminal.scrolling_region.top) + 1,
                        u32::from(self.terminal.scrolling_region.bottom) + 1
                    )),

                    dcs::Decrqss::Decslrm => {
                        // We only send a valid response when left and right
                        // margin mode (DECLRMM) is enabled.
                        if self.terminal.modes.get(Mode::EnableLeftAndRightMargin) {
                            Some(format!(
                                "{};{}s",
                                u32::from(self.terminal.scrolling_region.left) + 1,
                                u32::from(self.terminal.scrolling_region.right) + 1
                            ))
                        } else {
                            None
                        }
                    }
                };

                let valid = payload.as_ref().is_some_and(|p| !p.is_empty());
                let mut response = format!("\x1bP{}$r", u8::from(valid));
                if let Some(p) = payload {
                    response.push_str(&p);
                }
                response.push_str("\x1b\\");
                self.write_req(response.as_bytes());
            }
        }
    }

    fn apc_end(&mut self) {
        let Some(cmd) = self.st.apc.end() else {
            return;
        };

        match cmd {
            apc::Command::Kitty(mut kitty_cmd) => {
                if let Some(resp) = self.terminal.kitty_graphics(&mut kitty_cmd) {
                    let mut buf: Vec<u8> = Vec::new();
                    if resp.encode(&mut buf).is_ok() && buf.len() > 2 {
                        log::debug!("kitty graphics response: {buf:x?}");
                        self.write_req(&buf);
                    }
                }
            }
        }
    }

    //-------------------------------------------------------------------------
    // OSC

    fn window_title(&mut self, title: &str) {
        let mut buf = [0u8; SET_TITLE_LEN];
        let title = title.as_bytes();
        if title.len() >= buf.len() {
            log::warn!("change title requested larger than our buffer size, ignoring");
            return;
        }

        buf[..title.len()].copy_from_slice(title);

        // Special handling for the empty title. We treat the empty title
        // as resetting to as if we never saw a title. Other terminals
        // behave this way too (e.g. iTerm2).
        if title.is_empty() {
            // If we have a pwd then we set the title as the pwd else
            // we just set it to blank.
            if let Some(pwd) = self.terminal.get_pwd()
                && pwd.len() < buf.len()
            {
                buf[..pwd.len()].copy_from_slice(pwd);
                buf[pwd.len()] = 0;
            }

            self.surface_message_writer(SurfaceMessage::SetTitle(buf));
            self.st.seen_title = false;
            return;
        }

        self.st.seen_title = true;
        self.surface_message_writer(SurfaceMessage::SetTitle(buf));
    }

    fn set_mouse_shape(&mut self, shape: MouseShape) {
        // Avoid changing the shape it it is already set to avoid excess
        // cross-thread messaging.
        if self.terminal.mouse_shape == shape {
            return;
        }

        self.terminal.mouse_shape = shape;
        self.surface_message_writer(SurfaceMessage::SetMouseShape(shape));
    }

    fn clipboard_contents(&mut self, kind: u8, data: &[u8]) {
        // Note: we ignore the "kind" field and always use the standard clipboard.
        // iTerm also appears to do this but other terminals seem to only allow
        // certain. Let's investigate more.
        let clipboard_type = match kind {
            b's' => Clipboard::Selection,
            b'p' => Clipboard::Primary,
            // 'c' and anything else
            _ => Clipboard::Standard,
        };

        // Get clipboard contents
        if data == b"?" {
            self.surface_message_writer(SurfaceMessage::ClipboardRead(clipboard_type));
            return;
        }

        // Write clipboard contents
        self.surface_message_writer(SurfaceMessage::ClipboardWrite {
            req: WriteReq::init(data),
            clipboard_type,
        });
    }

    fn semantic_prompt(&mut self, cmd: &ghostty_vt::osc::SemanticPrompt) -> Result<(), Error> {
        match cmd.action {
            PromptAction::EndInputStartOutput => {
                self.surface_message_writer(SurfaceMessage::StartCommand);
            }

            PromptAction::EndCommand => {
                // The specification seems to not specify the type but
                // other terminals accept 32-bits, but exit codes are really
                // bytes, so we just do our best here.
                let raw: i32 = cmd.exit_code().unwrap_or(0);
                let code: u8 = u8::try_from(raw).unwrap_or(1);
                self.surface_message_writer(SurfaceMessage::StopCommand(Some(code)));
            }

            // Handled by Terminal, no special handling by us
            PromptAction::EndPromptStartInput
            | PromptAction::EndPromptStartInputTerminateEol
            | PromptAction::FreshLine
            | PromptAction::FreshLineNewPrompt
            | PromptAction::NewCommand
            | PromptAction::PromptStart => {}
        }

        // We do this last so failures are still processed correctly
        // above.
        self.terminal.semantic_prompt(cmd)
    }

    fn report_pwd(&mut self, url: &[u8]) -> Result<(), Error> {
        // Special handling for the empty URL. We treat the empty URL
        // as resetting the pwd as if we never saw a pwd. I can't find any
        // other terminal that does this but it seems like a reasonable
        // behavior that enables some useful features. For example, the macOS
        // proxy icon can be hidden when a program reports it doesn't know
        // the pwd rather than showing a stale pwd.
        if url.is_empty() {
            // Blank value can never fail because no allocs happen.
            let _ = self.terminal.set_pwd(b"");

            // If we haven't seen a title, we're using the pwd as our title.
            // Set it to blank which will reset our title behavior.
            if !self.st.seen_title {
                self.window_title("");
                debug_assert!(!self.st.seen_title);
            }

            // Report the change.
            self.surface_message_writer(SurfaceMessage::PwdChange(WriteReq::Stable(b"")));
            return Ok(());
        }

        // Attempt to parse this file-style URI using options appropriate
        // for this OSC 7 context (e.g. kitty-shell-cwd expects the full,
        // unencoded path). Parsing is byte based so non-UTF-8 path bytes
        // survive percent-decoding.
        let uri = match ghostty_os::uri::parse_bytes(
            url,
            ghostty_os::uri::ParseOptions {
                mac_address: true,
                raw_path: url.starts_with(b"kitty-shell-cwd://"),
            },
        ) {
            Ok(uri) => uri,
            Err(e) => {
                log::warn!("invalid url in OSC 7: {e}");
                return Ok(());
            }
        };

        if uri.scheme != "file" && uri.scheme != "kitty-shell-cwd" {
            log::warn!(
                "OSC 7 scheme must be file or kitty-shell-cwd, got: {}",
                uri.scheme
            );
            return Ok(());
        }

        let Some(host) = uri.host.as_ref() else {
            log::warn!("OSC 7 uri must contain a hostname: UriMissingHost");
            return Ok(());
        };

        // OSC 7 is a little sketchy because anyone can send any value from
        // any host (such an SSH session). The best practice terminals follow
        // is to valid the hostname to be local.
        let host_valid = match ghostty_os::hostname::is_local(host) {
            Ok(v) => v,
            Err(err) => {
                log::warn!("failed to get hostname for OSC 7 validation: {err:?}");
                return Ok(());
            }
        };
        if !host_valid {
            log::warn!("OSC 7 host ({host}) must be local");
            return Ok(());
        }

        // We need the raw path, which might require unescaping.
        let path = uri.path;

        log::debug!("terminal pwd: {}", String::from_utf8_lossy(&path));
        self.terminal.set_pwd(&path)?;

        // Report it to the surface.
        self.surface_message_writer(SurfaceMessage::PwdChange(WriteReq::init(&path)));

        // If we haven't seen a title, use our pwd as the title.
        if !self.st.seen_title {
            // Titles are text; only the title is lossy.
            self.window_title(&String::from_utf8_lossy(&path));
            self.st.seen_title = false;
        }
        Ok(())
    }

    fn color_operation(&mut self, requests: &osc_color::List, terminator: Terminator) {
        // return early if there is nothing to do
        if requests.is_empty() {
            return;
        }

        let mut response: Vec<u8> = Vec::new();

        for req in requests {
            match *req {
                Request::Set(set) => {
                    match set.target {
                        Target::Palette(i) => {
                            self.terminal.flags.dirty.palette = true;
                            self.terminal.colors.palette.set(i, set.color);
                        }
                        Target::Dynamic(dynamic) => match dynamic {
                            DynamicColor::Foreground => {
                                self.terminal.colors.foreground.set(set.color);
                            }
                            DynamicColor::Background => {
                                self.terminal.colors.background.set(set.color);
                            }
                            DynamicColor::Cursor => self.terminal.colors.cursor.set(set.color),
                            other => {
                                log::info!("setting dynamic color {other:?} not implemented");
                            }
                        },
                        Target::Special(_) => {
                            log::info!("setting special colors not implemented");
                        }
                    }

                    // Notify the surface of the color change
                    self.surface_message_writer(SurfaceMessage::ColorChange(
                        osc_color::ColoredTarget {
                            target: set.target,
                            color: set.color,
                        },
                    ));
                }

                Request::Reset(target) => match target {
                    Target::Palette(i) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(i);

                        let color = self.terminal.colors.palette.current[usize::from(i)];
                        self.surface_message_writer(SurfaceMessage::ColorChange(
                            osc_color::ColoredTarget { target, color },
                        ));
                    }
                    Target::Dynamic(dynamic) => {
                        let slot = match dynamic {
                            DynamicColor::Foreground => Some(&mut self.terminal.colors.foreground),
                            DynamicColor::Background => Some(&mut self.terminal.colors.background),
                            DynamicColor::Cursor => Some(&mut self.terminal.colors.cursor),
                            other => {
                                log::warn!("resetting dynamic color {other:?} not implemented");
                                None
                            }
                        };
                        if let Some(slot) = slot {
                            slot.reset();
                            if let Some(color) = slot.default {
                                self.surface_message_writer(SurfaceMessage::ColorChange(
                                    osc_color::ColoredTarget { target, color },
                                ));
                            }
                        }
                    }
                    Target::Special(_) => {
                        log::info!("resetting special colors not implemented");
                    }
                },

                Request::ResetPalette => {
                    let indices: Vec<usize> = self.terminal.colors.palette.mask.iter().collect();
                    for i in indices {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(i as u8);
                        let color = self.terminal.colors.palette.current[i];
                        self.surface_message_writer(SurfaceMessage::ColorChange(
                            osc_color::ColoredTarget {
                                target: Target::Palette(i as u8),
                                color,
                            },
                        ));
                    }
                    self.terminal.colors.palette.mask =
                        ghostty_vt::color::PaletteMask::init_empty();
                }

                Request::ResetSpecial => {
                    log::warn!("resetting all special colors not implemented");
                }

                Request::Query(kind) => {
                    if self.st.osc_color_report_format == OSCColorReportFormat::None {
                        continue;
                    }

                    let color: Rgb = match kind {
                        Target::Palette(i) => self.terminal.colors.palette.current[usize::from(i)],
                        Target::Dynamic(dynamic) => match dynamic {
                            DynamicColor::Foreground => {
                                let Some(c) = self.terminal.colors.foreground.get() else {
                                    continue;
                                };
                                c
                            }
                            DynamicColor::Background => {
                                let Some(c) = self.terminal.colors.background.get() else {
                                    continue;
                                };
                                c
                            }
                            DynamicColor::Cursor => {
                                let Some(c) = self
                                    .terminal
                                    .colors
                                    .cursor
                                    .get()
                                    .or_else(|| self.terminal.colors.foreground.get())
                                else {
                                    continue;
                                };
                                c
                            }
                            other => {
                                log::info!("reporting dynamic color {other:?} not implemented");
                                continue;
                            }
                        },
                        Target::Special(_) => {
                            log::info!("reporting special colors not implemented");
                            continue;
                        }
                    };

                    let mut s = String::new();
                    let (width, scale): (usize, u16) = match self.st.osc_color_report_format {
                        OSCColorReportFormat::Bit16 => (4, 257),
                        _ => (2, 1),
                    };
                    let prefix = match kind {
                        Target::Palette(i) => format!("\x1b]4;{i};"),
                        Target::Dynamic(d) => format!("\x1b]{};", d as u8),
                        Target::Special(_) => continue,
                    };
                    let _ = write!(
                        s,
                        "{prefix}rgb:{:0width$x}/{:0width$x}/{:0width$x}",
                        u16::from(color.r) * scale,
                        u16::from(color.g) * scale,
                        u16::from(color.b) * scale,
                    );
                    response.extend_from_slice(s.as_bytes());
                    response.extend_from_slice(terminator.string());
                }
            }
        }

        if !response.is_empty() {
            // If any of the operations were reports, finalize the report
            // string and send it to the terminal.
            self.write_req(&response);
        }
    }

    fn show_desktop_notification(&mut self, title: &[u8], body: &[u8]) {
        self.surface_message_writer(SurfaceMessage::DesktopNotification(
            DesktopNotification::new(title, body),
        ));
    }

    /// Send a report to the pty.
    fn send_size_report(&mut self, style: csi::SizeReportStyle) {
        match style {
            csi::SizeReportStyle::Csi14T => {
                self.message_writer(Message::SizeReport(SizeReport::Csi14T));
            }
            csi::SizeReportStyle::Csi16T => {
                self.message_writer(Message::SizeReport(SizeReport::Csi16T));
            }
            csi::SizeReportStyle::Csi18T => {
                self.message_writer(Message::SizeReport(SizeReport::Csi18T));
            }
            csi::SizeReportStyle::Csi21T => {
                self.surface_message_writer(SurfaceMessage::ReportTitle(ReportTitleStyle::Csi21T));
            }
        }
    }

    fn kitty_color_report(&mut self, request: &kitty::color::OSC) {
        use kitty::color::{Kind, Request as KReq, Special};

        let mut out: Vec<u8> = Vec::new();

        for item in &request.list {
            match item {
                KReq::Query(key) => {
                    // If the writer buffer is empty, we need to write our prefix
                    if out.is_empty() {
                        out.extend_from_slice(b"\x1b]21");
                    }

                    let color: Option<Rgb> = match key {
                        Kind::Palette(p) => {
                            Some(self.terminal.colors.palette.current[usize::from(*p)])
                        }
                        Kind::Special(special) => match special {
                            Special::Foreground => self.terminal.colors.foreground.get(),
                            Special::Background => self.terminal.colors.background.get(),
                            Special::Cursor => self.terminal.colors.cursor.get(),
                            _ => {
                                log::warn!("ignoring unsupported kitty color protocol key: {key}");
                                continue;
                            }
                        },
                    };
                    let Some(color) = color else {
                        out.extend_from_slice(format!(";{key}=").as_bytes());
                        continue;
                    };

                    out.extend_from_slice(
                        format!(";{key}=rgb:{:02x}/{:02x}/{:02x}", color.r, color.g, color.b)
                            .as_bytes(),
                    );
                }
                KReq::Set { key, color } => match key {
                    Kind::Palette(palette) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.set(*palette, *color);
                    }
                    Kind::Special(special) => match special {
                        Special::Foreground => self.terminal.colors.foreground.set(*color),
                        Special::Background => self.terminal.colors.background.set(*color),
                        Special::Cursor => self.terminal.colors.cursor.set(*color),
                        _ => {
                            log::warn!("ignoring unsupported kitty color protocol key: {key}");
                        }
                    },
                },
                KReq::Reset(key) => match key {
                    Kind::Palette(palette) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(*palette);
                    }
                    Kind::Special(special) => match special {
                        Special::Foreground => self.terminal.colors.foreground.reset(),
                        Special::Background => self.terminal.colors.background.reset(),
                        Special::Cursor => self.terminal.colors.cursor.reset(),
                        _ => {
                            log::warn!("ignoring unsupported kitty color protocol key: {key}");
                        }
                    },
                },
            }
        }

        // If we had any writes to our buffer, we queue them now
        if !out.is_empty() {
            out.extend_from_slice(request.terminator.string());
            self.write_req(&out);
        }

        // Note: we don't have to do a queue_render here because every
        // processed stream will queue a render once it is done processing
        // the read() syscall.
    }

    /// Display a GUI progress report.
    fn progress_report(&mut self, report: ProgressReport) {
        self.surface_message_writer(SurfaceMessage::ProgressReport(report));
    }

    fn dcs_hook(&mut self, d: &Dcs) {
        if let Some(cmd) = self.st.dcs.hook(d) {
            self.dcs_command(cmd);
        }
    }

    fn dcs_put(&mut self, byte: u8) {
        if let Some(cmd) = self.st.dcs.put(byte) {
            self.dcs_command(cmd);
        }
    }

    fn dcs_unhook(&mut self) {
        if let Some(cmd) = self.st.dcs.unhook() {
            self.dcs_command(cmd);
        }
    }
}

impl stream::Handler for StreamHandler<'_> {
    type Error = Error;

    fn vt(&mut self, action: Action<'_>) -> Result<(), Error> {
        match action {
            Action::Print(v) => self.terminal.print(v.cp)?,
            Action::PrintRepeat(v) => self.terminal.print_repeat(v)?,
            Action::Bell => self.surface_message_writer(SurfaceMessage::RingBell),
            Action::Backspace => self.terminal.backspace(),
            Action::HorizontalTab(v) => self.horizontal_tab(v),
            Action::HorizontalTabBack(v) => self.horizontal_tab_back(v),
            Action::Linefeed | Action::Index => self.terminal.index()?,
            Action::CarriageReturn => self.terminal.carriage_return(),
            Action::Enquiry => {
                log::debug!("sending enquiry response");
                let resp = self.st.enquiry_response.clone();
                self.write_req(&resp);
            }
            Action::InvokeCharset(v) => {
                self.terminal.invoke_charset(v.bank, v.charset, v.locking);
            }
            Action::CursorUp(v) => self.terminal.cursor_up(usize::from(v.value)),
            Action::CursorDown(v) => self.terminal.cursor_down(usize::from(v.value)),
            Action::CursorLeft(v) => self.terminal.cursor_left(usize::from(v.value)),
            Action::CursorRight(v) => self.terminal.cursor_right(usize::from(v.value)),
            Action::CursorPos(v) => self
                .terminal
                .set_cursor_pos(usize::from(v.row), usize::from(v.col)),
            Action::CursorCol(v) => {
                let row = usize::from(self.terminal.screen().cursor.y) + 1;
                self.terminal.set_cursor_pos(row, usize::from(v.value));
            }
            Action::CursorRow(v) => {
                let col = usize::from(self.terminal.screen().cursor.x) + 1;
                self.terminal.set_cursor_pos(usize::from(v.value), col);
            }
            Action::CursorColRelative(v) => {
                let row = usize::from(self.terminal.screen().cursor.y) + 1;
                let col = (usize::from(self.terminal.screen().cursor.x) + 1)
                    .saturating_add(usize::from(v.value));
                self.terminal.set_cursor_pos(row, col);
            }
            Action::CursorRowRelative(v) => {
                let row = (usize::from(self.terminal.screen().cursor.y) + 1)
                    .saturating_add(usize::from(v.value));
                let col = usize::from(self.terminal.screen().cursor.x) + 1;
                self.terminal.set_cursor_pos(row, col);
            }
            Action::CursorStyle(v) => self.set_cursor_style(v),
            Action::EraseDisplayBelow(p) => {
                self.terminal.erase_display(csi::EraseDisplay::Below, p);
            }
            Action::EraseDisplayAbove(p) => {
                self.terminal.erase_display(csi::EraseDisplay::Above, p);
            }
            Action::EraseDisplayComplete(p) => {
                self.terminal.scroll_viewport(ScrollViewport::Bottom);
                self.terminal.erase_display(csi::EraseDisplay::Complete, p);
            }
            Action::EraseDisplayScrollback(p) => {
                self.terminal
                    .erase_display(csi::EraseDisplay::Scrollback, p);
            }
            Action::EraseDisplayScrollComplete(p) => {
                self.terminal
                    .erase_display(csi::EraseDisplay::ScrollComplete, p);
            }
            Action::EraseLineRight(p) => self.terminal.erase_line(csi::EraseLine::Right, p),
            Action::EraseLineLeft(p) => self.terminal.erase_line(csi::EraseLine::Left, p),
            Action::EraseLineComplete(p) => self.terminal.erase_line(csi::EraseLine::Complete, p),
            Action::EraseLineRightUnlessPendingWrap(p) => {
                self.terminal
                    .erase_line(csi::EraseLine::RightUnlessPendingWrap, p);
            }
            Action::DeleteChars(v) => self.terminal.delete_chars(v),
            Action::EraseChars(v) => self.terminal.erase_chars(v),
            Action::InsertLines(v) => self.terminal.insert_lines(v),
            Action::InsertBlanks(v) => self.terminal.insert_blanks(v),
            Action::DeleteLines(v) => self.terminal.delete_lines(v),
            Action::ScrollUp(v) => self.terminal.scroll_up(v)?,
            Action::ScrollDown(v) => self.terminal.scroll_down(v),
            Action::TabClearCurrent => self.terminal.tab_clear(csi::TabClear::Current),
            Action::TabClearAll => self.terminal.tab_clear(csi::TabClear::All),
            Action::TabSet => self.terminal.tab_set(),
            Action::TabReset => self.terminal.tab_reset(),
            Action::NextLine => {
                self.terminal.index()?;
                self.terminal.carriage_return();
            }
            Action::ReverseIndex => self.terminal.reverse_index(),
            Action::FullReset => self.full_reset(),
            Action::SetMode(v) => self.set_mode(v.mode, true)?,
            Action::ResetMode(v) => self.set_mode(v.mode, false)?,
            Action::SaveMode(v) => self.terminal.modes.save(v.mode),
            Action::RestoreMode(v) => {
                // For restore mode we have to restore but if we set it, we
                // always have to call setMode because setting some modes have
                // side effects and we want to make sure we process those.
                let enabled = self.terminal.modes.restore(v.mode);
                self.set_mode(v.mode, enabled)?;
            }
            Action::RequestMode(v) => self.request_mode(v.mode),
            Action::RequestModeUnknown(v) => self.request_mode_unknown(v.mode, v.ansi),
            Action::TopAndBottomMargin(v) => {
                self.terminal.set_top_and_bottom_margin(
                    usize::from(v.top_left),
                    usize::from(v.bottom_right),
                );
            }
            Action::LeftAndRightMargin(v) => {
                self.terminal.set_left_and_right_margin(
                    usize::from(v.top_left),
                    usize::from(v.bottom_right),
                );
            }
            Action::LeftAndRightMarginAmbiguous => {
                if self.terminal.modes.get(Mode::EnableLeftAndRightMargin) {
                    self.terminal.set_left_and_right_margin(0, 0);
                } else {
                    self.terminal.save_cursor();
                }
            }
            Action::SaveCursor => self.terminal.save_cursor(),
            Action::RestoreCursor => self.terminal.restore_cursor(),
            Action::ModifyKeyFormat(v) => {
                self.terminal.flags.modify_other_keys_2 = false;
                if v == ModifyKeyFormat::OtherKeysNumeric {
                    self.terminal.flags.modify_other_keys_2 = true;
                }
            }
            Action::ProtectedModeOff => self.terminal.set_protected_mode(ProtectedMode::Off),
            Action::ProtectedModeIso => self.terminal.set_protected_mode(ProtectedMode::Iso),
            Action::ProtectedModeDec => self.terminal.set_protected_mode(ProtectedMode::Dec),
            Action::MouseShiftCapture(v) => {
                self.terminal.flags.mouse_shift_capture = if v {
                    MouseShiftCapture::True
                } else {
                    MouseShiftCapture::False
                };
            }
            Action::SizeReport(v) => self.send_size_report(v),
            Action::Xtversion => self.report_xtversion(),
            Action::DeviceAttributes(v) => self.device_attributes(v),
            Action::DeviceStatus(v) => self.device_status_report(v.request),
            Action::KittyKeyboardQuery => self.query_kitty_keyboard(),
            Action::KittyKeyboardPush(v) => {
                log::debug!("pushing kitty keyboard mode: {:?}", v.flags);
                self.terminal.screen_mut().kitty_keyboard.push(v.flags);
            }
            Action::KittyKeyboardPop(v) => {
                log::debug!("popping kitty keyboard mode n={v}");
                self.terminal
                    .screen_mut()
                    .kitty_keyboard
                    .pop(usize::from(v));
            }
            Action::KittyKeyboardSet(v) => {
                log::debug!("setting kitty keyboard mode: set {:?}", v.flags);
                self.terminal
                    .screen_mut()
                    .kitty_keyboard
                    .set(KittySetMode::Set, v.flags);
            }
            Action::KittyKeyboardSetOr(v) => {
                log::debug!("setting kitty keyboard mode: or {:?}", v.flags);
                self.terminal
                    .screen_mut()
                    .kitty_keyboard
                    .set(KittySetMode::Or, v.flags);
            }
            Action::KittyKeyboardSetNot(v) => {
                log::debug!("setting kitty keyboard mode: not {:?}", v.flags);
                self.terminal
                    .screen_mut()
                    .kitty_keyboard
                    .set(KittySetMode::Not, v.flags);
            }
            Action::KittyColorReport(v) => self.kitty_color_report(v),
            Action::ColorOperation(v) => self.color_operation(v.requests, v.terminator),
            Action::EndHyperlink => self.terminal.screen_mut().end_hyperlink(),
            Action::ActiveStatusDisplay(v) => self.terminal.status_display = v,
            Action::Decaln => self.terminal.decaln()?,
            Action::WindowTitle(v) => self.window_title(v.title),
            Action::ReportPwd(v) => self.report_pwd(v.url)?,
            Action::ShowDesktopNotification(v) => self.show_desktop_notification(v.title, v.body),
            Action::ProgressReport(v) => self.progress_report(v),
            Action::StartHyperlink(v) => self.terminal.screen_mut().start_hyperlink(v.uri, v.id)?,
            Action::ClipboardContents(v) => self.clipboard_contents(v.kind, v.data),
            Action::SemanticPrompt(v) => self.semantic_prompt(v)?,
            Action::MouseShape(v) => self.set_mouse_shape(v),
            Action::ConfigureCharset(v) => self.terminal.configure_charset(v.slot, v.charset),
            Action::SetAttribute(v) => {
                if let ghostty_vt::sgr::Attribute::Unknown(unk) = &v {
                    // We optimize for the happy path scenario here, since
                    // unknown/invalid SGRs aren't that common in the wild.
                    log::warn!("unimplemented or unknown SGR attribute: {unk:?}");
                } else if let Err(err) = self.terminal.set_attribute(v) {
                    log::warn!("error setting attribute: {err:?}");
                }
            }
            Action::DcsHook(v) => self.dcs_hook(&v),
            Action::DcsPut(v) => self.dcs_put(v),
            Action::DcsUnhook => self.dcs_unhook(),
            Action::ApcStart => self.st.apc.start(),
            Action::ApcEnd => self.apc_end(),
            Action::ApcPut(v) => self.st.apc.feed(v),

            // Unimplemented
            Action::TitlePush(_) | Action::TitlePop(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ghostty_terminal::terminal::Options;

    use super::*;

    /// Records surface messages.
    #[derive(Default)]
    struct Rec(Mutex<Vec<SurfaceMessage>>);

    impl SurfaceMailbox for Rec {
        fn push(&self, msg: SurfaceMessage, _t: Timeout) -> usize {
            let mut v = self.0.lock().unwrap();
            v.push(msg);
            v.len()
        }
    }

    struct Harness {
        term: Terminal,
        stream: TerminalStream,
        mailbox: Mailbox,
        surface: Arc<Rec>,
    }

    impl Harness {
        fn new() -> Harness {
            Self::with(|_| {})
        }

        fn with(f: impl FnOnce(&mut HandlerOptions)) -> Harness {
            let mailbox = Mailbox::new().unwrap();
            let surface = Arc::new(Rec::default());
            let mut opts = HandlerOptions {
                size: Size::default(),
                termio_mailbox: mailbox.clone(),
                surface_mailbox: surface.clone(),
                renderer_wakeup: Wakeup::new().unwrap(),
                default_cursor_style: CursorStyle::Block,
                default_cursor_blink: None,
                enquiry_response: Vec::new(),
                osc_color_report_format: OSCColorReportFormat::Bit16,
                clipboard_write: ClipboardAccess::Allow,
            };
            f(&mut opts);
            let mut term = Terminal::new(Options::new(80, 24)).unwrap();
            term.colors.foreground = DynamicRgb_init(255, 255, 255);
            term.colors.background = DynamicRgb_init(0, 0, 0);
            Harness {
                term,
                stream: TerminalStream::new(HandlerState::new(opts)),
                mailbox,
                surface,
            }
        }

        fn feed(&mut self, bytes: &[u8]) {
            self.stream.process(&mut self.term, bytes).unwrap();
        }

        /// All bytes written to the pty (concatenated write requests).
        fn written(&self) -> Vec<u8> {
            let mut out = Vec::new();
            while let Some(m) = self.mailbox.pop() {
                if let Message::Write(w) = m {
                    out.extend_from_slice(w.slice());
                }
            }
            out
        }

        fn termio_msgs(&self) -> Vec<Message> {
            std::iter::from_fn(|| self.mailbox.pop()).collect()
        }

        fn surface_msgs(&self) -> Vec<SurfaceMessage> {
            std::mem::take(&mut *self.surface.0.lock().unwrap())
        }
    }

    #[allow(non_snake_case)]
    fn DynamicRgb_init(r: u8, g: u8, b: u8) -> ghostty_vt::color::DynamicRgb {
        ghostty_vt::color::DynamicRgb::init(Rgb { r, g, b })
    }

    #[test]
    fn print_updates_terminal() {
        let mut h = Harness::new();
        h.feed(b"hello\x1b[31mred");
        assert_eq!("hellored", h.term.plain_string().unwrap());
    }

    #[test]
    fn parser_state_persists_across_calls() {
        let mut h = Harness::new();
        h.feed(b"ab\x1b[3");
        h.feed(b"1mc\xe2\x82");
        h.feed(b"\xacd");
        assert_eq!("abc\u{20ac}d", h.term.plain_string().unwrap());
    }

    #[test]
    fn da1_with_clipboard() {
        let mut h = Harness::new();
        h.feed(b"\x1b[c");
        assert_eq!(b"\x1b[?62;22;52c".as_slice(), h.written());
    }

    #[test]
    fn da1_clipboard_deny() {
        let mut h = Harness::with(|o| o.clipboard_write = ClipboardAccess::Deny);
        h.feed(b"\x1b[0c");
        assert_eq!(b"\x1b[?62;22c".as_slice(), h.written());
    }

    #[test]
    fn da2() {
        let mut h = Harness::new();
        h.feed(b"\x1b[>c");
        assert_eq!(b"\x1b[>1;10;0c".as_slice(), h.written());
    }

    #[test]
    fn dsr_operating_status() {
        let mut h = Harness::new();
        h.feed(b"\x1b[5n");
        assert_eq!(b"\x1b[0n".as_slice(), h.written());
    }

    #[test]
    fn dsr_cursor_position() {
        let mut h = Harness::new();
        h.feed(b"\x1b[3;5H\x1b[6n");
        assert_eq!(b"\x1b[3;5R".as_slice(), h.written());
    }

    #[test]
    fn dsr_cursor_position_origin_mode() {
        let mut h = Harness::new();
        h.feed(b"\x1b[5;20r\x1b[?6h\x1b[2;3H\x1b[6n");
        assert_eq!(b"\x1b[2;3R".as_slice(), h.written());
    }

    #[test]
    fn dsr_color_scheme_is_forced_report() {
        let mut h = Harness::new();
        h.feed(b"\x1b[?996n");
        let msgs = h.termio_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [Message::ColorSchemeReport { force: true }]
        ));
    }

    #[test]
    fn xtversion() {
        let mut h = Harness::new();
        h.feed(b"\x1b[>q");
        assert_eq!(b"\x1bP>|ghostty 1.3.1\x1b\\".as_slice(), h.written());
    }

    #[test]
    fn enquiry_response() {
        let mut h = Harness::with(|o| o.enquiry_response = b"ghostty!".to_vec());
        h.feed(b"\x05");
        assert_eq!(b"ghostty!".as_slice(), h.written());
    }

    #[test]
    fn enquiry_response_empty_writes_nothing_visible() {
        let mut h = Harness::new();
        h.feed(b"\x05");
        assert!(h.written().is_empty());
    }

    #[test]
    fn decrqm_known_and_unknown() {
        let mut h = Harness::new();
        // Wraparound (7) is on by default.
        h.feed(b"\x1b[?7$p");
        assert_eq!(b"\x1b[?7;1$y".as_slice(), h.written());
        h.feed(b"\x1b[?7l\x1b[?7$p");
        assert_eq!(b"\x1b[?7;2$y".as_slice(), h.written());
        h.feed(b"\x1b[?9999$p");
        assert_eq!(b"\x1b[?9999;0$y".as_slice(), h.written());
    }

    #[test]
    fn kitty_keyboard_query_and_push() {
        let mut h = Harness::new();
        h.feed(b"\x1b[?u");
        assert_eq!(b"\x1b[?0u".as_slice(), h.written());
        h.feed(b"\x1b[>1u\x1b[?u");
        assert_eq!(b"\x1b[?1u".as_slice(), h.written());
        h.feed(b"\x1b[<u\x1b[?u");
        assert_eq!(b"\x1b[?0u".as_slice(), h.written());
    }

    #[test]
    fn osc_color_query_16bit_and_8bit() {
        let mut h = Harness::new();
        h.feed(b"\x1b]10;?\x1b\\");
        assert_eq!(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\".as_slice(), h.written());
        h.feed(b"\x1b]11;?\x07");
        assert_eq!(b"\x1b]11;rgb:0000/0000/0000\x07".as_slice(), h.written());

        let mut h = Harness::with(|o| o.osc_color_report_format = OSCColorReportFormat::Bit8);
        h.feed(b"\x1b]10;?\x1b\\");
        assert_eq!(b"\x1b]10;rgb:ff/ff/ff\x1b\\".as_slice(), h.written());
    }

    #[test]
    fn osc_color_query_none_format_is_silent() {
        let mut h = Harness::with(|o| o.osc_color_report_format = OSCColorReportFormat::None);
        h.feed(b"\x1b]10;?\x1b\\");
        assert!(h.written().is_empty());
    }

    #[test]
    fn osc_color_query_cursor_falls_back_to_foreground() {
        let mut h = Harness::new();
        h.feed(b"\x1b]12;?\x1b\\");
        assert_eq!(b"\x1b]12;rgb:ffff/ffff/ffff\x1b\\".as_slice(), h.written());
    }

    #[test]
    fn osc_palette_query_and_set() {
        let mut h = Harness::new();
        h.feed(b"\x1b]4;1;rgb:12/34/56\x1b\\");
        let msgs = h.surface_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [SurfaceMessage::ColorChange(osc_color::ColoredTarget {
                target: Target::Palette(1),
                color: Rgb {
                    r: 0x12,
                    g: 0x34,
                    b: 0x56
                },
            })]
        ));
        assert!(h.term.flags.dirty.palette);
        h.feed(b"\x1b]4;1;?\x1b\\");
        assert_eq!(b"\x1b]4;1;rgb:1212/3434/5656\x1b\\".as_slice(), h.written());
    }

    #[test]
    fn osc_color_reset_notifies_surface() {
        let mut h = Harness::new();
        h.feed(b"\x1b]4;2;rgb:12/34/56\x1b\\");
        h.surface_msgs();
        h.feed(b"\x1b]104;2\x1b\\");
        let msgs = h.surface_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [SurfaceMessage::ColorChange(osc_color::ColoredTarget {
                target: Target::Palette(2),
                ..
            })]
        ));
        // Reset of the foreground reports the default.
        h.feed(b"\x1b]10;rgb:01/02/03\x1b\\");
        h.surface_msgs();
        h.feed(b"\x1b]110\x1b\\");
        let msgs = h.surface_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [SurfaceMessage::ColorChange(osc_color::ColoredTarget {
                target: Target::Dynamic(DynamicColor::Foreground),
                color: Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                },
            })]
        ));
    }

    #[test]
    fn kitty_color_query() {
        let mut h = Harness::new();
        h.feed(b"\x1b]21;foreground=?;background=?;cursor=?\x1b\\");
        assert_eq!(
            b"\x1b]21;foreground=rgb:ff/ff/ff;background=rgb:00/00/00;cursor=\x1b\\".as_slice(),
            h.written()
        );
    }

    #[test]
    fn kitty_color_set_and_reset() {
        let mut h = Harness::new();
        h.feed(b"\x1b]21;foreground=rgb:01/02/03\x1b\\");
        assert!(h.written().is_empty());
        assert_eq!(
            Some(Rgb { r: 1, g: 2, b: 3 }),
            h.term.colors.foreground.get()
        );
        h.feed(b"\x1b]21;foreground=\x1b\\");
        assert_eq!(
            Some(Rgb {
                r: 255,
                g: 255,
                b: 255
            }),
            h.term.colors.foreground.get()
        );
    }

    #[test]
    fn xtgettcap_known_and_unknown() {
        let mut h = Harness::new();
        // "TN" = 544E
        h.feed(b"\x1bP+q544E\x1b\\");
        assert_eq!(
            b"\x1bP1+r544E=787465726D2D67686F73747479\x1b\\".as_slice(),
            h.written()
        );
        // Unknown capability: no response.
        h.feed(b"\x1bP+q5A5A5A\x1b\\");
        assert!(h.written().is_empty());
        // Multiple, one unknown.
        h.feed(b"\x1bP+q5A5A5A;544E\x1b\\");
        assert_eq!(
            b"\x1bP1+r544E=787465726D2D67686F73747479\x1b\\".as_slice(),
            h.written()
        );
    }

    #[test]
    fn decrqss_sgr() {
        let mut h = Harness::new();
        h.feed(b"\x1bP$qm\x1b\\");
        assert_eq!(b"\x1bP1$r0m\x1b\\".as_slice(), h.written());
        h.feed(b"\x1b[1;31m\x1bP$qm\x1b\\");
        let w = h.written();
        assert!(
            w.starts_with(b"\x1bP1$r0;1;31m"),
            "{:?}",
            String::from_utf8_lossy(&w)
        );
        assert!(w.ends_with(b"m\x1b\\"));
    }

    #[test]
    fn decrqss_decscusr_decstbm_decslrm_invalid() {
        let mut h = Harness::new();
        // Default blinking block (Termio sets this default mode on init).
        h.term.modes.set(Mode::CursorBlinking, true);
        h.feed(b"\x1bP$q q\x1b\\");
        assert_eq!(b"\x1bP1$r1 q\x1b\\".as_slice(), h.written());
        h.feed(b"\x1b[6 q\x1bP$q q\x1b\\");
        assert_eq!(b"\x1bP1$r6 q\x1b\\".as_slice(), h.written());

        h.feed(b"\x1b[3;10r\x1bP$qr\x1b\\");
        assert_eq!(b"\x1bP1$r3;10r\x1b\\".as_slice(), h.written());

        // DECSLRM only valid when DECLRMM is enabled.
        h.feed(b"\x1bP$qs\x1b\\");
        assert_eq!(b"\x1bP0$r\x1b\\".as_slice(), h.written());
        h.feed(b"\x1b[?69h\x1b[2;5s\x1bP$qs\x1b\\");
        assert_eq!(b"\x1bP1$r2;5s\x1b\\".as_slice(), h.written());

        // Unknown request
        h.feed(b"\x1bP$qzz\x1b\\");
        assert_eq!(b"\x1bP0$r\x1b\\".as_slice(), h.written());
    }

    #[test]
    fn size_reports_are_forwarded() {
        let mut h = Harness::new();
        h.feed(b"\x1b[14t\x1b[16t\x1b[18t");
        let msgs = h.termio_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [
                Message::SizeReport(SizeReport::Csi14T),
                Message::SizeReport(SizeReport::Csi16T),
                Message::SizeReport(SizeReport::Csi18T)
            ]
        ));
        h.feed(b"\x1b[21t");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::ReportTitle(ReportTitleStyle::Csi21T)]
        ));
    }

    #[test]
    fn bell() {
        let mut h = Harness::new();
        h.feed(b"\x07");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::RingBell]
        ));
    }

    #[test]
    fn window_title_and_empty_title_reset() {
        let mut h = Harness::new();
        h.feed(b"\x1b]2;hello\x1b\\");
        let msgs = h.surface_msgs();
        let [SurfaceMessage::SetTitle(buf)] = msgs.as_slice() else {
            panic!("{msgs:?}")
        };
        assert_eq!(&buf[..6], b"hello\0");
        assert!(h.stream.state.seen_title);

        // Empty title resets: with a pwd it becomes the pwd.
        h.term.set_pwd(b"/tmp/x").unwrap();
        h.feed(b"\x1b]2;\x1b\\");
        let msgs = h.surface_msgs();
        let [SurfaceMessage::SetTitle(buf)] = msgs.as_slice() else {
            panic!("{msgs:?}")
        };
        assert_eq!(&buf[..7], b"/tmp/x\0");
        assert!(!h.stream.state.seen_title);
    }

    #[test]
    fn window_title_too_long_is_ignored() {
        let mut h = Harness::new();
        let mut seq = b"\x1b]2;".to_vec();
        seq.extend(std::iter::repeat_n(b'a', 300));
        seq.extend_from_slice(b"\x1b\\");
        h.feed(&seq);
        assert!(h.surface_msgs().is_empty());
    }

    #[test]
    fn report_pwd_local_host() {
        let mut h = Harness::new();
        h.feed(b"\x1b]7;file://localhost/tmp/a%20b\x1b\\");
        let msgs = h.surface_msgs();
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        let SurfaceMessage::PwdChange(w) = &msgs[0] else {
            panic!("{msgs:?}")
        };
        assert_eq!(w.slice(), b"/tmp/a b");
        // The pwd becomes the title when no title was seen.
        assert!(matches!(&msgs[1], SurfaceMessage::SetTitle(t) if t.starts_with(b"/tmp/a b\0")));
        assert_eq!(Some(b"/tmp/a b".as_slice()), h.term.get_pwd());
        assert!(!h.stream.state.seen_title);
    }

    #[test]
    fn report_pwd_remote_host_ignored_and_reset() {
        let mut h = Harness::new();
        h.feed(b"\x1b]7;file://definitely-not-this-host.invalid/tmp\x1b\\");
        assert!(h.surface_msgs().is_empty());
        assert!(h.term.get_pwd().is_none());
        h.feed(b"\x1b]7;http://localhost/tmp\x1b\\");
        assert!(h.surface_msgs().is_empty());

        h.feed(b"\x1b]7;file://localhost/tmp\x1b\\");
        h.surface_msgs();
        h.feed(b"\x1b]7;\x1b\\");
        let msgs = h.surface_msgs();
        assert!(
            msgs.iter()
                .any(|m| matches!(m, SurfaceMessage::PwdChange(w) if w.slice().is_empty())),
            "{msgs:?}"
        );
        assert!(h.term.get_pwd().is_none());
    }

    #[test]
    fn clipboard_write_and_read() {
        let mut h = Harness::new();
        h.feed(b"\x1b]52;c;aGVsbG8=\x1b\\");
        let msgs = h.surface_msgs();
        let [
            SurfaceMessage::ClipboardWrite {
                clipboard_type,
                req,
            },
        ] = msgs.as_slice()
        else {
            panic!("{msgs:?}")
        };
        assert_eq!(*clipboard_type, Clipboard::Standard);
        assert_eq!(req.slice(), b"aGVsbG8=");

        h.feed(b"\x1b]52;p;?\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::ClipboardRead(Clipboard::Primary)]
        ));
        h.feed(b"\x1b]52;s;?\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::ClipboardRead(Clipboard::Selection)]
        ));
    }

    #[test]
    fn desktop_notification_and_progress() {
        let mut h = Harness::new();
        h.feed(b"\x1b]777;notify;Title;Body text\x1b\\");
        let msgs = h.surface_msgs();
        let [SurfaceMessage::DesktopNotification(n)] = msgs.as_slice() else {
            panic!("{msgs:?}")
        };
        assert_eq!(n.title(), b"Title");
        assert_eq!(n.body(), b"Body text");

        h.feed(b"\x1b]9;4;1;50\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::ProgressReport(ProgressReport {
                state: ProgressState::Set,
                progress: Some(50)
            })]
        ));
    }

    #[test]
    fn full_reset_removes_progress_and_reports_scheme() {
        let mut h = Harness::new();
        h.feed(b"\x1b[?1000h\x1bc");
        let surface = h.surface_msgs();
        assert!(surface.iter().any(|m| matches!(
            m,
            SurfaceMessage::ProgressReport(ProgressReport {
                state: ProgressState::Remove,
                ..
            })
        )));
        assert!(
            h.termio_msgs()
                .iter()
                .any(|m| matches!(m, Message::ColorSchemeReport { force: false }))
        );
    }

    #[test]
    fn semantic_prompt_commands() {
        let mut h = Harness::new();
        h.feed(b"\x1b]133;C\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::StartCommand]
        ));
        h.feed(b"\x1b]133;D;1\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::StopCommand(Some(1))]
        ));
        h.feed(b"\x1b]133;D;300\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::StopCommand(Some(1))]
        ));
        h.feed(b"\x1b]133;D\x1b\\");
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::StopCommand(Some(0))]
        ));
    }

    #[test]
    fn mode_side_effects_are_messaged() {
        let mut h = Harness::new();
        h.feed(b"\x1b[?2026h");
        assert!(matches!(
            h.termio_msgs().as_slice(),
            [Message::StartSynchronizedOutput]
        ));
        h.feed(b"\x1b[?2026l");
        assert!(h.termio_msgs().is_empty());

        h.feed(b"\x1b[20h");
        // Linefeed mode (20) is an ANSI mode.
        assert!(matches!(
            h.termio_msgs().as_slice(),
            [Message::LinefeedMode(true)]
        ));

        h.feed(b"\x1b[?2048h");
        assert!(matches!(
            h.termio_msgs().as_slice(),
            [Message::SizeReport(SizeReport::Mode2048)]
        ));

        h.term.flags.focused = false;
        h.feed(b"\x1b[?1004h");
        assert!(matches!(
            h.termio_msgs().as_slice(),
            [Message::Focused(false)]
        ));
    }

    #[test]
    fn mouse_modes_set_shape_once() {
        let mut h = Harness::new();
        // Text is not the default shape, so the first enable emits Default.
        h.feed(b"\x1b[?1000h");
        assert_eq!(h.term.flags.mouse_event, MouseEvents::Normal);
        let msgs = h.surface_msgs();
        assert!(matches!(
            msgs.as_slice(),
            [] | [SurfaceMessage::SetMouseShape(MouseShape::Default)]
        ));
        h.feed(b"\x1b[?1000l");
        assert_eq!(h.term.flags.mouse_event, MouseEvents::None);
        assert!(matches!(
            h.surface_msgs().as_slice(),
            [SurfaceMessage::SetMouseShape(MouseShape::Text)]
        ));
        // Same shape again: no message.
        h.feed(b"\x1b[?1000l");
        assert!(h.surface_msgs().is_empty());
        h.feed(b"\x1b[?1006h");
        assert_eq!(h.term.flags.mouse_format, MouseFormat::Sgr);
    }

    #[test]
    fn cursor_style_default_and_explicit() {
        let mut h = Harness::with(|o| {
            o.default_cursor_style = CursorStyle::Bar;
            o.default_cursor_blink = Some(false);
        });
        h.feed(b"\x1b[3 q");
        assert_eq!(CursorStyle::Underline, h.term.screen().cursor.cursor_style);
        assert!(h.term.modes.get(Mode::CursorBlinking));
        assert!(!h.stream.state.default_cursor);

        // CSI 0 q goes back to the configured default.
        h.feed(b"\x1b[0 q");
        assert_eq!(CursorStyle::Bar, h.term.screen().cursor.cursor_style);
        assert!(!h.term.modes.get(Mode::CursorBlinking));
        assert!(h.stream.state.default_cursor);
    }

    #[test]
    fn mode_12_ignored_when_default_blink_configured() {
        let mut h = Harness::with(|o| o.default_cursor_blink = Some(true));
        h.feed(b"\x1b[?12l");
        assert!(h.term.modes.get(Mode::CursorBlinking) || !h.term.modes.get(Mode::CursorBlinking));
        let before = h.term.modes.get(Mode::CursorBlinking);
        h.feed(b"\x1b[?12h\x1b[?12l");
        assert_eq!(before, h.term.modes.get(Mode::CursorBlinking));

        let mut h = Harness::new();
        h.feed(b"\x1b[?12h");
        assert!(h.term.modes.get(Mode::CursorBlinking));
        h.feed(b"\x1b[?12l");
        assert!(!h.term.modes.get(Mode::CursorBlinking));
    }

    #[test]
    fn change_config_updates_default_cursor_and_requests_scheme_report() {
        let mut h = Harness::new();
        let mut cfg = crate::termio::DerivedConfig::new(&ghostty_config::config::Config::default());
        cfg.cursor_style = CursorStyle::Underline;
        cfg.cursor_blink = Some(false);
        cfg.enquiry_response = "hi".into();
        h.stream.change_config(&mut h.term, &cfg);
        assert_eq!(CursorStyle::Underline, h.term.screen().cursor.cursor_style);
        assert!(!h.term.modes.get(Mode::CursorBlinking));
        assert!(
            h.termio_msgs()
                .iter()
                .any(|m| matches!(m, Message::ColorSchemeReport { force: false }))
        );
        h.feed(b"\x05");
        assert_eq!(b"hi".as_slice(), h.written());
    }

    #[test]
    fn full_mailbox_spills_to_outbox_preserving_order() {
        let mut h = Harness::new();
        for _ in 0..crate::mailbox::CAPACITY {
            h.mailbox.try_send(Message::Crash).unwrap();
        }
        h.feed(b"\x1b[5n\x1b[c");
        assert!(h.stream.state.has_outbox());
        // Nothing was lost or reordered: drain the queue, then flush.
        let mut seen = 0;
        while h.mailbox.pop().is_some() {
            seen += 1;
        }
        assert_eq!(seen, crate::mailbox::CAPACITY);
        h.stream.flush_outbox();
        assert_eq!(b"\x1b[0n\x1b[?62;22;52c".as_slice(), h.written());
        assert!(!h.stream.state.has_outbox());
    }

    #[test]
    fn decaln_and_erase_display_still_apply() {
        let mut h = Harness::new();
        h.feed(b"abc\x1b#8");
        assert!(h.term.plain_string().unwrap().starts_with('E'));
        h.feed(b"\x1b[2J");
        assert_eq!("", h.term.plain_string().unwrap());
    }

    #[test]
    fn dcs_tmux_enter_creates_viewer_and_exit_drops_it() {
        let mut h = Harness::new();
        h.feed(b"\x1bP1000p%begin 1 2 3\n%end 1 2 3\n");
        assert!(h.stream.state.tmux_viewer.is_some());
        h.feed(b"%exit\n\x1b\\after");
        assert!(h.stream.state.tmux_viewer.is_none());
        assert!(h.term.plain_string().unwrap().contains("after"));
    }

    #[test]
    fn dcs_tmux_is_ignored_without_panicking() {
        let mut h = Harness::new();
        h.feed(b"\x1bP1000p%begin 1 2 3\n%end 1 2 3\n\x1b\\after");
        assert!(h.term.plain_string().unwrap().contains("after"));
    }

    #[test]
    fn kitty_graphics_query_is_harmless() {
        let mut h = Harness::new();
        h.feed(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\x");
        assert!(h.term.plain_string().unwrap().contains('x'));
    }
}
