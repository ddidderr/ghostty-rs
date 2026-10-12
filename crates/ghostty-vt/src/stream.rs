//! The stream layer (ZIG/src/terminal/stream.zig).
//!
//! A [`Stream`] feeds bytes through the UTF-8 decoder and the VT [`Parser`],
//! decodes the resulting CSI/ESC/OSC/DCS/APC actions into high level
//! [`Action`]s and hands them to a [`Handler`].
//!
//! Rust port notes:
//! - The Zig comptime interface `handler.vt(comptime action, value)` is the
//!   [`Handler::vt`] method taking the [`Action`] enum. [`ActionKey`] is the
//!   Zig `Action.Key` (the tag without payload), see [`Action::key`].
//! - Zig `!void` becomes `Result<(), H::Error>`; errors returned by the
//!   handler abort processing and propagate out of `next`/`next_slice`.
//! - Zig's optional `vtRaw` is [`Handler::vt_raw`] (default: `Ok(false)`).
//! - Zig's `deinit` is `Drop` on the handler.
//! - The SIMD fast path is a scalar path built on `crate::simd`.
//! - Payloads borrow from the parser's owned OSC command (`'a`); handlers that
//!   need to keep data must copy it.

#![allow(clippy::single_match_else, clippy::if_not_else)]

use crate::{
    ansi,
    charsets,
    csi,
    device_status,
    kitty,
    modes,
    mouse_shape::MouseShape,
    osc,
    parser::{self, Csi, Esc, MAX_PARAMS, Parser, State},
    sgr,
    simd,
    utf8_decoder::Utf8Decoder,
};

macro_rules! define_actions {
    ($( $(#[$meta:meta])* $name:ident $( ( $ty:ty ) )? ),* $(,)?) => {
        /// The possible actions that can be emitted by the [`Stream`] for
        /// handling (Zig `Action`).
        #[derive(Debug, Clone, PartialEq)]
        pub enum Action<'a> {
            $( $(#[$meta])* $name $( ( $ty ) )? ),*
        }

        /// The tag of an [`Action`] (Zig `Action.Key`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ActionKey {
            $( $name ),*
        }

        impl Action<'_> {
            /// The key (tag) of this action.
            pub fn key(&self) -> ActionKey {
                match self {
                    $( Action::$name { .. } => ActionKey::$name ),*
                }
            }
        }
    };
}

define_actions! {
    Print(Print),
    PrintRepeat(usize),
    Bell,
    Backspace,
    HorizontalTab(u16),
    HorizontalTabBack(u16),
    Linefeed,
    CarriageReturn,
    Enquiry,
    InvokeCharset(InvokeCharset),
    CursorUp(CursorMovement),
    CursorDown(CursorMovement),
    CursorLeft(CursorMovement),
    CursorRight(CursorMovement),
    CursorCol(CursorMovement),
    CursorRow(CursorMovement),
    CursorColRelative(CursorMovement),
    CursorRowRelative(CursorMovement),
    CursorPos(CursorPos),
    CursorStyle(ansi::CursorStyle),
    /// The bool payload of the erase actions is "protected".
    EraseDisplayBelow(bool),
    EraseDisplayAbove(bool),
    EraseDisplayComplete(bool),
    EraseDisplayScrollback(bool),
    EraseDisplayScrollComplete(bool),
    EraseLineRight(bool),
    EraseLineLeft(bool),
    EraseLineComplete(bool),
    EraseLineRightUnlessPendingWrap(bool),
    DeleteChars(usize),
    EraseChars(usize),
    InsertLines(usize),
    InsertBlanks(usize),
    DeleteLines(usize),
    ScrollUp(usize),
    ScrollDown(usize),
    TabClearCurrent,
    TabClearAll,
    TabSet,
    TabReset,
    Index,
    NextLine,
    ReverseIndex,
    FullReset,
    SetMode(Mode),
    ResetMode(Mode),
    SaveMode(Mode),
    RestoreMode(Mode),
    RequestMode(Mode),
    RequestModeUnknown(RawMode),
    TopAndBottomMargin(Margin),
    LeftAndRightMargin(Margin),
    LeftAndRightMarginAmbiguous,
    SaveCursor,
    RestoreCursor,
    ModifyKeyFormat(ansi::ModifyKeyFormat),
    MouseShiftCapture(bool),
    ProtectedModeOff,
    ProtectedModeIso,
    ProtectedModeDec,
    SizeReport(csi::SizeReportStyle),
    TitlePush(u16),
    TitlePop(u16),
    Xtversion,
    DeviceAttributes(ansi::DeviceAttributeReq),
    DeviceStatus(DeviceStatus),
    KittyKeyboardQuery,
    KittyKeyboardPush(KittyKeyboardFlags),
    KittyKeyboardPop(u16),
    KittyKeyboardSet(KittyKeyboardFlags),
    KittyKeyboardSetOr(KittyKeyboardFlags),
    KittyKeyboardSetNot(KittyKeyboardFlags),
    DcsHook(parser::Dcs),
    DcsPut(u8),
    DcsUnhook,
    ApcStart,
    ApcEnd,
    ApcPut(u8),
    EndHyperlink,
    ActiveStatusDisplay(ansi::StatusDisplay),
    Decaln,
    WindowTitle(WindowTitle<'a>),
    ReportPwd(ReportPwd<'a>),
    ShowDesktopNotification(ShowDesktopNotification<'a>),
    ProgressReport(osc::ProgressReport),
    StartHyperlink(StartHyperlink<'a>),
    ClipboardContents(ClipboardContents<'a>),
    MouseShape(MouseShape),
    ConfigureCharset(ConfigureCharset),
    SetAttribute(sgr::Attribute<'a>),
    KittyColorReport(&'a kitty::color::OSC<'a>),
    ColorOperation(ColorOperation<'a>),
    SemanticPrompt(&'a osc::SemanticPrompt<'a>),
}

/// Field types of [`Action`] payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Print {
    pub cp: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvokeCharset {
    pub bank: charsets::ActiveSlot,
    pub charset: charsets::Slots,
    pub locking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorMovement {
    /// The value of the cursor movement. Depending on the action this may be
    /// an absolute value or a relative value. For example, `CursorUp` is
    /// relative, but `CursorRow` is absolute.
    pub value: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStatus {
    pub request: device_status::Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub mode: modes::Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMode {
    pub mode: u16,
    pub ansi: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margin {
    pub top_left: u16,
    pub bottom_right: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KittyKeyboardFlags {
    pub flags: kitty::key::Flags,
}

/// Always valid UTF-8 (the stream rejects invalid titles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTitle<'a> {
    pub title: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportPwd<'a> {
    pub url: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowDesktopNotification<'a> {
    pub title: &'a [u8],
    pub body: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartHyperlink<'a> {
    pub uri: &'a [u8],
    pub id: Option<&'a [u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardContents<'a> {
    pub kind: u8,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureCharset {
    pub slot: charsets::Slots,
    pub charset: charsets::Charset,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorOperation<'a> {
    pub op: osc::color::Operation,
    pub requests: &'a osc::color::List<'a>,
    pub terminator: osc::Terminator,
}

/// Alias matching Zig `Action.SemanticPrompt`.
pub type SemanticPrompt<'a> = osc::SemanticPrompt<'a>;

/// The receiver of [`Action`]s (Zig: a type with a
/// `vt(comptime action, value) !void` method).
///
/// The handler can choose to react to whatever actions it cares about and
/// ignore the rest with a `_ => {}` arm.
pub trait Handler {
    /// Error returned by handler methods (Zig `!void` error set). Errors
    /// abort processing of the current `next`/`next_slice` call.
    type Error: std::fmt::Debug;

    /// Handle one high level action.
    fn vt(&mut self, action: Action<'_>) -> Result<(), Self::Error>;

    /// A handler can use this to get the raw parser action before it is
    /// further decoded (Zig `vtRaw`). If this returns `Ok(true)` the stream
    /// skips processing the action itself. Errors are logged and the action
    /// is skipped.
    fn vt_raw(&mut self, action: &parser::Action) -> Result<bool, Self::Error> {
        let _ = action;
        Ok(false)
    }
}

/// A mutable reference to a handler is a handler (Zig allows pointer
/// handlers).
impl<H: Handler + ?Sized> Handler for &mut H {
    type Error = H::Error;

    fn vt(&mut self, action: Action<'_>) -> Result<(), Self::Error> {
        (**self).vt(action)
    }

    fn vt_raw(&mut self, action: &parser::Action) -> Result<bool, Self::Error> {
        (**self).vt_raw(action)
    }
}

/// Maximum number of codepoints decoded at one time in `next_slice`.
const CP_BUF_LEN: usize = 4096;

/// Processes a stream of tty control characters and calls the handler.
pub struct Stream<H: Handler> {
    pub handler: H,
    pub parser: Parser,
    pub utf8decoder: Utf8Decoder,
}

type Res<H> = Result<(), <H as Handler>::Error>;

/// `0 => 1, 1 => params[0], else => None`, the most common CSI param form.
#[inline]
fn count_param(params: &[u16]) -> Option<u16> {
    match params.len() {
        0 => Some(1),
        1 => Some(params[0]),
        _ => None,
    }
}

impl<H: Handler> Stream<H> {
    /// Initialize an allocation-free stream (Zig `init`). Things that need
    /// heap allocation in Zig (e.g. OSC 52 and the color OSCs) are rejected
    /// like in Zig. Use [`Stream::new_alloc`] for full behavior.
    pub fn new(h: H) -> Self {
        Self {
            handler: h,
            parser: Parser::new(),
            utf8decoder: Utf8Decoder::new(),
        }
    }

    /// Initialize a stream that supports "heap allocation" as necessary
    /// (Zig `initAlloc`): sets `parser.osc_parser.alloc = true`.
    pub fn new_alloc(h: H) -> Self {
        let mut s = Self::new(h);
        s.parser.osc_parser.alloc = true;
        s
    }

    /// Process a string of characters.
    pub fn next_slice(&mut self, input: &[u8]) -> Res<H> {
        let mut cp_buf = [0u32; CP_BUF_LEN];
        for chunk in input.chunks(CP_BUF_LEN) {
            self.next_slice_capped(chunk, &mut cp_buf)?;
        }
        Ok(())
    }

    fn next_slice_capped(&mut self, input: &[u8], cp_buf: &mut [u32]) -> Res<H> {
        debug_assert!(input.len() <= cp_buf.len());

        let mut offset = 0;

        // If the scalar UTF-8 decoder was in the middle of processing a code
        // sequence, we continue until it's not.
        while self.utf8decoder.in_sequence() {
            if offset >= input.len() {
                return Ok(());
            }
            self.next_utf8(input[offset])?;
            offset += 1;
        }
        if offset >= input.len() {
            return Ok(());
        }

        // If we're not in the ground state then we process until we are. This
        // can happen if the last chunk of input put us in the middle of a
        // control sequence.
        offset += self.consume_until_ground(&input[offset..])?;
        if offset >= input.len() {
            return Ok(());
        }
        offset += self.consume_all_escapes(&input[offset..])?;

        // If we're in the ground state then we can use SIMD to process input
        // until we see an ESC (0x1B), since all other characters up to that
        // point are just UTF-8.
        while self.parser.state == State::Ground && offset < input.len() {
            let res = simd::utf8_decode_until_control_seq(&input[offset..], cp_buf);
            for &cp in &cp_buf[..res.decoded] {
                if cp <= 0xF {
                    self.execute(cp as u8)?;
                } else {
                    self.print(cp)?;
                }
            }
            // Consume the bytes we just processed.
            offset += res.consumed;

            if offset >= input.len() {
                return Ok(());
            }

            // If our offset is NOT an escape then we must have a partial
            // UTF-8 sequence. In that case, we pass the whole remainder off
            // to the scalar parser.
            if input[offset] != 0x1B {
                for &c in &input[offset..] {
                    self.next_utf8(c)?;
                }
                return Ok(());
            }

            // Process control sequences until we run out.
            offset += self.consume_all_escapes(&input[offset..])?;
        }
        Ok(())
    }

    /// Parses back-to-back escape sequences until none are left. Returns the
    /// number of bytes consumed from the provided input.
    ///
    /// Expects input to start with 0x1B, use `consume_until_ground` first if
    /// the stream may be in the middle of an escape sequence.
    fn consume_all_escapes(&mut self, input: &[u8]) -> Result<usize, H::Error> {
        let mut offset = 0;
        while input[offset] == 0x1B {
            self.parser.state = State::Escape;
            self.parser.clear();
            offset += 1;
            offset += self.consume_until_ground(&input[offset..])?;
            if offset >= input.len() {
                return Ok(input.len());
            }
        }
        Ok(offset)
    }

    /// Parses escape sequences until the parser reaches the ground state.
    /// Returns the number of bytes consumed from the provided input.
    fn consume_until_ground(&mut self, input: &[u8]) -> Result<usize, H::Error> {
        let mut offset = 0;
        while self.parser.state != State::Ground {
            if offset >= input.len() {
                return Ok(input.len());
            }
            self.next_non_utf8(input[offset])?;
            offset += 1;
        }
        Ok(offset)
    }

    /// Like `next_slice` but takes one byte and is necessarily a scalar
    /// operation. Prefer `next_slice` if you can.
    #[inline]
    pub fn next(&mut self, c: u8) -> Res<H> {
        // The scalar path can be responsible for decoding UTF-8.
        if self.parser.state == State::Ground {
            return self.next_utf8(c);
        }
        self.next_non_utf8(c)
    }

    /// Process the next byte and print as necessary. Assumes the parser is in
    /// the ground state.
    #[inline]
    fn next_utf8(&mut self, c: u8) -> Res<H> {
        debug_assert_eq!(self.parser.state, State::Ground);

        let (cp, consumed) = self.utf8decoder.next(c);
        if let Some(cp) = cp {
            self.handle_codepoint(cp)?;
        }
        if !consumed {
            // Ill-formed sequence: replacement was emitted, retry the byte.
            let (cp, consumed) = self.utf8decoder.next(c);
            // It should be impossible for the decoder to not consume the
            // byte twice in a row.
            debug_assert!(consumed);
            if let Some(cp) = cp {
                self.handle_codepoint(cp)?;
            }
        }
        Ok(())
    }

    /// To be called whenever the utf-8 decoder produces a codepoint.
    ///
    /// This handles the case where the decoder emits a 0x1B after rejecting
    /// an ill-formed sequence.
    #[inline]
    fn handle_codepoint(&mut self, c: u32) -> Res<H> {
        // C0 control
        if c <= 0xF {
            return self.execute(c as u8);
        }
        // ESC
        if c == 0x1B {
            self.parser.state = State::Escape;
            self.parser.clear();
            return Ok(());
        }
        self.print(c)
    }

    /// Process the next character and call any callbacks if necessary.
    /// Assumes the parser is not in the ground state.
    fn next_non_utf8(&mut self, c: u8) -> Res<H> {
        debug_assert_ne!(self.parser.state, State::Ground);

        // Fast path for CSI entry.
        if self.parser.state == State::Escape && c == b'[' {
            self.parser.state = State::CsiEntry;
            return Ok(());
        }

        // Fast path for CSI params.
        if self.parser.state == State::CsiParam {
            // csi_param is the most common parser state other than ground.
            match c {
                // A C0 escape (yes, this is valid):
                0x00..=0x0F => {
                    self.execute(c)?;
                    return Ok(());
                }
                // We ignore C0 escapes > 0xF since execute doesn't have
                // processing for them anyway:
                0x10..=0x17 | 0x19 | 0x1C..=0x1F => return Ok(()),
                // We don't currently have any handling for 0x18 or 0x1A, but
                // they should still move the parser state to ground.
                0x18 | 0x1A => {
                    self.parser.state = State::Ground;
                    return Ok(());
                }
                // A parameter digit:
                b'0'..=b'9' => {
                    if (self.parser.params_idx as usize) < MAX_PARAMS {
                        self.parser.param_acc = self.parser.param_acc.saturating_mul(10);
                        self.parser.param_acc =
                            self.parser.param_acc.saturating_add(u16::from(c - b'0'));
                        // The parser's CSI param action uses param_acc_idx to
                        // decide if there's a final param that needs to be
                        // consumed or not, but it doesn't matter really what
                        // it is as long as it's not 0.
                        self.parser.param_acc_idx |= 1;
                    }
                    return Ok(());
                }
                // A parameter separator:
                b':' | b';' => {
                    if (self.parser.params_idx as usize) < MAX_PARAMS {
                        let idx = self.parser.params_idx as usize;
                        self.parser.params[idx] = self.parser.param_acc;
                        if c == b':' {
                            self.parser.params_sep.set(idx);
                        }
                        self.parser.params_idx += 1;

                        self.parser.param_acc = 0;
                        self.parser.param_acc_idx = 0;
                    }
                    return Ok(());
                }
                // Explicitly ignored:
                0x7F => return Ok(()),
                // Defer to the state machine for any other characters.
                _ => {}
            }
        }

        let actions = self.parser.next(c);
        for action in actions.into_iter().flatten() {
            // A handler can expose this to get the raw action before it is
            // further parsed. If this returns `true` we skip processing.
            match self.handler.vt_raw(&action) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!("error handling action manually err={err:?} action={action:?}");
                    // Always skip erroneous actions because we can't be sure...
                    continue;
                }
            }

            match action {
                parser::Action::Print(p) => self.print(p)?,
                parser::Action::Execute(code) => self.execute(code)?,
                parser::Action::CsiDispatch(csi_action) => self.csi_dispatch(&csi_action)?,
                parser::Action::EscDispatch(esc) => self.esc_dispatch(esc)?,
                parser::Action::OscDispatch(cmd) => self.osc_dispatch(&cmd)?,
                parser::Action::DcsHook(dcs) => self.handler.vt(Action::DcsHook(dcs))?,
                parser::Action::DcsPut(code) => self.handler.vt(Action::DcsPut(code))?,
                parser::Action::DcsUnhook => self.handler.vt(Action::DcsUnhook)?,
                parser::Action::ApcStart => self.handler.vt(Action::ApcStart)?,
                parser::Action::ApcPut(code) => self.handler.vt(Action::ApcPut(code))?,
                parser::Action::ApcEnd => self.handler.vt(Action::ApcEnd)?,
            }
        }
        Ok(())
    }

    #[inline]
    pub fn print(&mut self, c: u32) -> Res<H> {
        self.handler.vt(Action::Print(Print { cp: c }))
    }

    pub fn execute(&mut self, c: u8) -> Res<H> {
        // If the character is > 0x7F, it's a C1 (8-bit) control, which is
        // strictly equivalent to `ESC` plus `c - 0x40`.
        if c > 0x7F {
            log::info!("executing C1 0x{:x} as ESC {}", c, (c - 0x40) as char);
            return self.esc_dispatch(Esc::new(&[], c - 0x40));
        }

        match c {
            // We ignore SOH/STX: https://github.com/microsoft/terminal/issues/10786
            0x00..=0x02 => Ok(()),
            0x05 => self.handler.vt(Action::Enquiry),
            0x07 => self.handler.vt(Action::Bell),
            0x08 => self.handler.vt(Action::Backspace),
            0x09 => self.handler.vt(Action::HorizontalTab(1)),
            0x0A..=0x0C => self.handler.vt(Action::Linefeed),
            0x0D => self.handler.vt(Action::CarriageReturn),
            0x0E => self.handler.vt(Action::InvokeCharset(InvokeCharset {
                bank: charsets::ActiveSlot::GL,
                charset: charsets::Slots::G1,
                locking: false,
            })),
            0x0F => self.handler.vt(Action::InvokeCharset(InvokeCharset {
                bank: charsets::ActiveSlot::GL,
                charset: charsets::Slots::G0,
                locking: false,
            })),
            _ => {
                log::warn!("invalid C0 character, ignoring: 0x{c:x}");
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
    fn csi_dispatch(&mut self, input: &Csi) -> Res<H> {
        let params = input.params();
        let inter = input.intermediates();

        macro_rules! unimpl_inter {
            ($c:literal) => {
                log::warn!(
                    "ignoring unimplemented CSI {} with intermediates: {}",
                    $c,
                    String::from_utf8_lossy(inter)
                )
            };
        }

        // `CSI <count> <final>` forms with no intermediates where
        // the result maps to an action using the given constructor.
        macro_rules! counted {
            ($c:literal, $name:literal, $make:expr) => {{
                if inter.is_empty() {
                    match count_param(params) {
                        Some(v) => {
                            let make = $make;
                            self.handler.vt(make(v))
                        }
                        None => {
                            log::warn!("invalid {} command: {}", $name, input);
                            Ok(())
                        }
                    }
                } else {
                    unimpl_inter!($c);
                    Ok(())
                }
            }};
        }

        match input.final_ {
            // CUU - Cursor Up
            b'A' | b'k' => counted!("A", "cursor up", |v| Action::CursorUp(CursorMovement {
                value: v
            })),
            // CUD - Cursor Down
            b'B' => counted!("B", "cursor down", |v| Action::CursorDown(CursorMovement {
                value: v
            })),
            // CUF - Cursor Right
            b'C' => counted!("C", "cursor right", |v| Action::CursorRight(
                CursorMovement { value: v }
            )),
            // CUB - Cursor Left
            b'D' | b'j' => counted!("D", "cursor left", |v| Action::CursorLeft(CursorMovement {
                value: v
            })),

            // CNL - Cursor Next Line
            b'E' => {
                if !inter.is_empty() {
                    unimpl_inter!("E");
                    return Ok(());
                }
                let Some(v) = count_param(params) else {
                    log::warn!("invalid cursor up command: {input}");
                    return Ok(());
                };
                self.handler
                    .vt(Action::CursorDown(CursorMovement { value: v }))?;
                self.handler.vt(Action::CarriageReturn)
            }

            // CPL - Cursor Previous Line
            b'F' => {
                if !inter.is_empty() {
                    unimpl_inter!("F");
                    return Ok(());
                }
                let Some(v) = count_param(params) else {
                    log::warn!("invalid cursor down command: {input}");
                    return Ok(());
                };
                self.handler
                    .vt(Action::CursorUp(CursorMovement { value: v }))?;
                self.handler.vt(Action::CarriageReturn)
            }

            // HPA - Cursor Horizontal Position Absolute
            b'G' | b'`' => counted!("G", "HPA", |v| Action::CursorCol(CursorMovement {
                value: v
            })),

            // CUP - Set Cursor Position.
            b'H' | b'f' => {
                if !inter.is_empty() {
                    unimpl_inter!("H");
                    return Ok(());
                }
                let pos = match params.len() {
                    0 => CursorPos { row: 1, col: 1 },
                    1 => CursorPos {
                        row: params[0],
                        col: 1,
                    },
                    2 => CursorPos {
                        row: params[0],
                        col: params[1],
                    },
                    _ => {
                        log::warn!("invalid CUP command: {input}");
                        return Ok(());
                    }
                };
                self.handler.vt(Action::CursorPos(pos))
            }

            // CHT - Cursor Horizontal Tabulation
            b'I' => counted!("I", "horizontal tab", Action::HorizontalTab),

            // Erase Display
            b'J' => {
                let protected = match inter.len() {
                    0 => Some(false),
                    1 if inter[0] == b'?' => Some(true),
                    _ => None,
                };
                let Some(protected) = protected else {
                    log::warn!("invalid erase display command: {input}");
                    return Ok(());
                };

                let mode = match params.len() {
                    0 => Some(csi::EraseDisplay::Below),
                    1 => csi::EraseDisplay::from_int(params[0]),
                    _ => None,
                };
                let Some(mode) = mode else {
                    log::warn!("invalid erase display command: {input}");
                    return Ok(());
                };

                self.handler.vt(match mode {
                    csi::EraseDisplay::Below => Action::EraseDisplayBelow(protected),
                    csi::EraseDisplay::Above => Action::EraseDisplayAbove(protected),
                    csi::EraseDisplay::Complete => Action::EraseDisplayComplete(protected),
                    csi::EraseDisplay::Scrollback => Action::EraseDisplayScrollback(protected),
                    csi::EraseDisplay::ScrollComplete => {
                        Action::EraseDisplayScrollComplete(protected)
                    }
                })
            }

            // Erase Line
            b'K' => {
                let protected = match inter.len() {
                    0 => Some(false),
                    1 if inter[0] == b'?' => Some(true),
                    _ => None,
                };
                let Some(protected) = protected else {
                    log::warn!("invalid erase line command: {input}");
                    return Ok(());
                };

                let mode = match params.len() {
                    0 => Some(csi::EraseLine::Right),
                    1 if params[0] < 3 => Some(csi::EraseLine::from_int(params[0] as u8)),
                    _ => None,
                };
                let Some(mode) = mode else {
                    log::warn!("invalid erase line command: {input}");
                    return Ok(());
                };

                match mode {
                    csi::EraseLine::Right => self.handler.vt(Action::EraseLineRight(protected)),
                    csi::EraseLine::Left => self.handler.vt(Action::EraseLineLeft(protected)),
                    csi::EraseLine::Complete => {
                        self.handler.vt(Action::EraseLineComplete(protected))
                    }
                    csi::EraseLine::RightUnlessPendingWrap => self
                        .handler
                        .vt(Action::EraseLineRightUnlessPendingWrap(protected)),
                    csi::EraseLine::Other(_) => {
                        log::warn!("invalid erase line mode: {mode:?}");
                        Ok(())
                    }
                }
            }

            // IL - Insert Lines
            b'L' => counted!("L", "IL", |v| Action::InsertLines(usize::from(v))),
            // DL - Delete Lines
            b'M' => counted!("M", "DL", |v| Action::DeleteLines(usize::from(v))),
            // Delete Character (DCH)
            b'P' => counted!("P", "delete characters", |v| Action::DeleteChars(
                usize::from(v)
            )),
            // Scroll Up (SU)
            b'S' => counted!("S", "scroll up", |v| Action::ScrollUp(usize::from(v))),
            // Scroll Down (SD)
            b'T' => counted!("T", "scroll down", |v| Action::ScrollDown(usize::from(v))),

            // Cursor Tabulation Control
            b'W' => match inter.len() {
                0 => {
                    if params.is_empty() || (params.len() == 1 && params[0] == 0) {
                        return self.handler.vt(Action::TabSet);
                    }
                    if params.len() == 1 {
                        match params[0] {
                            2 => return self.handler.vt(Action::TabClearCurrent),
                            5 => return self.handler.vt(Action::TabClearAll),
                            _ => {}
                        }
                    }
                    log::warn!("invalid cursor tabulation control: {input}");
                    Ok(())
                }
                1 => {
                    if inter[0] == b'?' && params.len() == 1 && params[0] == 5 {
                        self.handler.vt(Action::TabReset)
                    } else {
                        log::warn!("invalid cursor tabulation control: {input}");
                        Ok(())
                    }
                }
                _ => {
                    unimpl_inter!("W");
                    Ok(())
                }
            },

            // Erase Characters (ECH)
            b'X' => counted!("X", "erase characters", |v| Action::EraseChars(
                usize::from(v)
            )),
            // CBT - Cursor Horizontal Tabulation Back
            b'Z' => counted!("Z", "horizontal tab back", Action::HorizontalTabBack),
            // HPR - Cursor Horizontal Position Relative
            b'a' => counted!("a", "HPR", |v| Action::CursorColRelative(CursorMovement {
                value: v
            })),
            // Repeat Previous Char (REP)
            b'b' => counted!("b", "print repeat", |v| Action::PrintRepeat(usize::from(v))),

            // c - Device Attributes (DA1)
            b'c' => {
                let req = match inter.len() {
                    0 => Some(ansi::DeviceAttributeReq::Primary),
                    1 => match inter[0] {
                        b'>' => Some(ansi::DeviceAttributeReq::Secondary),
                        b'=' => Some(ansi::DeviceAttributeReq::Tertiary),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(r) = req {
                    self.handler.vt(Action::DeviceAttributes(r))
                } else {
                    log::warn!("invalid device attributes command: {input}");
                    Ok(())
                }
            }

            // VPA - Cursor Vertical Position Absolute
            b'd' => counted!("d", "VPA", |v| Action::CursorRow(CursorMovement {
                value: v
            })),
            // VPR - Cursor Vertical Position Relative
            b'e' => counted!("e", "VPR", |v| Action::CursorRowRelative(CursorMovement {
                value: v
            })),

            // TBC - Tab Clear
            b'g' => {
                if !inter.is_empty() {
                    unimpl_inter!("g");
                    return Ok(());
                }
                if params.len() != 1 {
                    log::warn!("invalid tab clear command: {input}");
                    return Ok(());
                }
                // Zig's intToEnum into a u8 enum fails when the value
                // doesn't fit.
                let Ok(raw) = u8::try_from(params[0]) else {
                    log::warn!("invalid tab clear mode: {}", params[0]);
                    return Ok(());
                };
                match csi::TabClear::from_int(raw) {
                    csi::TabClear::Current => self.handler.vt(Action::TabClearCurrent),
                    csi::TabClear::All => self.handler.vt(Action::TabClearAll),
                    csi::TabClear::Other(m) => {
                        log::warn!("unknown tab clear mode: {m}");
                        Ok(())
                    }
                }
            }

            // SM - Set Mode / RM - Reset Mode
            b'h' | b'l' => {
                let set = input.final_ == b'h';
                let ansi_mode = match inter.len() {
                    0 => true,
                    1 if inter[0] == b'?' => false,
                    _ => {
                        log::warn!("invalid set mode command: {input}");
                        return Ok(());
                    }
                };

                for &mode_int in params {
                    if let Some(mode) = modes::mode_from_int(mode_int, ansi_mode) {
                        let m = Mode { mode };
                        self.handler.vt(if set {
                            Action::SetMode(m)
                        } else {
                            Action::ResetMode(m)
                        })?;
                    } else {
                        log::warn!("unimplemented mode: {mode_int}");
                    }
                }
                Ok(())
            }

            // SGR - Select Graphic Rendition
            b'm' => match inter.len() {
                0 => {
                    let mut p = sgr::Parser {
                        params,
                        params_sep: input.params_sep,
                        idx: 0,
                    };
                    for attr in &mut p {
                        self.handler.vt(Action::SetAttribute(attr))?;
                    }
                    Ok(())
                }

                1 => match inter[0] {
                    b'>' => {
                        if params.is_empty() {
                            // Reset
                            return self
                                .handler
                                .vt(Action::ModifyKeyFormat(ansi::ModifyKeyFormat::Legacy));
                        }

                        let mut format = match params[0] {
                            0 => ansi::ModifyKeyFormat::Legacy,
                            1 => ansi::ModifyKeyFormat::CursorKeys,
                            2 => ansi::ModifyKeyFormat::FunctionKeys,
                            4 => ansi::ModifyKeyFormat::OtherKeysNone,
                            _ => {
                                log::warn!("invalid setModifyKeyFormat: {input}");
                                return Ok(());
                            }
                        };

                        if params.len() > 2 {
                            log::warn!("invalid setModifyKeyFormat: {input}");
                            return Ok(());
                        }

                        // We only support the numeric form for the subparam.
                        if params.len() == 2
                            && format == ansi::ModifyKeyFormat::OtherKeysNone
                            && params[1] == 2
                        {
                            format = ansi::ModifyKeyFormat::OtherKeysNumeric;
                        }

                        self.handler.vt(Action::ModifyKeyFormat(format))
                    }
                    c => {
                        log::warn!("unknown CSI m with intermediate: {c}");
                        Ok(())
                    }
                },

                _ => {
                    // There are other forms of CSI m that have intermediates.
                    // `vim --clean` uses `CSI ? 4 m` and I don't know what
                    // that means.
                    unimpl_inter!("m");
                    Ok(())
                }
            },

            b'n' => {
                // Handle deviceStatusReport first
                if inter.is_empty() || inter[0] == b'?' {
                    if params.len() != 1 {
                        log::warn!("invalid device status report command: {input}");
                        return Ok(());
                    }

                    let question = match inter.len() {
                        0 => false,
                        1 if inter[0] == b'?' => true,
                        _ => {
                            log::warn!("invalid set mode command: {input}");
                            return Ok(());
                        }
                    };

                    let Some(req) = device_status::req_from_int(params[0], question) else {
                        log::warn!("invalid device status report command: {input}");
                        return Ok(());
                    };

                    return self
                        .handler
                        .vt(Action::DeviceStatus(DeviceStatus { request: req }));
                }

                // Handle other forms of CSI n
                match inter.len() {
                    1 => match inter[0] {
                        // This isn't strictly correct. CSI > n has parameters
                        // that control what exactly is being disabled. However,
                        // we only support reverting back to modify other keys
                        // in numeric except format.
                        b'>' => self.handler.vt(Action::ModifyKeyFormat(
                            ansi::ModifyKeyFormat::OtherKeysNumericExcept,
                        )),
                        c => {
                            log::warn!("unknown CSI n with intermediate: {c}");
                            Ok(())
                        }
                    },
                    _ => {
                        unimpl_inter!("n");
                        Ok(())
                    }
                }
            }

            // DECRQM - Request Mode
            b'p' => {
                if inter.len() != 2 {
                    unimpl_inter!("p");
                    return Ok(());
                }
                if !(inter[0] == b'?' && inter[1] == b'$') {
                    unimpl_inter!("p");
                    return Ok(());
                }
                let ansi_mode = false;

                if params.len() != 1 {
                    log::warn!("invalid DECRQM command: {input}");
                    return Ok(());
                }

                let mode_raw = params[0];
                match modes::mode_from_int(mode_raw, ansi_mode) {
                    Some(m) => self.handler.vt(Action::RequestMode(Mode { mode: m })),
                    None => self.handler.vt(Action::RequestModeUnknown(RawMode {
                        mode: mode_raw,
                        ansi: ansi_mode,
                    })),
                }
            }

            b'q' => {
                if inter.len() != 1 {
                    unimpl_inter!("p");
                    return Ok(());
                }
                match inter[0] {
                    // DECSCUSR - Select Cursor Style
                    b' ' => {
                        let style = match params.len() {
                            0 => ansi::CursorStyle::Default,
                            1 => match params[0] {
                                0 => ansi::CursorStyle::Default,
                                1 => ansi::CursorStyle::BlinkingBlock,
                                2 => ansi::CursorStyle::SteadyBlock,
                                3 => ansi::CursorStyle::BlinkingUnderline,
                                4 => ansi::CursorStyle::SteadyUnderline,
                                5 => ansi::CursorStyle::BlinkingBar,
                                6 => ansi::CursorStyle::SteadyBar,
                                v => {
                                    log::warn!("invalid cursor style value: {v}");
                                    return Ok(());
                                }
                            },
                            _ => {
                                log::warn!("invalid set cursor style command: {input}");
                                return Ok(());
                            }
                        };
                        self.handler.vt(Action::CursorStyle(style))
                    }

                    // DECSCA
                    b'"' => {
                        let mode = match params.len() {
                            0 => Some(ansi::ProtectedMode::Off),
                            1 => match params[0] {
                                0 | 2 => Some(ansi::ProtectedMode::Off),
                                1 => Some(ansi::ProtectedMode::Dec),
                                _ => None,
                            },
                            _ => None,
                        };
                        let Some(mode) = mode else {
                            log::warn!("invalid set protected mode command: {input}");
                            return Ok(());
                        };
                        self.handler.vt(match mode {
                            ansi::ProtectedMode::Off => Action::ProtectedModeOff,
                            ansi::ProtectedMode::Iso => Action::ProtectedModeIso,
                            ansi::ProtectedMode::Dec => Action::ProtectedModeDec,
                        })
                    }

                    // XTVERSION
                    b'>' => self.handler.vt(Action::Xtversion),

                    _ => {
                        unimpl_inter!("q");
                        Ok(())
                    }
                }
            }

            b'r' => match inter.len() {
                // DECSTBM - Set Top and Bottom Margins
                0 => {
                    let m = match params.len() {
                        0 => Margin {
                            top_left: 0,
                            bottom_right: 0,
                        },
                        1 => Margin {
                            top_left: params[0],
                            bottom_right: 0,
                        },
                        2 => Margin {
                            top_left: params[0],
                            bottom_right: params[1],
                        },
                        _ => {
                            log::warn!("invalid DECSTBM command: {input}");
                            return Ok(());
                        }
                    };
                    self.handler.vt(Action::TopAndBottomMargin(m))
                }

                1 => match inter[0] {
                    // Restore Mode
                    b'?' => {
                        for &mode_int in params {
                            if let Some(mode) = modes::mode_from_int(mode_int, false) {
                                self.handler.vt(Action::RestoreMode(Mode { mode }))?;
                            } else {
                                log::warn!("unimplemented restore mode: {mode_int}");
                            }
                        }
                        Ok(())
                    }
                    _ => {
                        log::warn!("unknown CSI s with intermediate: {input}");
                        Ok(())
                    }
                },

                _ => {
                    log::warn!("ignoring unimplemented CSI s with intermediates: {input}");
                    Ok(())
                }
            },

            b's' => match inter.len() {
                // DECSLRM
                0 => match params.len() {
                    // CSI S is ambiguous with zero params so we defer to our
                    // handler to do the proper logic. If mode 69 is set, then
                    // we should invoke DECSLRM, otherwise we should invoke SC.
                    0 => self.handler.vt(Action::LeftAndRightMarginAmbiguous),
                    1 => self.handler.vt(Action::LeftAndRightMargin(Margin {
                        top_left: params[0],
                        bottom_right: 0,
                    })),
                    2 => self.handler.vt(Action::LeftAndRightMargin(Margin {
                        top_left: params[0],
                        bottom_right: params[1],
                    })),
                    _ => {
                        log::warn!("invalid DECSLRM command: {input}");
                        Ok(())
                    }
                },

                1 => match inter[0] {
                    b'?' => {
                        for &mode_int in params {
                            if let Some(mode) = modes::mode_from_int(mode_int, false) {
                                self.handler.vt(Action::SaveMode(Mode { mode }))?;
                            } else {
                                log::warn!("unimplemented save mode: {mode_int}");
                            }
                        }
                        Ok(())
                    }

                    // XTSHIFTESCAPE
                    b'>' => {
                        let capture = match params.len() {
                            0 => false,
                            1 => match params[0] {
                                0 => false,
                                1 => true,
                                _ => {
                                    log::warn!("invalid XTSHIFTESCAPE command: {input}");
                                    return Ok(());
                                }
                            },
                            _ => {
                                log::warn!("invalid XTSHIFTESCAPE command: {input}");
                                return Ok(());
                            }
                        };
                        self.handler.vt(Action::MouseShiftCapture(capture))
                    }

                    _ => {
                        log::warn!("unknown CSI s with intermediate: {input}");
                        Ok(())
                    }
                },

                _ => {
                    log::warn!("ignoring unimplemented CSI s with intermediates: {input}");
                    Ok(())
                }
            },

            // XTWINOPS
            b't' => {
                if !inter.is_empty() {
                    log::warn!("ignoring unimplemented CSI t with intermediates: {input}");
                    return Ok(());
                }
                if params.is_empty() {
                    log::error!("ignoring CSI t with no parameters: {input}");
                    return Ok(());
                }

                let style = match params[0] {
                    14 => Some(csi::SizeReportStyle::Csi14T),
                    16 => Some(csi::SizeReportStyle::Csi16T),
                    18 => Some(csi::SizeReportStyle::Csi18T),
                    21 => Some(csi::SizeReportStyle::Csi21T),
                    _ => None,
                };
                if let Some(style) = style {
                    if params.len() == 1 {
                        // report text area size / cell size / screen size /
                        // window title
                        return self.handler.vt(Action::SizeReport(style));
                    }
                    log::warn!(
                        "ignoring CSI {} t with extra parameters: {input}",
                        params[0]
                    );
                    return Ok(());
                }

                match params[0] {
                    n @ (22 | 23) => {
                        // we only support window title
                        if (params.len() == 2 || params.len() == 3)
                            && (params[1] == 0 || params[1] == 2)
                        {
                            // push/pop title
                            let index = if params.len() == 3 { params[2] } else { 0 };
                            self.handler.vt(if n == 22 {
                                Action::TitlePush(index)
                            } else {
                                Action::TitlePop(index)
                            })
                        } else {
                            log::warn!("ignoring CSI 22/23 t with extra parameters: {input}");
                            Ok(())
                        }
                    }
                    _ => {
                        log::warn!("ignoring CSI t with unimplemented parameter: {input}");
                        Ok(())
                    }
                }
            }

            b'u' => match inter.len() {
                0 => self.handler.vt(Action::RestoreCursor),

                // Kitty keyboard protocol
                1 => match inter[0] {
                    b'?' => self.handler.vt(Action::KittyKeyboardQuery),

                    b'>' => {
                        let flags = if params.len() == 1 {
                            // Zig: std.math.cast(u5, ...)
                            match u8::try_from(params[0]) {
                                Ok(f) if f < 32 => f,
                                _ => {
                                    log::warn!("invalid pushKittyKeyboard command: {input}");
                                    return Ok(());
                                }
                            }
                        } else {
                            0
                        };
                        self.handler
                            .vt(Action::KittyKeyboardPush(KittyKeyboardFlags {
                                flags: kitty::key::Flags::from_int(flags),
                            }))
                    }

                    b'<' => {
                        let number = if params.len() == 1 { params[0] } else { 1 };
                        self.handler.vt(Action::KittyKeyboardPop(number))
                    }

                    b'=' => {
                        let flags = if !params.is_empty() {
                            match u8::try_from(params[0]) {
                                Ok(f) if f < 32 => f,
                                _ => {
                                    log::warn!("invalid setKittyKeyboard command: {input}");
                                    return Ok(());
                                }
                            }
                        } else {
                            0
                        };
                        let number = if params.len() >= 2 { params[1] } else { 1 };
                        let kf = KittyKeyboardFlags {
                            flags: kitty::key::Flags::from_int(flags),
                        };
                        match number {
                            1 => self.handler.vt(Action::KittyKeyboardSet(kf)),
                            2 => self.handler.vt(Action::KittyKeyboardSetOr(kf)),
                            3 => self.handler.vt(Action::KittyKeyboardSetNot(kf)),
                            _ => {
                                log::warn!("invalid setKittyKeyboard command: {input}");
                                Ok(())
                            }
                        }
                    }

                    _ => {
                        log::warn!("unknown CSI s with intermediate: {input}");
                        Ok(())
                    }
                },

                _ => {
                    log::warn!("ignoring unimplemented CSI u: {input}");
                    Ok(())
                }
            },

            // ICH - Insert Blanks
            b'@' => {
                if !inter.is_empty() {
                    log::warn!("ignoring unimplemented CSI @: {input}");
                    return Ok(());
                }
                let v = match params.len() {
                    0 => 1,
                    1 => params[0].max(1),
                    _ => {
                        log::warn!("invalid ICH command: {input}");
                        return Ok(());
                    }
                };
                self.handler.vt(Action::InsertBlanks(usize::from(v)))
            }

            // DECSASD - Select Active Status Display
            b'}' => {
                // Verify we're getting a DECSASD command
                if inter.len() != 1 || inter[0] != b'$' || params.len() != 1 {
                    log::warn!("unimplemented CSI callback: {input}");
                    return Ok(());
                }
                let display = match params[0] {
                    0 => ansi::StatusDisplay::Main,
                    1 => ansi::StatusDisplay::StatusLine,
                    _ => {
                        log::warn!("unimplemented CSI callback: {input}");
                        return Ok(());
                    }
                };
                self.handler.vt(Action::ActiveStatusDisplay(display))
            }

            _ => {
                log::warn!("unimplemented CSI action: {input}");
                Ok(())
            }
        }
    }

    fn osc_dispatch(&mut self, cmd: &osc::Command) -> Res<H> {
        use osc::Command as C;
        match cmd {
            C::SemanticPrompt(sp) => self.handler.vt(Action::SemanticPrompt(sp)),

            C::ChangeWindowTitle(title) => {
                let Ok(title) = std::str::from_utf8(title) else {
                    log::warn!("change title request: invalid utf-8, ignoring request");
                    return Ok(());
                };
                self.handler.vt(Action::WindowTitle(WindowTitle { title }))
            }

            C::ChangeWindowIcon(icon) => {
                log::info!(
                    "OSC 1 (change icon) received and ignored icon={}",
                    String::from_utf8_lossy(icon)
                );
                Ok(())
            }

            C::ClipboardContents { kind, data } => {
                self.handler
                    .vt(Action::ClipboardContents(ClipboardContents {
                        kind: *kind,
                        data,
                    }))
            }

            C::ReportPwd { value } => self.handler.vt(Action::ReportPwd(ReportPwd { url: value })),

            C::MouseShape { value } => {
                let shape = std::str::from_utf8(value)
                    .ok()
                    .and_then(MouseShape::from_string);
                let Some(shape) = shape else {
                    log::warn!("unknown cursor shape: {}", String::from_utf8_lossy(value));
                    return Ok(());
                };
                self.handler.vt(Action::MouseShape(shape))
            }

            C::ColorOperation {
                op,
                requests,
                terminator,
            } => self.handler.vt(Action::ColorOperation(ColorOperation {
                op: *op,
                requests,
                terminator: *terminator,
            })),

            C::KittyColorProtocol(v) => self.handler.vt(Action::KittyColorReport(v)),

            C::ShowDesktopNotification { title, body } => {
                self.handler
                    .vt(Action::ShowDesktopNotification(ShowDesktopNotification {
                        title,
                        body,
                    }))
            }

            C::HyperlinkStart { id, uri } => {
                self.handler.vt(Action::StartHyperlink(StartHyperlink {
                    uri,
                    id: id.as_deref(),
                }))
            }

            C::HyperlinkEnd => self.handler.vt(Action::EndHyperlink),

            C::ConemuProgressReport(v) => self.handler.vt(Action::ProgressReport(*v)),

            C::ConemuSleep { .. }
            | C::ConemuShowMessageBox(_)
            | C::ConemuChangeTabTitle(_)
            | C::ConemuWaitInput
            | C::ConemuGuimacro(_)
            | C::ConemuComment(_)
            | C::ConemuXtermEmulation { .. }
            | C::ConemuOutputEnvironmentVariable(_)
            | C::ConemuRunProcess(_)
            | C::KittyTextSizing(_)
            | C::KittyClipboardProtocol(_)
            | C::ContextSignal(_) => {
                log::debug!("unimplemented OSC callback: {cmd:?}");
                Ok(())
            }

            C::Invalid => {
                // This is an invalid internal state, not an invalid OSC
                // string being parsed. We shouldn't see this.
                log::warn!("invalid OSC, should never happen");
                Ok(())
            }
        }
    }

    fn configure_charset(&mut self, intermediates: &[u8], set: charsets::Charset) -> Res<H> {
        if intermediates.len() != 1 {
            log::warn!("invalid charset intermediate: {intermediates:?}");
            return Ok(());
        }

        let slot = match intermediates[0] {
            // TODO: support slots '-', '.', '/'
            b'(' => charsets::Slots::G0,
            b')' => charsets::Slots::G1,
            b'*' => charsets::Slots::G2,
            b'+' => charsets::Slots::G3,
            _ => {
                log::warn!("invalid charset intermediate: {intermediates:?}");
                return Ok(());
            }
        };

        self.handler.vt(Action::ConfigureCharset(ConfigureCharset {
            slot,
            charset: set,
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn esc_dispatch(&mut self, action: Esc) -> Res<H> {
        let inter = action.intermediates();

        // ESC commands that must have no intermediates, mapped to an action;
        // otherwise log `$msg` and ignore.
        macro_rules! bare {
            ($msg:literal, $act:expr) => {{
                if inter.is_empty() {
                    self.handler.vt($act)
                } else {
                    log::warn!(concat!($msg, ": {}"), action);
                    Ok(())
                }
            }};
        }
        macro_rules! shift {
            ($msg:literal, $bank:ident, $slot:ident, $locking:literal) => {
                bare!(
                    $msg,
                    Action::InvokeCharset(InvokeCharset {
                        bank: charsets::ActiveSlot::$bank,
                        charset: charsets::Slots::$slot,
                        locking: $locking,
                    })
                )
            };
        }

        match action.final_ {
            // Charsets
            b'B' => self.configure_charset(inter, charsets::Charset::Ascii),
            b'A' => self.configure_charset(inter, charsets::Charset::British),
            b'0' => self.configure_charset(inter, charsets::Charset::DecSpecial),

            // DECSC - Save Cursor
            b'7' => bare!("invalid command", Action::SaveCursor),

            b'8' => {
                match inter.len() {
                    // DECRC - Restore Cursor
                    0 => return self.handler.vt(Action::RestoreCursor),
                    // DECALN - Fill Screen with E
                    1 if inter[0] == b'#' => return self.handler.vt(Action::Decaln),
                    _ => {}
                }
                log::warn!("unimplemented ESC action: {action}");
                Ok(())
            }

            // IND - Index
            b'D' => bare!("invalid index command", Action::Index),
            // NEL - Next Line
            b'E' => bare!("invalid next line command", Action::NextLine),
            // HTS - Horizontal Tab Set
            b'H' => bare!("invalid tab set command", Action::TabSet),
            // RI - Reverse Index
            b'M' => bare!("invalid reverse index command", Action::ReverseIndex),
            // SS2 - Single Shift 2
            b'N' => shift!("invalid single shift 2 command", GL, G2, true),
            // SS3 - Single Shift 3
            b'O' => shift!("invalid single shift 3 command", GL, G3, true),
            // SPA - Start of Guarded Area
            b'V' => bare!("unimplemented ESC callback", Action::ProtectedModeIso),
            // EPA - End of Guarded Area
            b'W' => bare!("unimplemented ESC callback", Action::ProtectedModeOff),
            // DECID
            b'Z' => bare!(
                "unimplemented ESC callback",
                Action::DeviceAttributes(ansi::DeviceAttributeReq::Primary)
            ),
            // RIS - Full Reset
            b'c' => bare!("invalid full reset command", Action::FullReset),
            // LS2 - Locking Shift 2
            b'n' => shift!("invalid single shift 2 command", GL, G2, false),
            // LS3 - Locking Shift 3
            b'o' => shift!("invalid single shift 3 command", GL, G3, false),
            // LS1R - Locking Shift 1 Right
            b'~' => shift!("invalid locking shift 1 right command", GR, G1, false),
            // LS2R - Locking Shift 2 Right
            b'}' => shift!("invalid locking shift 2 right command", GR, G2, false),
            // LS3R - Locking Shift 3 Right
            b'|' => shift!("invalid locking shift 3 right command", GR, G3, false),

            // Set application keypad mode
            b'=' => bare!(
                "unimplemented setMode",
                Action::SetMode(Mode {
                    mode: modes::Mode::KeypadKeys
                })
            ),
            // Reset application keypad mode
            b'>' => bare!(
                "unimplemented setMode",
                Action::ResetMode(Mode {
                    mode: modes::Mode::KeypadKeys
                })
            ),

            // Sets ST (string terminator). We don't have to do anything
            // because our parser always accepts ST.
            b'\\' => Ok(()),

            _ => {
                log::warn!("unimplemented ESC action: {action}");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::convert::Infallible;

    use super::*;

    /// Test handler that forwards every action to a closure.
    struct F<T, C: FnMut(&mut T, Action<'_>)> {
        state: T,
        f: C,
    }

    impl<T, C: FnMut(&mut T, Action<'_>)> Handler for F<T, C> {
        type Error = Infallible;
        fn vt(&mut self, action: Action<'_>) -> Result<(), Infallible> {
            (self.f)(&mut self.state, action);
            Ok(())
        }
    }

    fn stream<T, C: FnMut(&mut T, Action<'_>)>(state: T, f: C) -> Stream<F<T, C>> {
        Stream::new(F { state, f })
    }

    fn feed(s: &mut Stream<impl Handler>, input: &str) {
        for &c in input.as_bytes() {
            s.next(c).unwrap();
        }
    }

    fn slice(s: &mut Stream<impl Handler>, input: &[u8]) {
        s.next_slice(input).unwrap();
    }

    #[test]
    fn stream_print() {
        let mut s = stream(Some(0u32), |c, a| {
            if let Action::Print(p) = a {
                *c = Some(p.cp);
            }
        });
        s.next(b'x').unwrap();
        assert_eq!(s.handler.state, Some(u32::from('x')));
    }

    #[test]
    fn simd_print_invalid_utf8() {
        let mut s = stream(Some(0u32), |c, a| {
            if let Action::Print(p) = a {
                *c = Some(p.cp);
            }
        });
        slice(&mut s, &[0xFF]);
        assert_eq!(s.handler.state, Some(0xFFFD));
    }

    #[test]
    fn simd_complete_incomplete_utf8() {
        let mut s = stream(None::<u32>, |c, a| {
            if let Action::Print(p) = a {
                *c = Some(p.cp);
            }
        });
        slice(&mut s, &[0xE0]); // 3 byte
        assert_eq!(s.handler.state, None);
        slice(&mut s, &[0xA0]); // still incomplete
        assert_eq!(s.handler.state, None);
        slice(&mut s, &[0x80]);
        assert_eq!(s.handler.state, Some(0x800));
    }

    #[test]
    fn stream_cursor_right_cuf() {
        let mut s = stream(0u16, |n, a| {
            if let Action::CursorRight(m) = a {
                *n = m.value;
            }
        });
        slice(&mut s, b"\x1B[C");
        assert_eq!(s.handler.state, 1);

        slice(&mut s, b"\x1B[5C");
        assert_eq!(s.handler.state, 5);

        s.handler.state = 0;
        slice(&mut s, b"\x1B[5;4C");
        assert_eq!(s.handler.state, 0);

        s.handler.state = 0;
        slice(&mut s, b"\x1b[?3C");
        assert_eq!(s.handler.state, 0);
    }

    #[test]
    fn stream_dec_set_mode_sm_and_reset_mode_rm() {
        // Zig initializes to `@enumFromInt(1)`; mode index 1 is `insert`.
        let init = modes::Mode::Insert;
        let mut s = stream(init, move |m, a| match a {
            Action::SetMode(v) => *m = v.mode,
            Action::ResetMode(_) => *m = init,
            _ => {}
        });
        slice(&mut s, b"\x1B[?6h");
        assert_eq!(s.handler.state, modes::Mode::Origin);

        slice(&mut s, b"\x1B[?6l");
        assert_eq!(s.handler.state, init);

        s.handler.state = init;
        slice(&mut s, b"\x1B[6 h");
        assert_eq!(s.handler.state, init);
    }

    #[test]
    fn stream_ansi_set_mode_sm_and_reset_mode_rm() {
        let mut s = stream(None::<modes::Mode>, |m, a| match a {
            Action::SetMode(v) => *m = Some(v.mode),
            Action::ResetMode(_) => *m = None,
            _ => {}
        });
        slice(&mut s, b"\x1B[4h");
        assert_eq!(s.handler.state, Some(modes::Mode::Insert));

        slice(&mut s, b"\x1B[4l");
        assert_eq!(s.handler.state, None);

        s.handler.state = None;
        slice(&mut s, b"\x1B[>5h");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_ansi_set_mode_sm_and_reset_mode_rm_with_unknown_value() {
        let mut s = stream(None::<modes::Mode>, |m, a| match a {
            Action::SetMode(v) => *m = Some(v.mode),
            Action::ResetMode(_) => *m = None,
            _ => {}
        });
        slice(&mut s, b"\x1B[6h");
        assert_eq!(s.handler.state, None);

        slice(&mut s, b"\x1B[6l");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_restore_mode() {
        let mut s = stream(false, |c, a| {
            if a.key() == ActionKey::TopAndBottomMargin {
                *c = true;
            }
        });
        feed(&mut s, "\x1B[?42r");
        assert!(!s.handler.state);
    }

    #[test]
    fn stream_pop_kitty_keyboard_with_no_params_defaults_to_1() {
        let mut s = stream(0u16, |n, a| {
            if let Action::KittyKeyboardPop(v) = a {
                *n = v;
            }
        });
        feed(&mut s, "\x1B[<u");
        assert_eq!(s.handler.state, 1);
    }

    #[test]
    fn stream_decsca() {
        let mut s = stream(None::<ansi::ProtectedMode>, |v, a| match a {
            Action::ProtectedModeOff => *v = Some(ansi::ProtectedMode::Off),
            Action::ProtectedModeIso => *v = Some(ansi::ProtectedMode::Iso),
            Action::ProtectedModeDec => *v = Some(ansi::ProtectedMode::Dec),
            _ => {}
        });
        feed(&mut s, "\x1B[\"q");
        assert_eq!(s.handler.state, Some(ansi::ProtectedMode::Off));
        feed(&mut s, "\x1B[0\"q");
        assert_eq!(s.handler.state, Some(ansi::ProtectedMode::Off));
        feed(&mut s, "\x1B[2\"q");
        assert_eq!(s.handler.state, Some(ansi::ProtectedMode::Off));
        feed(&mut s, "\x1B[1\"q");
        assert_eq!(s.handler.state, Some(ansi::ProtectedMode::Dec));
    }

    #[test]
    fn stream_deced_decsed() {
        use csi::EraseDisplay as E;
        let mut s = stream((None::<E>, None::<bool>), |st, a| {
            let (m, p) = match a {
                Action::EraseDisplayBelow(p) => (E::Below, p),
                Action::EraseDisplayAbove(p) => (E::Above, p),
                Action::EraseDisplayComplete(p) => (E::Complete, p),
                Action::EraseDisplayScrollback(p) => (E::Scrollback, p),
                Action::EraseDisplayScrollComplete(p) => (E::ScrollComplete, p),
                _ => return,
            };
            *st = (Some(m), Some(p));
        });
        for (seq, mode, prot) in [
            ("\x1B[?J", E::Below, true),
            ("\x1B[?0J", E::Below, true),
            ("\x1B[?1J", E::Above, true),
            ("\x1B[?2J", E::Complete, true),
            ("\x1B[?3J", E::Scrollback, true),
            ("\x1B[J", E::Below, false),
            ("\x1B[0J", E::Below, false),
            ("\x1B[1J", E::Above, false),
            ("\x1B[2J", E::Complete, false),
            ("\x1B[3J", E::Scrollback, false),
            // Invalid and ignored by the handler
            ("\x1B[>0J", E::Scrollback, false),
        ] {
            feed(&mut s, seq);
            assert_eq!(s.handler.state, (Some(mode), Some(prot)), "{seq:?}");
        }
        // Kitty extension
        feed(&mut s, "\x1B[22J");
        assert_eq!(s.handler.state, (Some(E::ScrollComplete), Some(false)));
    }

    #[test]
    fn stream_decel_decsel() {
        use csi::EraseLine as E;
        let mut s = stream((None::<E>, None::<bool>), |st, a| {
            let (m, p) = match a {
                Action::EraseLineRight(p) => (E::Right, p),
                Action::EraseLineLeft(p) => (E::Left, p),
                Action::EraseLineComplete(p) => (E::Complete, p),
                Action::EraseLineRightUnlessPendingWrap(p) => (E::RightUnlessPendingWrap, p),
                _ => return,
            };
            *st = (Some(m), Some(p));
        });
        for (seq, mode, prot) in [
            ("\x1B[?K", E::Right, true),
            ("\x1B[?0K", E::Right, true),
            ("\x1B[?1K", E::Left, true),
            ("\x1B[?2K", E::Complete, true),
            ("\x1B[K", E::Right, false),
            ("\x1B[0K", E::Right, false),
            ("\x1B[1K", E::Left, false),
            ("\x1B[2K", E::Complete, false),
            // Invalid and ignored by the handler
            ("\x1B[<1K", E::Complete, false),
        ] {
            feed(&mut s, seq);
            assert_eq!(s.handler.state, (Some(mode), Some(prot)), "{seq:?}");
        }
    }

    #[test]
    fn stream_decscusr() {
        let mut s = stream(None::<ansi::CursorStyle>, |st, a| {
            if let Action::CursorStyle(v) = a {
                *st = Some(v);
            }
        });
        slice(&mut s, b"\x1B[ q");
        assert_eq!(s.handler.state, Some(ansi::CursorStyle::Default));

        slice(&mut s, b"\x1B[1 q");
        assert_eq!(s.handler.state, Some(ansi::CursorStyle::BlinkingBlock));

        // Invalid and ignored by the handler
        slice(&mut s, b"\x1B[?0 q");
        assert_eq!(s.handler.state, Some(ansi::CursorStyle::BlinkingBlock));
    }

    #[test]
    fn stream_decscusr_without_space() {
        let mut s = stream(None::<ansi::CursorStyle>, |st, a| {
            if let Action::CursorStyle(v) = a {
                *st = Some(v);
            }
        });
        slice(&mut s, b"\x1B[q");
        assert_eq!(s.handler.state, None);

        slice(&mut s, b"\x1B[1q");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_xtshiftescape() {
        let mut s = stream(None::<bool>, |st, a| {
            if let Action::MouseShiftCapture(v) = a {
                *st = Some(v);
            }
        });
        slice(&mut s, b"\x1B[>2s");
        assert_eq!(s.handler.state, None);

        slice(&mut s, b"\x1B[>s");
        assert_eq!(s.handler.state, Some(false));

        slice(&mut s, b"\x1B[>0s");
        assert_eq!(s.handler.state, Some(false));

        slice(&mut s, b"\x1B[>1s");
        assert_eq!(s.handler.state, Some(true));

        // Invalid and ignored by the handler
        slice(&mut s, b"\x1B[1 s");
        assert_eq!(s.handler.state, Some(true));
    }

    #[test]
    fn stream_change_window_title_with_invalid_utf8() {
        {
            let mut s = stream(false, |seen, a| {
                if let Action::WindowTitle(_) = a {
                    *seen = true;
                }
            });
            slice(&mut s, b"\x1b]2;abc\x1b\\");
            assert!(s.handler.state);
        }
        {
            let mut s = stream(false, |seen, a| {
                if let Action::WindowTitle(_) = a {
                    *seen = true;
                }
            });
            slice(&mut s, b"\x1b]2;abc\xc0\x1b\\");
            assert!(!s.handler.state);
        }
    }

    #[test]
    fn stream_insert_characters() {
        let mut s = stream(false, |c, a| {
            if let Action::InsertBlanks(_) = a {
                *c = true;
            }
        });
        feed(&mut s, "\x1B[42@");
        assert!(s.handler.state);

        s.handler.state = false;
        feed(&mut s, "\x1B[?42@");
        assert!(!s.handler.state);
    }

    #[test]
    fn stream_insert_characters_explicit_zero_clamps_to_1() {
        let mut s = stream(None::<usize>, |v, a| {
            if let Action::InsertBlanks(n) = a {
                *v = Some(n);
            }
        });
        feed(&mut s, "\x1B[0@");
        assert_eq!(s.handler.state, Some(1));
    }

    #[test]
    fn stream_scosc() {
        let mut s = stream(false, |c, a| match a {
            Action::LeftAndRightMargin(_) => panic!("bad"),
            Action::LeftAndRightMarginAmbiguous => *c = true,
            _ => {}
        });
        feed(&mut s, "\x1B[s");
        assert!(s.handler.state);
    }

    #[test]
    fn stream_scorc() {
        let mut s = stream(false, |c, a| {
            if let Action::RestoreCursor = a {
                *c = true;
            }
        });
        feed(&mut s, "\x1B[u");
        assert!(s.handler.state);
    }

    #[test]
    fn stream_too_many_csi_params() {
        let mut s = stream((), |(), a| {
            assert!(!matches!(a, Action::CursorRight(_)));
        });
        slice(&mut s, b"\x1B[1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1C");
    }

    #[test]
    fn stream_csi_param_too_long() {
        let mut s = stream((), |(), _| {});
        let mut input = b"\x1B[".to_vec();
        input.extend(std::iter::repeat_n(b'1', 300));
        input.push(b'C');
        slice(&mut s, &input);
    }

    #[test]
    fn stream_send_report_with_csi_t() {
        let mut s = stream(None::<csi::SizeReportStyle>, |st, a| {
            if let Action::SizeReport(v) = a {
                *st = Some(v);
            }
        });
        slice(&mut s, b"\x1b[14t");
        assert_eq!(s.handler.state, Some(csi::SizeReportStyle::Csi14T));
        slice(&mut s, b"\x1b[16t");
        assert_eq!(s.handler.state, Some(csi::SizeReportStyle::Csi16T));
        slice(&mut s, b"\x1b[18t");
        assert_eq!(s.handler.state, Some(csi::SizeReportStyle::Csi18T));
        slice(&mut s, b"\x1b[21t");
        assert_eq!(s.handler.state, Some(csi::SizeReportStyle::Csi21T));
    }

    #[test]
    fn stream_invalid_csi_t() {
        let mut s = stream(None::<csi::SizeReportStyle>, |st, a| {
            if let Action::SizeReport(v) = a {
                *st = Some(v);
            }
        });
        slice(&mut s, b"\x1b[19t");
        assert_eq!(s.handler.state, None);
    }

    #[allow(clippy::type_complexity)]
    fn title_stream(
        push: bool,
    ) -> Stream<F<Option<u16>, impl FnMut(&mut Option<u16>, Action<'_>)>> {
        stream(None::<u16>, move |idx, a| match a {
            Action::TitlePush(v) if push => *idx = Some(v),
            Action::TitlePop(v) if !push => *idx = Some(v),
            _ => {}
        })
    }

    #[test]
    fn stream_csi_t_push_title() {
        let mut s = title_stream(true);
        slice(&mut s, b"\x1b[22;0t");
        assert_eq!(s.handler.state, Some(0));
    }

    #[test]
    fn stream_csi_t_push_title_with_explicit_window() {
        let mut s = title_stream(true);
        slice(&mut s, b"\x1b[22;2t");
        assert_eq!(s.handler.state, Some(0));
    }

    #[test]
    fn stream_csi_t_push_title_with_explicit_icon() {
        let mut s = title_stream(true);
        slice(&mut s, b"\x1b[22;1t");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_csi_t_push_title_with_index() {
        let mut s = title_stream(true);
        slice(&mut s, b"\x1b[22;0;5t");
        assert_eq!(s.handler.state, Some(5));
    }

    #[test]
    fn stream_csi_t_pop_title() {
        let mut s = title_stream(false);
        slice(&mut s, b"\x1b[23;0t");
        assert_eq!(s.handler.state, Some(0));
    }

    #[test]
    fn stream_csi_t_pop_title_with_explicit_window() {
        let mut s = title_stream(false);
        slice(&mut s, b"\x1b[23;2t");
        assert_eq!(s.handler.state, Some(0));
    }

    #[test]
    fn stream_csi_t_pop_title_with_explicit_icon() {
        let mut s = title_stream(false);
        slice(&mut s, b"\x1b[23;1t");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_csi_t_pop_title_with_index() {
        let mut s = title_stream(false);
        slice(&mut s, b"\x1b[23;0;5t");
        assert_eq!(s.handler.state, Some(5));
    }

    #[test]
    fn stream_csi_w_clear_tab_stops() {
        let mut s = stream(None::<ActionKey>, |st, a| *st = Some(a.key()));
        slice(&mut s, b"\x1b[2W");
        assert_eq!(s.handler.state, Some(ActionKey::TabClearCurrent));
        slice(&mut s, b"\x1b[5W");
        assert_eq!(s.handler.state, Some(ActionKey::TabClearAll));
    }

    #[test]
    fn stream_csi_w_tab_set() {
        let mut s = stream(None::<ActionKey>, |st, a| *st = Some(a.key()));
        slice(&mut s, b"\x1b[W");
        assert_eq!(s.handler.state, Some(ActionKey::TabSet));

        s.handler.state = None;
        slice(&mut s, b"\x1b[0W");
        assert_eq!(s.handler.state, Some(ActionKey::TabSet));

        s.handler.state = None;
        slice(&mut s, b"\x1b[>W");
        assert_eq!(s.handler.state, None);

        s.handler.state = None;
        slice(&mut s, b"\x1b[99W");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_csi_question_w_reset_tab_stops() {
        let mut s = stream(None::<ActionKey>, |st, a| *st = Some(a.key()));
        slice(&mut s, b"\x1b[?2W");
        assert_eq!(s.handler.state, None);

        slice(&mut s, b"\x1b[?5W");
        assert_eq!(s.handler.state, Some(ActionKey::TabReset));

        // Invalid and ignored by the handler
        s.handler.state = None;
        slice(&mut s, b"\x1b[?1;2;3W");
        assert_eq!(s.handler.state, None);
    }

    #[test]
    fn stream_sgr_with_17_plus_parameters_for_underline_color() {
        let mut s = stream(false, |called, a| {
            if let Action::SetAttribute(_) = a {
                *called = true;
            }
        });
        // Kakoune-style SGR with underline color as 17th parameter
        slice(
            &mut s,
            b"\x1b[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136;0m",
        );
        assert!(s.handler.state);
    }

    #[test]
    fn stream_tab_clear_with_overflowing_param() {
        // Regression test for a fuzz crash: a parameter that saturates to
        // 65535 doesn't fit the u8 TabClear enum.
        let mut s = stream(false, |called, a| {
            if matches!(a, Action::TabClearCurrent | Action::TabClearAll) {
                *called = true;
            }
        });
        slice(&mut s, b"\x1b[388888888888888888888888888888888888g\x1b[0m");
        assert!(!s.handler.state);
    }

    // Extra tests

    #[test]
    fn extra_slice_and_byte_paths_agree() {
        let input =
            "héllo \x1b[1;31mred\x1b[0m wörld \u{1f600}\r\n\x1b]2;title\x07\x1b(0x\x1bM\x1b[3 q"
                .as_bytes();
        let collect = |bytewise: bool| {
            let mut s = stream(Vec::<String>::new(), |v, a| v.push(format!("{a:?}")));
            if bytewise {
                for &c in input {
                    s.next(c).unwrap();
                }
            } else {
                s.next_slice(input).unwrap();
            }
            s.handler.state
        };
        let a = collect(true);
        assert_eq!(a, collect(false));
        assert!(a.len() > 20);
    }

    #[test]
    fn extra_split_input_at_every_position() {
        let input = "a\u{1f600}\x1b[1;31;4:3mx\x1b]2;hi\x1b\\y\u{e9}".as_bytes();
        let run = |split: usize| {
            let mut s = Stream::new_alloc(F {
                state: Vec::<String>::new(),
                f: |v: &mut Vec<String>, a: Action<'_>| v.push(format!("{a:?}")),
            });
            s.next_slice(&input[..split]).unwrap();
            s.next_slice(&input[split..]).unwrap();
            s.handler.state
        };
        let whole = run(input.len());
        for i in 0..input.len() {
            assert_eq!(run(i), whole, "split at {i}");
        }
    }

    #[test]
    fn extra_c1_executes_as_esc() {
        let mut s = stream(Vec::<ActionKey>::new(), |v, a| v.push(a.key()));
        // 0x8D = RI (ESC M); decoded from UTF-8 as U+008D it is printed
        // since the stream only routes bytes < 0x10 to execute.
        s.execute(0x8D).unwrap();
        assert_eq!(s.handler.state, vec![ActionKey::ReverseIndex]);
    }

    #[test]
    fn extra_cnl_cpl_emit_two_actions() {
        let mut s = stream(Vec::<ActionKey>::new(), |v, a| v.push(a.key()));
        slice(&mut s, b"\x1b[2E\x1b[F");
        assert_eq!(
            s.handler.state,
            vec![
                ActionKey::CursorDown,
                ActionKey::CarriageReturn,
                ActionKey::CursorUp,
                ActionKey::CarriageReturn
            ]
        );
    }

    #[test]
    fn extra_osc_alloc_flag() {
        let mut s = Stream::new_alloc(F {
            state: 0u32,
            f: |n: &mut u32, a: Action<'_>| {
                if matches!(a, Action::ColorOperation(_)) {
                    *n += 1;
                }
            },
        });
        slice(&mut s, b"\x1b]10;?\x1b\\");
        assert_eq!(s.handler.state, 1);

        let mut s = stream(0u32, |n, a| {
            if matches!(a, Action::ColorOperation(_)) {
                *n += 1;
            }
        });
        slice(&mut s, b"\x1b]10;?\x1b\\");
        assert_eq!(s.handler.state, 0);
    }

    #[test]
    fn extra_vt_raw_can_skip() {
        struct H(Vec<ActionKey>);
        impl Handler for H {
            type Error = Infallible;
            fn vt(&mut self, a: Action<'_>) -> Result<(), Infallible> {
                self.0.push(a.key());
                Ok(())
            }
            fn vt_raw(&mut self, a: &parser::Action) -> Result<bool, Infallible> {
                Ok(matches!(a, parser::Action::CsiDispatch(_)))
            }
        }
        let mut s = Stream::new(H(Vec::new()));
        slice(&mut s, b"\x1b[5Cx");
        assert_eq!(s.handler.0, vec![ActionKey::Print]);
    }

    #[test]
    fn extra_handler_error_propagates() {
        struct H;
        impl Handler for H {
            type Error = &'static str;
            fn vt(&mut self, _: Action<'_>) -> Result<(), &'static str> {
                Err("boom")
            }
        }
        let mut s = Stream::new(H);
        assert_eq!(s.next_slice(b"a"), Err("boom"));
    }

    #[test]
    fn extra_mut_ref_handler() {
        let mut inner = F {
            state: 0u32,
            f: |n: &mut u32, a: Action<'_>| {
                if matches!(a, Action::Print(_)) {
                    *n += 1;
                }
            },
        };
        let mut s = Stream::new(&mut inner);
        slice(&mut s, b"abc");
        drop(s);
        assert_eq!(inner.state, 3);
    }
}
