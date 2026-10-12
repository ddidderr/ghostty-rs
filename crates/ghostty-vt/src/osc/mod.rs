//! OSC (Operating System Command) related functions and types
//! (ZIG/src/terminal/osc.zig).
//!
//! OSC is another set of control sequences for terminal programs that start with
//! "ESC ]". Unlike CSI or standard ESC sequences, they may contain strings
//! and other irregular formatting so a dedicated parser is created to handle it.
//!
//! Rust port notes (W1):
//! - Zig `[:0]const u8` fields become owned `Vec<u8>` (no NUL terminator, not
//!   necessarily valid UTF-8).
//! - Zig's "fixed buffer vs. allocating writer" is a single `Vec<u8>` with an
//!   optional maximum length (`MAX_BUF`, which always reserves one byte for
//!   the NUL terminator the Zig code writes). The Zig `alloc: ?Allocator` is
//!   `alloc: bool` ("an allocator is available").
//! - `Parser::end` returns an owned `Command` instead of a pointer into the
//!   parser.

pub mod encoding;
pub mod parsers;

pub use parsers::{color, semantic_prompt};

use crate::kitty::color as kitty_color;

#[derive(Debug, Clone, PartialEq)]
pub enum Command<'a> {
    /// This generally shouldn't ever be set except as an initial zero value.
    /// Ignore it.
    Invalid,

    /// Set the window title of the terminal
    ///
    /// If title mode 0 is set text is expect to be hex encoded (i.e. utf-8
    /// with each code unit further encoded with two hex digits).
    ///
    /// If title mode 2 is set or the terminal is setup for unconditional
    /// utf-8 titles text is interpreted as utf-8. Else text is interpreted
    /// as latin1.
    ChangeWindowTitle(Bytes<'a>),

    /// Set the icon of the terminal window. The name of the icon is not
    /// well defined, so this is currently ignored by Ghostty at the time
    /// of writing this. We just parse it so that we don't get parse errors
    /// in the log.
    ChangeWindowIcon(Bytes<'a>),

    /// Semantic prompt command: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md
    SemanticPrompt(SemanticPrompt<'a>),

    /// Set or get clipboard contents. If data is null, then the current
    /// clipboard contents are sent to the pty. If data is set, this
    /// contents is set on the clipboard.
    ClipboardContents {
        kind: u8,
        data: Bytes<'a>,
    },

    /// OSC 7. Reports the current working directory of the shell. This is
    /// a moderately flawed escape sequence but one that many major terminals
    /// support so we also support it. To understand the flaws, read through
    /// this terminal-wg issue: https://gitlab.freedesktop.org/terminal-wg/specifications/-/issues/20
    ReportPwd {
        /// The reported pwd value. This is not checked for validity. It should
        /// be a file URL but it is up to the caller to utilize this value.
        value: Bytes<'a>,
    },

    /// OSC 22. Set the mouse shape. There doesn't seem to be a standard
    /// naming scheme for cursors but it looks like terminals such as Foot
    /// are moving towards using the W3C CSS cursor names. For OSC parsing,
    /// we just parse whatever string is given.
    MouseShape {
        value: Bytes<'a>,
    },

    /// OSC color operations to set, reset, or report color settings. Some OSCs
    /// allow multiple operations to be specified in a single OSC so we need a
    /// list-like datastructure to manage them.
    ///
    /// Currently, these OSCs are handled by `color_operation`:
    ///
    /// 4, 5, 10-19, 104, 105, 110-119
    ColorOperation {
        op: color::Operation,
        requests: color::List,
        terminator: Terminator,
    },

    /// Kitty color protocol, OSC 21
    /// https://sw.kovidgoyal.net/kitty/color-stack/#id1
    KittyColorProtocol(kitty_color::OSC),

    /// Show a desktop notification (OSC 9 or OSC 777)
    ShowDesktopNotification {
        title: Bytes<'a>,
        body: Bytes<'a>,
    },

    /// Start a hyperlink (OSC 8)
    HyperlinkStart {
        id: Option<Bytes<'a>>,
        uri: Bytes<'a>,
    },

    /// End a hyperlink (OSC 8)
    HyperlinkEnd,

    /// ConEmu sleep (OSC 9;1)
    ConemuSleep {
        duration_ms: u16,
    },

    /// ConEmu show GUI message box (OSC 9;2)
    ConemuShowMessageBox(Bytes<'a>),

    /// ConEmu change tab title (OSC 9;3)
    ConemuChangeTabTitle(ConemuChangeTabTitle<'a>),

    /// ConEmu progress report (OSC 9;4)
    ConemuProgressReport(ProgressReport),

    /// ConEmu wait input (OSC 9;5)
    ConemuWaitInput,

    /// ConEmu GUI macro (OSC 9;6)
    ConemuGuimacro(Bytes<'a>),

    /// ConEmu run process (OSC 9;7)
    ConemuRunProcess(Bytes<'a>),

    /// ConEmu output environment variable (OSC 9;8)
    ConemuOutputEnvironmentVariable(Bytes<'a>),

    /// ConEmu XTerm keyboard and output emulation (OSC 9;10)
    /// https://conemu.github.io/en/TerminalModes.html
    ConemuXtermEmulation {
        /// None => do not change
        /// Some(false) => turn off
        /// Some(true) => turn on
        keyboard: Option<bool>,
        /// None => do not change
        /// Some(false) => turn off
        /// Some(true) => turn on
        output: Option<bool>,
    },

    /// ConEmu comment (OSC 9;11)
    ConemuComment(Bytes<'a>),

    /// Kitty text sizing protocol (OSC 66)
    KittyTextSizing(parsers::kitty_text_sizing::OSC<'a>),

    KittyClipboardProtocol(KittyClipboardProtocol<'a>),

    /// OSC 3008. Hierarchical context signalling (UAPI spec).
    /// https://uapi-group.org/specifications/specs/osc_context/
    ContextSignal(parsers::context_signal::Command<'a>),
}

/// Payload bytes of a [`Command`]: borrowed from the parser's payload storage
/// (Zig: slices into the parser's buffer), so producing a command does not
/// allocate. Use [`Command::into_owned`] to keep one past the next parser
/// operation.
pub type Bytes<'a> = std::borrow::Cow<'a, [u8]>;

pub type SemanticPrompt<'a> = parsers::semantic_prompt::Command<'a>;
pub type KittyClipboardProtocol<'a> = parsers::kitty_clipboard_protocol::OSC<'a>;

/// Payload of `Command::ConemuChangeTabTitle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConemuChangeTabTitle<'a> {
    Reset,
    Value(Bytes<'a>),
}

/// The discriminant of a [`Command`] (Zig `Command.Key`). Order matches Zig.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Invalid,
    ChangeWindowTitle,
    ChangeWindowIcon,
    SemanticPrompt,
    ClipboardContents,
    ReportPwd,
    MouseShape,
    ColorOperation,
    KittyColorProtocol,
    ShowDesktopNotification,
    HyperlinkStart,
    HyperlinkEnd,
    ConemuSleep,
    ConemuShowMessageBox,
    ConemuChangeTabTitle,
    ConemuProgressReport,
    ConemuWaitInput,
    ConemuGuimacro,
    ConemuRunProcess,
    ConemuOutputEnvironmentVariable,
    ConemuXtermEmulation,
    ConemuComment,
    KittyTextSizing,
    KittyClipboardProtocol,
    ContextSignal,
}

fn own(b: Bytes<'_>) -> Bytes<'static> {
    Bytes::Owned(b.into_owned())
}

impl Command<'_> {
    /// Detach the command from the parser's storage (copies the payloads),
    /// so it can outlive the next parser operation.
    pub fn into_owned(self) -> Command<'static> {
        match self {
            Command::Invalid => Command::Invalid,
            Command::ChangeWindowTitle(v) => Command::ChangeWindowTitle(own(v)),
            Command::ChangeWindowIcon(v) => Command::ChangeWindowIcon(own(v)),
            Command::SemanticPrompt(v) => Command::SemanticPrompt(v.into_owned()),
            Command::ClipboardContents { kind, data } => Command::ClipboardContents {
                kind,
                data: own(data),
            },
            Command::ReportPwd { value } => Command::ReportPwd { value: own(value) },
            Command::MouseShape { value } => Command::MouseShape { value: own(value) },
            Command::ColorOperation {
                op,
                requests,
                terminator,
            } => Command::ColorOperation {
                op,
                requests,
                terminator,
            },
            Command::KittyColorProtocol(v) => Command::KittyColorProtocol(v),
            Command::ShowDesktopNotification { title, body } => Command::ShowDesktopNotification {
                title: own(title),
                body: own(body),
            },
            Command::HyperlinkStart { id, uri } => Command::HyperlinkStart {
                id: id.map(own),
                uri: own(uri),
            },
            Command::HyperlinkEnd => Command::HyperlinkEnd,
            Command::ConemuSleep { duration_ms } => Command::ConemuSleep { duration_ms },
            Command::ConemuShowMessageBox(v) => Command::ConemuShowMessageBox(own(v)),
            Command::ConemuChangeTabTitle(v) => Command::ConemuChangeTabTitle(match v {
                ConemuChangeTabTitle::Reset => ConemuChangeTabTitle::Reset,
                ConemuChangeTabTitle::Value(b) => ConemuChangeTabTitle::Value(own(b)),
            }),
            Command::ConemuProgressReport(v) => Command::ConemuProgressReport(v),
            Command::ConemuWaitInput => Command::ConemuWaitInput,
            Command::ConemuGuimacro(v) => Command::ConemuGuimacro(own(v)),
            Command::ConemuRunProcess(v) => Command::ConemuRunProcess(own(v)),
            Command::ConemuOutputEnvironmentVariable(v) => {
                Command::ConemuOutputEnvironmentVariable(own(v))
            }
            Command::ConemuXtermEmulation { keyboard, output } => {
                Command::ConemuXtermEmulation { keyboard, output }
            }
            Command::ConemuComment(v) => Command::ConemuComment(own(v)),
            Command::KittyTextSizing(v) => Command::KittyTextSizing(v.into_owned()),
            Command::KittyClipboardProtocol(v) => Command::KittyClipboardProtocol(v.into_owned()),
            Command::ContextSignal(v) => Command::ContextSignal(v.into_owned()),
        }
    }

    pub fn key(&self) -> Key {
        match self {
            Command::Invalid => Key::Invalid,
            Command::ChangeWindowTitle(_) => Key::ChangeWindowTitle,
            Command::ChangeWindowIcon(_) => Key::ChangeWindowIcon,
            Command::SemanticPrompt(_) => Key::SemanticPrompt,
            Command::ClipboardContents { .. } => Key::ClipboardContents,
            Command::ReportPwd { .. } => Key::ReportPwd,
            Command::MouseShape { .. } => Key::MouseShape,
            Command::ColorOperation { .. } => Key::ColorOperation,
            Command::KittyColorProtocol(_) => Key::KittyColorProtocol,
            Command::ShowDesktopNotification { .. } => Key::ShowDesktopNotification,
            Command::HyperlinkStart { .. } => Key::HyperlinkStart,
            Command::HyperlinkEnd => Key::HyperlinkEnd,
            Command::ConemuSleep { .. } => Key::ConemuSleep,
            Command::ConemuShowMessageBox(_) => Key::ConemuShowMessageBox,
            Command::ConemuChangeTabTitle(_) => Key::ConemuChangeTabTitle,
            Command::ConemuProgressReport(_) => Key::ConemuProgressReport,
            Command::ConemuWaitInput => Key::ConemuWaitInput,
            Command::ConemuGuimacro(_) => Key::ConemuGuimacro,
            Command::ConemuRunProcess(_) => Key::ConemuRunProcess,
            Command::ConemuOutputEnvironmentVariable(_) => Key::ConemuOutputEnvironmentVariable,
            Command::ConemuXtermEmulation { .. } => Key::ConemuXtermEmulation,
            Command::ConemuComment(_) => Key::ConemuComment,
            Command::KittyTextSizing(_) => Key::KittyTextSizing,
            Command::KittyClipboardProtocol(_) => Key::KittyClipboardProtocol,
            Command::ContextSignal(_) => Key::ContextSignal,
        }
    }
}

/// ConEmu progress report (OSC 9;4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressReport {
    pub state: ProgressState,
    pub progress: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Remove,
    Set,
    Error,
    Indeterminate,
    Pause,
}

/// The terminator used to end an OSC command. For OSC commands that demand
/// a response, we try to match the terminator used in the request since that
/// is most likely to be accepted by the calling program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terminator {
    /// The preferred string terminator is ESC followed by \
    #[default]
    St,

    /// Some applications and terminals use BELL (0x07) as the string terminator.
    Bel,
}

impl Terminator {
    /// Initialize the terminator based on the last byte seen. If the
    /// last byte is a BEL then we use BEL, otherwise we just assume ST.
    pub fn init(ch: Option<u8>) -> Terminator {
        match ch {
            Some(0x07) => Terminator::Bel,
            _ => Terminator::St,
        }
    }

    /// The terminator as a string.
    pub fn string(self) -> &'static [u8] {
        match self {
            Terminator::St => b"\x1b\\",
            Terminator::Bel => b"\x07",
        }
    }
}

impl std::fmt::Display for Terminator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The terminator is always ASCII.
        f.write_str(std::str::from_utf8(self.string()).unwrap_or(""))
    }
}

/// State of the OSC parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Start,
    Invalid,

    // OSC command prefixes. Not all of these are valid OSCs, but may be
    // needed to "bridge" to a valid OSC (e.g. to support OSC 777 we need to
    // have a state "77" even though there is no OSC 77).
    S0,
    S1,
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
    S9,
    S30,
    S300,
    S3008,
    S10,
    S11,
    S12,
    S13,
    S14,
    S15,
    S16,
    S17,
    S18,
    S19,
    S21,
    S22,
    S52,
    S55,
    S66,
    S77,
    S104,
    S110,
    S111,
    S112,
    S113,
    S114,
    S115,
    S116,
    S117,
    S118,
    S119,
    S133,
    S552,
    S777,
    S1337,
    S5522,
}

/// How OSC payload data is being accumulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writer {
    /// Not accumulating (still parsing the numeric prefix).
    None,
    /// Fixed-size buffer of `MAX_BUF` bytes.
    Fixed,
    /// Growable buffer.
    Allocating,
}

/// The payload storage ran out of memory (the allocator refused to grow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageFull;

/// Allocator abstraction for the parser's payload buffer (Zig: the
/// `Allocator` the parser was initialized with). The default storage is a
/// `Vec<u8>` on the global allocator; the C API supplies storage that is
/// allocated through the caller's `GhosttyAllocator`, so allocation limits and
/// failures of that allocator govern the parser's memory like they do in Zig.
///
/// A failed `try_extend`/`try_reserve` invalidates the OSC being parsed
/// (Zig: the writer's `error.WriteFailed` sets the state to invalid; a failed
/// initial allocating-writer allocation falls back to the fixed buffer).
pub trait ByteStorage: std::fmt::Debug + Send {
    /// The bytes accumulated so far.
    fn bytes(&self) -> &[u8];
    /// Make room for `additional` more bytes.
    fn try_reserve(&mut self, additional: usize) -> Result<(), StorageFull>;
    /// Append bytes; on error nothing is appended.
    fn try_extend(&mut self, bytes: &[u8]) -> Result<(), StorageFull>;
    /// Drop the contents and release any memory acquired by growth (Zig
    /// `reset` deinits the allocating writer).
    fn reset(&mut self);
    /// Number of accumulated bytes.
    fn len(&self) -> usize {
        self.bytes().len()
    }
    /// Whether nothing has been accumulated.
    fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }
}

impl ByteStorage for Vec<u8> {
    fn bytes(&self) -> &[u8] {
        self
    }
    fn try_reserve(&mut self, additional: usize) -> Result<(), StorageFull> {
        Vec::try_reserve(self, additional).map_err(|_| StorageFull)
    }
    fn try_extend(&mut self, bytes: &[u8]) -> Result<(), StorageFull> {
        Vec::try_reserve(self, bytes.len()).map_err(|_| StorageFull)?;
        self.extend_from_slice(bytes);
        Ok(())
    }
    fn reset(&mut self) {
        self.clear();
    }
}

/// Maximum size of a "normal" OSC.
pub const MAX_BUF: usize = 2048;

/// The OSC parser. `S` is the payload storage: a `Vec<u8>` on the global
/// allocator by default, or caller-allocated storage (C API), in which case
/// the parser holds it inline (no extra allocation).
#[derive(Debug, Clone)]
pub struct Parser<S: ByteStorage = Vec<u8>> {
    /// Whether an allocator is available, i.e. whether data longer than
    /// MAX_BUF may be accepted for some commands (e.g. OSC 52), and whether
    /// commands that require an allocator are processed. (Zig:
    /// `alloc: ?Allocator`.)
    pub alloc: bool,

    /// Current state of the parser.
    pub state: State,

    /// Buffer for temporary storage of OSC data
    buffer: S,
    writer: Writer,
}

impl Default for Parser {
    fn default() -> Self {
        Self::init(false)
    }
}

impl Parser {
    pub fn init(alloc: bool) -> Parser {
        Parser {
            alloc,
            state: State::Start,
            buffer: Vec::new(),
            writer: Writer::None,
        }
    }
}

impl<S: ByteStorage> Parser<S> {
    /// Maximum size of a "normal" OSC.
    pub const MAX_BUF: usize = MAX_BUF;

    /// Like [`Parser::init`] with an allocator-backed payload buffer
    /// (`alloc` is true: an allocator is available).
    pub fn with_storage(storage: S) -> Parser<S> {
        Parser {
            alloc: true,
            state: State::Start,
            buffer: storage,
            writer: Writer::None,
        }
    }

    /// Reset the parser state.
    pub fn reset(&mut self) {
        self.state = State::Start;
        self.buffer.reset();
        self.writer = Writer::None;
    }

    /// Make sure that we have an allocator. If we don't, set the state to
    /// invalid so that any additional OSC data is discarded.
    fn ensure_allocator(&mut self) -> bool {
        if self.alloc {
            return true;
        }
        log::warn!(
            "An allocator is required to process OSC {:?} but none was provided.",
            self.state
        );
        self.state = State::Invalid;
        false
    }

    /// Set up a fixed Writer to collect the rest of the OSC data.
    fn write_to_fixed(&mut self) {
        self.writer = Writer::Fixed;
    }

    /// Set up an allocating Writer to collect the rest of the OSC data. If we
    /// don't have an allocator, fall back to writing to a fixed buffer and
    /// hope that it's big enough.
    fn write_to_allocating(&mut self) {
        if !self.alloc {
            self.write_to_fixed();
            return;
        }
        // Zig: `Writer.Allocating.initCapacity(alloc, 2048) catch` falls back
        // to the fixed buffer.
        if self.buffer.try_reserve(Self::MAX_BUF).is_err() {
            self.write_to_fixed();
            return;
        }
        self.writer = Writer::Allocating;
    }

    /// Helper for the per-command parsers. Mirrors the Zig prologue
    /// `writer orelse invalid; writer.writeByte(0) catch invalid`: returns
    /// `None` (and marks the parser invalid) if no data was being accumulated
    /// or there was no room for the NUL terminator. On success the payload
    /// is available, borrowed and without copying, from [`Parser::payload`].
    pub fn data(&mut self) -> Option<()> {
        if self.writer == Writer::None {
            self.state = State::Invalid;
            return None;
        }
        // Reserve room for the NUL terminator.
        if self.writer == Writer::Fixed && self.buffer.len() >= Self::MAX_BUF {
            self.state = State::Invalid;
            return None;
        }
        Some(())
    }

    /// Whether `writer.writeByte(0)` would succeed: when the fixed buffer is
    /// full there is no room for the NUL terminator. Pure so that it can be
    /// asked while the payload is borrowed; the caller returns `None`, which
    /// invalidates the OSC.
    pub fn reserve_nul(&self) -> bool {
        !(self.writer == Writer::Fixed && self.buffer.len() >= Self::MAX_BUF)
    }

    /// Like [`Parser::data`] but without reserving room for the NUL
    /// terminator (for parsers that, like Zig's color parsers, read
    /// `writer.buffered()` directly).
    pub fn buffered(&mut self) -> Option<()> {
        if self.writer == Writer::None {
            self.state = State::Invalid;
            return None;
        }
        Some(())
    }

    /// The accumulated OSC payload (Zig `writer.buffered()`), borrowed from
    /// the parser's storage. Commands built from it borrow it too, which is
    /// why producing a command never allocates.
    pub fn payload(&self) -> &[u8] {
        self.buffer.bytes()
    }

    /// Whether any payload writer has been set up (Zig `parser.writer != null`).
    pub fn has_writer(&self) -> bool {
        self.writer != Writer::None
    }

    /// Append to the buffer; an allocation failure invalidates the parser
    /// (Zig: `error.WriteFailed` -> `.invalid`).
    #[inline]
    fn push_or_invalid(&mut self, bytes: &[u8]) {
        if self.buffer.try_extend(bytes).is_err() {
            self.state = State::Invalid;
        }
    }

    #[inline]
    fn write_byte(&mut self, c: u8) {
        match self.writer {
            Writer::None => unreachable!(),
            Writer::Fixed => {
                if self.buffer.len() >= Self::MAX_BUF {
                    // Overflow: discard any further input.
                    self.state = State::Invalid;
                } else {
                    self.push_or_invalid(&[c]);
                }
            }
            Writer::Allocating => self.push_or_invalid(&[c]),
        }
    }

    /// Consume the next character c and advance the parser state.
    ///
    /// The payload-accumulation fast path is inlined into callers; the
    /// numeric-prefix state machine lives in [`Parser::next_state`].
    #[inline]
    pub fn next(&mut self, c: u8) {
        // If the state becomes invalid for any reason, just discard
        // any further input.
        if self.state == State::Invalid {
            return;
        }

        // If a writer has been initialized, we just accumulate the rest of the
        // OSC sequence in the writer's buffer and skip the state machine.
        if self.writer != Writer::None {
            self.write_byte(c);
            return;
        }

        self.next_state(c);
    }

    /// Consume a run of characters; exactly equivalent to calling
    /// [`Parser::next`] for each byte, but payload bytes are appended in bulk.
    pub fn next_slice(&mut self, mut bytes: &[u8]) {
        while let Some((&c, rest)) = bytes.split_first() {
            if self.state == State::Invalid {
                return;
            }
            match self.writer {
                Writer::None => {
                    self.next_state(c);
                    bytes = rest;
                }
                Writer::Allocating => {
                    self.push_or_invalid(bytes);
                    return;
                }
                Writer::Fixed => {
                    let room = Self::MAX_BUF.saturating_sub(self.buffer.len());
                    if bytes.len() <= room {
                        self.push_or_invalid(bytes);
                    } else {
                        // Overflow: keep what fits, discard the rest.
                        self.push_or_invalid(&bytes[..room]);
                        self.state = State::Invalid;
                    }
                    return;
                }
            }
        }
    }

    #[inline(never)]
    fn next_state(&mut self, c: u8) {
        #[allow(clippy::enum_glob_use)]
        use State::*;

        match self.state {
            // handled above, so should never be here
            Invalid => unreachable!(),

            Start => {
                self.state = match c {
                    b'0' => S0,
                    b'1' => S1,
                    b'2' => S2,
                    b'3' => S3,
                    b'4' => S4,
                    b'5' => S5,
                    b'6' => S6,
                    b'7' => S7,
                    b'8' => S8,
                    b'9' => S9,
                    _ => Invalid,
                }
            }

            S3 => {
                self.state = match c {
                    b'0' => S30,
                    _ => Invalid,
                }
            }

            S30 => {
                self.state = match c {
                    b'0' => S300,
                    _ => Invalid,
                }
            }

            S300 => {
                self.state = match c {
                    b'8' => S3008,
                    _ => Invalid,
                }
            }

            S3008 => match c {
                b';' => self.write_to_fixed(),
                _ => self.state = Invalid,
            },

            S1 => match c {
                b';' => self.write_to_fixed(),
                b'0' => self.state = S10,
                b'1' => self.state = S11,
                b'2' => self.state = S12,
                b'3' => self.state = S13,
                b'4' => self.state = S14,
                b'5' => self.state = S15,
                b'6' => self.state = S16,
                b'7' => self.state = S17,
                b'8' => self.state = S18,
                b'9' => self.state = S19,
                _ => self.state = Invalid,
            },

            S10 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                b'4' => self.state = S104,
                _ => self.state = Invalid,
            },

            S104 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                _ => self.state = Invalid,
            },

            S11 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                b'0' => self.state = S110,
                b'1' => self.state = S111,
                b'2' => self.state = S112,
                b'3' => self.state = S113,
                b'4' => self.state = S114,
                b'5' => self.state = S115,
                b'6' => self.state = S116,
                b'7' => self.state = S117,
                b'8' => self.state = S118,
                b'9' => self.state = S119,
                _ => self.state = Invalid,
            },

            S4 | S12 | S14 | S15 | S16 | S17 | S18 | S19 | S21 | S110 | S111 | S112 | S113
            | S114 | S115 | S116 | S117 | S118 | S119 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                _ => self.state = Invalid,
            },

            S13 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                b'3' => self.state = S133,
                _ => self.state = Invalid,
            },

            S2 => match c {
                b';' => self.write_to_fixed(),
                b'1' => self.state = S21,
                b'2' => self.state = S22,
                _ => self.state = Invalid,
            },

            S5 => match c {
                b';' => {
                    if self.ensure_allocator() {
                        self.write_to_fixed();
                    }
                }
                b'2' => self.state = S52,
                b'5' => self.state = S55,
                _ => self.state = Invalid,
            },

            S6 => match c {
                b'6' => self.state = S66,
                _ => self.state = Invalid,
            },

            S52 | S66 => match c {
                b';' => self.write_to_allocating(),
                _ => self.state = Invalid,
            },

            S55 => match c {
                b'2' => self.state = S552,
                _ => self.state = Invalid,
            },

            S7 => match c {
                b';' => self.write_to_fixed(),
                b'7' => self.state = S77,
                _ => self.state = Invalid,
            },

            S77 => match c {
                b'7' => self.state = S777,
                _ => self.state = Invalid,
            },

            S133 => match c {
                b';' => self.write_to_fixed(),
                b'7' => self.state = S1337,
                _ => self.state = Invalid,
            },

            S552 => match c {
                b'2' => self.state = S5522,
                _ => self.state = Invalid,
            },

            S1337 => match c {
                b';' => self.write_to_fixed(),
                _ => self.state = Invalid,
            },

            S5522 => match c {
                b';' => self.write_to_allocating(),
                _ => self.state = Invalid,
            },

            S0 | S22 | S777 | S8 | S9 => match c {
                b';' => self.write_to_fixed(),
                _ => self.state = Invalid,
            },
        }
    }

    /// End the sequence and return the command, if any. If the return value
    /// is None, then no valid command was found. The optional terminator_ch
    /// is the final character in the OSC sequence. This is used to determine
    /// the response terminator.
    pub fn end(&mut self, terminator_ch: Option<u8>) -> Option<Command<'_>> {
        #[allow(clippy::enum_glob_use)]
        use State::*;
        match self.state {
            Start => None,

            Invalid => None,

            S0 | S2 => parsers::change_window_title::parse(self, terminator_ch),

            S1 => parsers::change_window_icon::parse(self, terminator_ch),

            S4 | S5 | S10 | S11 | S12 | S13 | S14 | S15 | S16 | S17 | S18 | S19 | S104 | S110
            | S111 | S112 | S113 | S114 | S115 | S116 | S117 | S118 | S119 => {
                parsers::color::parse(self, terminator_ch)
            }

            S7 => parsers::report_pwd::parse(self, terminator_ch),

            S8 => parsers::hyperlink::parse(self, terminator_ch),

            S9 => parsers::osc9::parse(self, terminator_ch),

            S21 => parsers::kitty_color::parse(self, terminator_ch),

            S22 => parsers::mouse_shape::parse(self, terminator_ch),

            S52 => parsers::clipboard_operation::parse(self, terminator_ch),

            S55 => None,

            S3 | S30 | S300 => None,

            S3008 => parsers::context_signal::parse(self, terminator_ch),

            S6 => None,

            S66 => parsers::kitty_text_sizing::parse(self, terminator_ch),

            S77 => None,

            S133 => parsers::semantic_prompt::parse(self, terminator_ch),

            S552 => None,

            S777 => parsers::rxvt_extension::parse(self, terminator_ch),

            S1337 => parsers::iterm2::parse(self, terminator_ch),

            S5522 => parsers::kitty_clipboard_protocol::parse(self, terminator_ch),
        }
    }
}

#[cfg(test)]
mod next_slice_tests {
    use super::*;

    /// Storage that holds at most `cap` bytes (an allocator with a limit).
    #[derive(Debug, Clone)]
    struct Limited {
        data: Vec<u8>,
        cap: usize,
    }
    impl ByteStorage for Limited {
        fn bytes(&self) -> &[u8] {
            &self.data
        }
        fn try_reserve(&mut self, additional: usize) -> Result<(), StorageFull> {
            if self.data.len() + additional > self.cap {
                return Err(StorageFull);
            }
            Ok(())
        }
        fn try_extend(&mut self, bytes: &[u8]) -> Result<(), StorageFull> {
            self.try_reserve(bytes.len())?;
            self.data.extend_from_slice(bytes);
            Ok(())
        }
        fn reset(&mut self) {
            self.data.clear();
        }
    }

    #[test]
    fn storage_failure_invalidates_osc() {
        let mut p = Parser::with_storage(Limited {
            data: Vec::new(),
            cap: 8,
        });
        for b in b"0;abcdefghijkl" {
            p.next(*b);
        }
        assert_eq!(p.state, State::Invalid);
        assert!(p.end(Some(0x07)).is_none());

        // Within the limit it parses as usual.
        p.reset();
        for b in b"0;abc" {
            p.next(*b);
        }
        let cmd = p.end(Some(0x07)).expect("command");
        assert!(matches!(cmd, Command::ChangeWindowTitle(t) if &t[..] == b"abc"));
    }

    #[test]
    fn next_slice_matches_next() {
        for alloc in [false, true] {
            for prefix in ["0;", "52;c;", "4;", "7;", "8;;", "99;", "x"] {
                for n in [0usize, 1, 5, 2040, 2046, 2047, 2048, 2049, 2050, 3000] {
                    let mut input = prefix.as_bytes().to_vec();
                    input.extend((0..n).map(|i| b'a' + (i % 26) as u8));
                    let mut a = Parser::init(alloc);
                    let mut b = Parser::init(alloc);
                    for &c in &input {
                        a.next(c);
                    }
                    let mid = input.len() / 2;
                    b.next_slice(&input[..mid]);
                    b.next_slice(&input[mid..]);
                    assert_eq!(a.state, b.state);
                    assert_eq!(a.buffer.bytes(), b.buffer.bytes());
                    assert_eq!(a.writer, b.writer);
                }
            }
        }
    }
}
