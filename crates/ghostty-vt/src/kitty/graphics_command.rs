//! Kitty graphics protocol command parser
//! (ZIG/src/terminal/kitty/graphics_command.zig).

use std::collections::HashMap;

/// The key-value pairs for the control information for a command. The
/// keys are always single characters and the values are either single
/// characters or 32-bit unsigned integers.
///
/// For the value of this: if the value is a single printable ASCII character
/// it is the ASCII code. Otherwise, it is parsed as a 32-bit unsigned integer.
type Kv = HashMap<u8, u32>;

/// Errors from the command parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid format")]
    InvalidFormat,
    #[error("invalid data")]
    InvalidData,
    #[error("invalid character")]
    InvalidCharacter,
    #[error("overflow")]
    Overflow,
}

/// Mirrors `std.fmt.parseInt(T, buf, 10)` for 32-bit targets: optional sign,
/// at least one digit, range checked.
fn parse_int(buf: &[u8], signed: bool) -> Result<u32, Error> {
    // kv_temp is at most 11 bytes so an i64 always suffices unless there
    // are overflow digits, which we detect via checked arithmetic.
    let (neg, digits) = match buf.first() {
        Some(b'+') if buf.len() > 1 => (false, &buf[1..]),
        Some(b'-') if buf.len() > 1 => (true, &buf[1..]),
        _ => (false, buf),
    };
    if digits.is_empty() {
        return Err(Error::InvalidCharacter);
    }
    let mut v: i64 = 0;
    let mut overflow = false;
    for &c in digits {
        if !c.is_ascii_digit() {
            return Err(Error::InvalidCharacter);
        }
        match v
            .checked_mul(10)
            .and_then(|x| x.checked_add(i64::from(c - b'0')))
        {
            Some(x) => v = x,
            None => overflow = true,
        }
    }
    if overflow {
        return Err(Error::Overflow);
    }
    if neg {
        v = -v;
    }
    if signed {
        i32::try_from(v)
            .map(|x| x as u32)
            .map_err(|_| Error::Overflow)
    } else {
        u32::try_from(v).map_err(|_| Error::Overflow)
    }
}

/// Decode base64 payload data. The reference implementation uses simdutf,
/// which ignores ASCII whitespace and tolerates missing padding.
pub fn decode_base64(input: &[u8]) -> Result<Vec<u8>, Error> {
    use base64::{
        Engine,
        engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
    };
    const ENGINE: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let cleaned: Vec<u8> = input
        .iter()
        .copied()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
        .collect();
    let trimmed_len = cleaned
        .iter()
        .rposition(|&b| b != b'=')
        .map_or(0, |i| i + 1);
    ENGINE
        .decode(&cleaned[..trimmed_len])
        .map_err(|_| Error::InvalidData)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Parsing k/v pairs. The "ignore" variants are in that state
    /// but ignore any data because we know they're invalid.
    ControlKey,
    ControlKeyIgnore,
    ControlValue,
    ControlValueIgnore,

    /// Collecting the data payload blob.
    Data,
}

/// Command parser parses the Kitty graphics protocol escape sequence.
#[derive(Debug)]
pub struct Parser {
    /// The list of KV pairs that we're building up.
    kv: Kv,

    /// Buffer to store the key/value of a KV pair. The value of a KV pair is
    /// at most a 32-bit integer which at most is 10 characters (4294967295),
    /// plus one character for the sign bit on signed ints.
    kv_temp: [u8; 11],
    kv_temp_len: usize,
    /// Current kv key
    kv_current: u8,

    /// The bytes of the data payload.
    data: Vec<u8>,

    state: State,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            kv: HashMap::new(),
            kv_temp: [0; 11],
            kv_temp_len: 0,
            kv_current: 0,
            data: Vec::new(),
            state: State::ControlKey,
        }
    }

    /// Parse a complete command string.
    pub fn parse_string(data: &[u8]) -> Result<Command, Error> {
        let mut parser = Self::new();
        for &c in data {
            parser.feed(c)?;
        }
        parser.complete()
    }

    /// Feed a single byte to the parser.
    ///
    /// The first byte to start parsing should be the byte immediately following
    /// the "G" in the APC sequence, i.e. "\x1b_G123" the first byte should
    /// be "1".
    pub fn feed(&mut self, c: u8) -> Result<(), Error> {
        match self.state {
            State::ControlKey => match c {
                // '=' means the key is complete and we're moving to the value.
                b'=' => {
                    if self.kv_temp_len == 1 {
                        self.kv_current = self.kv_temp[0];
                        self.kv_temp_len = 0;
                        self.state = State::ControlValue;
                    } else {
                        // All control keys are a single character right now so
                        // if we're not a single character just ignore follow-up
                        // data.
                        self.state = State::ControlValueIgnore;
                        self.kv_temp_len = 0;
                    }
                }

                // This can be encountered if we have a sequence with no
                // control data, only payload data (i.e. "\x1b_G;<data>").
                //
                // Kitty treats this as valid so we do as well.
                b';' => self.state = State::Data,

                _ => self.accumulate_value(c, State::ControlKeyIgnore),
            },

            State::ControlKeyIgnore => {
                if c == b'=' {
                    self.state = State::ControlValueIgnore;
                }
            }

            State::ControlValue => match c {
                // move to next key
                b',' => self.finish_value(State::ControlKey)?,
                // move to data
                b';' => self.finish_value(State::Data)?,
                _ => self.accumulate_value(c, State::ControlValueIgnore),
            },

            State::ControlValueIgnore => match c {
                b',' => self.state = State::ControlKeyIgnore,
                b';' => self.state = State::Data,
                _ => {}
            },

            State::Data => self.data.push(c),
        }
        Ok(())
    }

    /// Complete the parsing. This must be called after all the
    /// bytes have been fed to the parser. The payload data is moved into the
    /// resulting command.
    pub fn complete(&mut self) -> Result<Command, Error> {
        match self.state {
            // We can't ever end in the control key state and be valid.
            // This means the command looked something like "a=1,b"
            State::ControlKey | State::ControlKeyIgnore => return Err(Error::InvalidFormat),

            // Some commands (i.e. placements) end without extra data so
            // we end in the value state. i.e. "a=1,b=2"
            State::ControlValue => self.finish_value(State::Data)?,
            State::ControlValueIgnore => {}

            // Most commands end in data, i.e. "a=1,b=2;1234"
            State::Data => {}
        }

        // Determine our action, which is always a single character.
        let action: u8 = match self.kv.get(&b'a') {
            None => b't',
            Some(&v) => u8::try_from(v).map_err(|_| Error::InvalidFormat)?,
        };
        let kv = &self.kv;
        let control = match action {
            b'q' => Control::Query(Transmission::parse(kv)?),
            b't' => Control::Transmit(Transmission::parse(kv)?),
            b'T' => Control::TransmitAndDisplay {
                transmission: Transmission::parse(kv)?,
                display: Display::parse(kv)?,
            },
            b'p' => Control::Display(Display::parse(kv)?),
            b'd' => Control::Delete(Delete::parse(kv)?),
            b'f' => Control::TransmitAnimationFrame(AnimationFrameLoading::parse(kv)?),
            b'a' => Control::ControlAnimation(AnimationControl::parse(kv)?),
            b'c' => Control::ComposeAnimation(AnimationFrameComposition::parse(kv)?),
            _ => return Err(Error::InvalidFormat),
        };

        // Determine our quiet value
        let quiet = match self.kv.get(&b'q') {
            Some(0) => Quiet::No,
            Some(1) => Quiet::Ok,
            Some(2) => Quiet::Failures,
            Some(_) => return Err(Error::InvalidFormat),
            None => Quiet::No,
        };

        Ok(Command {
            control,
            quiet,
            data: self.decode_data()?,
        })
    }

    /// Decodes the payload data from base64 and returns it. This consumes
    /// the collected payload bytes.
    fn decode_data(&mut self) -> Result<Vec<u8>, Error> {
        if self.data.is_empty() {
            return Ok(Vec::new());
        }
        let data = std::mem::take(&mut self.data);
        decode_base64(&data).inspect_err(|e| {
            log::warn!("failed to decode base64 payload data: {e}");
        })
    }

    fn accumulate_value(&mut self, c: u8, overflow_state: State) {
        let idx = self.kv_temp_len;
        self.kv_temp_len += 1;
        if self.kv_temp_len > self.kv_temp.len() {
            self.state = overflow_state;
            self.kv_temp_len = 0;
            return;
        }
        self.kv_temp[idx] = c;
    }

    fn finish_value(&mut self, next_state: State) -> Result<(), Error> {
        // We can move states right away, we don't use it.
        self.state = next_state;

        // Check for ASCII chars first
        if self.kv_temp_len == 1 {
            let c = self.kv_temp[0];
            if !c.is_ascii_digit() {
                self.kv.insert(self.kv_current, u32::from(c));
                self.kv_temp_len = 0;
                return Ok(());
            }
        }

        // Handle integer fields, parsing signed fields accordingly. We still
        // store the fields as u32 as they can be bitcast back later during
        // building of the higher-level command tree.
        let buf = &self.kv_temp[..self.kv_temp_len];
        let v = match self.kv_current {
            b'z' | b'H' | b'V' => parse_int(buf, true)?,
            _ => parse_int(buf, false)?,
        };
        self.kv.insert(self.kv_current, v);

        // Clear our temp buffer
        self.kv_temp_len = 0;
        Ok(())
    }
}

/// Represents a possible response to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub message: String,
}

impl Default for Response {
    fn default() -> Self {
        Self {
            id: 0,
            image_number: 0,
            placement_id: 0,
            message: "OK".to_string(),
        }
    }
}

impl Response {
    pub fn encode(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        // We only encode a result if we have either an id or an image number.
        if self.id == 0 && self.image_number == 0 {
            return Ok(());
        }

        // Used to cheaply keep track if we need to add a comma before
        // the next key-value pair.
        let mut prior = false;

        writer.write_all(b"\x1b_G")?;
        if self.id > 0 {
            prior = true;
            write!(writer, "i={}", self.id)?;
        }
        if self.image_number > 0 {
            if prior {
                writer.write_all(b",")?;
            } else {
                prior = true;
            }
            write!(writer, "I={}", self.image_number)?;
        }
        if self.placement_id > 0 {
            if prior {
                writer.write_all(b",")?;
            }
            write!(writer, "p={}", self.placement_id)?;
        }
        writer.write_all(b";")?;
        writer.write_all(self.message.as_bytes())?;
        writer.write_all(b"\x1b\\")?;
        Ok(())
    }

    /// Returns true if this response is not an error.
    pub fn ok(&self) -> bool {
        self.message == "OK"
    }

    /// Empty response
    pub fn empty(&self) -> bool {
        self.id == 0 && self.image_number == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub control: Control,
    pub quiet: Quiet,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// q
    Query,
    /// t
    Transmit,
    /// T
    TransmitAndDisplay,
    /// p
    Display,
    /// d
    Delete,
    /// f
    TransmitAnimationFrame,
    /// a
    ControlAnimation,
    /// c
    ComposeAnimation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quiet {
    /// 0
    #[default]
    No,
    /// 1
    Ok,
    /// 2
    Failures,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Query(Transmission),
    Transmit(Transmission),
    TransmitAndDisplay {
        transmission: Transmission,
        display: Display,
    },
    Display(Display),
    Delete(Delete),
    TransmitAnimationFrame(AnimationFrameLoading),
    ControlAnimation(AnimationControl),
    ComposeAnimation(AnimationFrameComposition),
}

impl Control {
    pub fn action(&self) -> Action {
        match self {
            Control::Query(_) => Action::Query,
            Control::Transmit(_) => Action::Transmit,
            Control::TransmitAndDisplay { .. } => Action::TransmitAndDisplay,
            Control::Display(_) => Action::Display,
            Control::Delete(_) => Action::Delete,
            Control::TransmitAnimationFrame(_) => Action::TransmitAnimationFrame,
            Control::ControlAnimation(_) => Action::ControlAnimation,
            Control::ComposeAnimation(_) => Action::ComposeAnimation,
        }
    }
}

impl Command {
    /// Take ownership over the data in this command, leaving it empty.
    pub fn to_owned_data(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.data)
    }

    /// Returns the transmission data if it has any.
    pub fn transmission(&self) -> Option<Transmission> {
        match &self.control {
            Control::Query(t) | Control::Transmit(t) => Some(*t),
            Control::TransmitAndDisplay { transmission, .. } => Some(*transmission),
            _ => None,
        }
    }

    /// Returns the display data if it has any.
    pub fn display(&self) -> Option<Display> {
        match &self.control {
            Control::Display(d) => Some(*d),
            Control::TransmitAndDisplay { display, .. } => Some(*display),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transmission {
    /// f
    pub format: TransmissionFormat,
    /// t
    pub medium: TransmissionMedium,
    /// s
    pub width: u32,
    /// v
    pub height: u32,
    /// S
    pub size: u32,
    /// O
    pub offset: u32,
    /// i
    pub image_id: u32,
    /// I
    pub image_number: u32,
    /// p
    pub placement_id: u32,
    /// o
    pub compression: TransmissionCompression,
    /// m
    pub more_chunks: bool,
}

impl Default for Transmission {
    fn default() -> Self {
        Self {
            format: TransmissionFormat::Rgba,
            medium: TransmissionMedium::Direct,
            width: 0,
            height: 0,
            size: 0,
            offset: 0,
            image_id: 0,
            image_number: 0,
            placement_id: 0,
            compression: TransmissionCompression::None,
            more_chunks: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionFormat {
    /// 24
    Rgb,
    /// 32
    Rgba,
    /// 100
    Png,

    // The following are not supported directly via the protocol
    // but they are formats that a png may decode to that we
    // support.
    GrayAlpha,
    Gray,
}

impl TransmissionFormat {
    /// Bytes per pixel.
    ///
    /// # Panics
    /// Panics for `Png`, which must be validated before.
    pub fn bpp(self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::GrayAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
            Self::Png => unreachable!("png must be validated before"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionMedium {
    /// d
    Direct,
    /// f
    File,
    /// t
    TemporaryFile,
    /// s
    SharedMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionCompression {
    None,
    /// z
    ZlibDeflate,
}

fn get(kv: &Kv, k: u8) -> Option<u32> {
    kv.get(&k).copied()
}

impl Transmission {
    fn parse(kv: &Kv) -> Result<Transmission, Error> {
        let mut result = Transmission::default();
        if let Some(v) = get(kv, b'f') {
            result.format = match v {
                24 => TransmissionFormat::Rgb,
                32 => TransmissionFormat::Rgba,
                100 => TransmissionFormat::Png,
                _ => return Err(Error::InvalidFormat),
            };
        }

        if let Some(v) = get(kv, b't') {
            let c = u8::try_from(v).map_err(|_| Error::InvalidFormat)?;
            result.medium = match c {
                b'd' => TransmissionMedium::Direct,
                b'f' => TransmissionMedium::File,
                b't' => TransmissionMedium::TemporaryFile,
                b's' => TransmissionMedium::SharedMemory,
                _ => return Err(Error::InvalidFormat),
            };
        }

        if let Some(v) = get(kv, b's') {
            result.width = v;
        }
        if let Some(v) = get(kv, b'v') {
            result.height = v;
        }
        if let Some(v) = get(kv, b'S') {
            result.size = v;
        }
        if let Some(v) = get(kv, b'O') {
            result.offset = v;
        }
        if let Some(v) = get(kv, b'i') {
            result.image_id = v;
        }
        if let Some(v) = get(kv, b'I') {
            result.image_number = v;
        }
        if let Some(v) = get(kv, b'p') {
            result.placement_id = v;
        }

        if let Some(v) = get(kv, b'o') {
            let c = u8::try_from(v).map_err(|_| Error::InvalidFormat)?;
            result.compression = match c {
                b'z' => TransmissionCompression::ZlibDeflate,
                _ => return Err(Error::InvalidFormat),
            };
        }

        // If the transmission medium is a local-only medium, ignore the "m"
        // key. The Kitty graphics protocol specification does not explicitly
        // call out this behavior (although the "m" key is only mentioned in
        // connection with remote clients) but that's how it's implemented in
        // Kitty and at least one client (mpv) relies on this behavior when
        // using the shared memory transmission medium.
        //
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-transmission-medium
        // https://github.com/kovidgoyal/kitty/blob/ccc3bee9af794f332b4e9adcd714a649f639c397/kitty/graphics.c#L547-L592
        if result.medium == TransmissionMedium::Direct
            && let Some(v) = get(kv, b'm')
        {
            result.more_chunks = v > 0;
        }

        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Display {
    /// i
    pub image_id: u32,
    /// I
    pub image_number: u32,
    /// p
    pub placement_id: u32,
    /// x
    pub x: u32,
    /// y
    pub y: u32,
    /// w
    pub width: u32,
    /// h
    pub height: u32,
    /// X
    pub x_offset: u32,
    /// Y
    pub y_offset: u32,
    /// c
    pub columns: u32,
    /// r
    pub rows: u32,
    /// C
    pub cursor_movement: CursorMovement,
    /// U
    pub virtual_placement: bool,
    /// P
    pub parent_id: u32,
    /// Q
    pub parent_placement_id: u32,
    /// H
    pub horizontal_offset: i32,
    /// V
    pub vertical_offset: i32,
    /// z
    pub z: i32,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            image_id: 0,
            image_number: 0,
            placement_id: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            x_offset: 0,
            y_offset: 0,
            columns: 0,
            rows: 0,
            cursor_movement: CursorMovement::After,
            virtual_placement: false,
            parent_id: 0,
            parent_placement_id: 0,
            horizontal_offset: 0,
            vertical_offset: 0,
            z: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMovement {
    /// 0
    After,
    /// 1
    None,
}

impl Display {
    fn parse(kv: &Kv) -> Result<Display, Error> {
        let mut r = Display::default();

        if let Some(v) = get(kv, b'i') {
            r.image_id = v;
        }
        if let Some(v) = get(kv, b'I') {
            r.image_number = v;
        }
        if let Some(v) = get(kv, b'p') {
            r.placement_id = v;
        }
        if let Some(v) = get(kv, b'x') {
            r.x = v;
        }
        if let Some(v) = get(kv, b'y') {
            r.y = v;
        }
        if let Some(v) = get(kv, b'w') {
            r.width = v;
        }
        if let Some(v) = get(kv, b'h') {
            r.height = v;
        }
        if let Some(v) = get(kv, b'X') {
            r.x_offset = v;
        }
        if let Some(v) = get(kv, b'Y') {
            r.y_offset = v;
        }
        if let Some(v) = get(kv, b'c') {
            r.columns = v;
        }
        if let Some(v) = get(kv, b'r') {
            r.rows = v;
        }
        if let Some(v) = get(kv, b'C') {
            r.cursor_movement = match v {
                0 => CursorMovement::After,
                1 => CursorMovement::None,
                _ => return Err(Error::InvalidFormat),
            };
        }
        if let Some(v) = get(kv, b'U') {
            r.virtual_placement = match v {
                0 => false,
                1 => true,
                _ => return Err(Error::InvalidFormat),
            };
        }
        if let Some(v) = get(kv, b'z') {
            // We can bitcast here because of how we parse it earlier.
            r.z = v as i32;
        }
        if let Some(v) = get(kv, b'P') {
            r.parent_id = v;
        }
        if let Some(v) = get(kv, b'Q') {
            r.parent_placement_id = v;
        }
        if let Some(v) = get(kv, b'H') {
            r.horizontal_offset = v as i32;
        }
        if let Some(v) = get(kv, b'V') {
            r.vertical_offset = v as i32;
        }

        Ok(r)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionMode {
    /// 0
    AlphaBlend,
    /// 1
    Overwrite,
}

fn composition_mode(v: u32) -> Result<CompositionMode, Error> {
    match v {
        0 => Ok(CompositionMode::AlphaBlend),
        1 => Ok(CompositionMode::Overwrite),
        _ => Err(Error::InvalidFormat),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationBackground {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl AnimationBackground {
    /// Equivalent of the Zig `@bitCast` from a `u32` (r is the low byte).
    pub fn from_int(v: u32) -> Self {
        Self {
            r: v as u8,
            g: (v >> 8) as u8,
            b: (v >> 16) as u8,
            a: (v >> 24) as u8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationFrameLoading {
    /// x
    pub x: u32,
    /// y
    pub y: u32,
    /// c
    pub create_frame: u32,
    /// r
    pub edit_frame: u32,
    /// z
    pub gap_ms: u32,
    /// X
    pub composition_mode: CompositionMode,
    /// Y
    pub background: AnimationBackground,
}

impl Default for AnimationFrameLoading {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            create_frame: 0,
            edit_frame: 0,
            gap_ms: 0,
            composition_mode: CompositionMode::AlphaBlend,
            background: AnimationBackground::default(),
        }
    }
}

impl AnimationFrameLoading {
    fn parse(kv: &Kv) -> Result<Self, Error> {
        let mut r = Self::default();
        if let Some(v) = get(kv, b'x') {
            r.x = v;
        }
        if let Some(v) = get(kv, b'y') {
            r.y = v;
        }
        if let Some(v) = get(kv, b'c') {
            r.create_frame = v;
        }
        if let Some(v) = get(kv, b'r') {
            r.edit_frame = v;
        }
        if let Some(v) = get(kv, b'z') {
            r.gap_ms = v;
        }
        if let Some(v) = get(kv, b'X') {
            r.composition_mode = composition_mode(v)?;
        }
        if let Some(v) = get(kv, b'Y') {
            r.background = AnimationBackground::from_int(v);
        }
        Ok(r)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationFrameComposition {
    /// c
    pub frame: u32,
    /// r
    pub edit_frame: u32,
    /// x
    pub x: u32,
    /// y
    pub y: u32,
    /// w
    pub width: u32,
    /// h
    pub height: u32,
    /// X
    pub left_edge: u32,
    /// Y
    pub top_edge: u32,
    /// C
    pub composition_mode: CompositionMode,
}

impl Default for AnimationFrameComposition {
    fn default() -> Self {
        Self {
            frame: 0,
            edit_frame: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            left_edge: 0,
            top_edge: 0,
            composition_mode: CompositionMode::AlphaBlend,
        }
    }
}

impl AnimationFrameComposition {
    fn parse(kv: &Kv) -> Result<Self, Error> {
        let mut r = Self::default();
        if let Some(v) = get(kv, b'c') {
            r.frame = v;
        }
        if let Some(v) = get(kv, b'r') {
            r.edit_frame = v;
        }
        if let Some(v) = get(kv, b'x') {
            r.x = v;
        }
        if let Some(v) = get(kv, b'y') {
            r.y = v;
        }
        if let Some(v) = get(kv, b'w') {
            r.width = v;
        }
        if let Some(v) = get(kv, b'h') {
            r.height = v;
        }
        if let Some(v) = get(kv, b'X') {
            r.left_edge = v;
        }
        if let Some(v) = get(kv, b'Y') {
            r.top_edge = v;
        }
        if let Some(v) = get(kv, b'C') {
            r.composition_mode = composition_mode(v)?;
        }
        Ok(r)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationAction {
    /// 0
    Invalid,
    /// 1
    Stop,
    /// 2
    RunWait,
    /// 3
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationControl {
    /// s
    pub action: AnimationAction,
    /// r
    pub frame: u32,
    /// z
    pub gap_ms: u32,
    /// c
    pub current_frame: u32,
    /// v
    pub loops: u32,
}

impl Default for AnimationControl {
    fn default() -> Self {
        Self {
            action: AnimationAction::Invalid,
            frame: 0,
            gap_ms: 0,
            current_frame: 0,
            loops: 0,
        }
    }
}

impl AnimationControl {
    fn parse(kv: &Kv) -> Result<Self, Error> {
        let mut r = Self::default();
        if let Some(v) = get(kv, b's') {
            r.action = match v {
                0 => AnimationAction::Invalid,
                1 => AnimationAction::Stop,
                2 => AnimationAction::RunWait,
                3 => AnimationAction::Run,
                _ => return Err(Error::InvalidFormat),
            };
        }
        if let Some(v) = get(kv, b'r') {
            r.frame = v;
        }
        if let Some(v) = get(kv, b'z') {
            r.gap_ms = v;
        }
        if let Some(v) = get(kv, b'c') {
            r.current_frame = v;
        }
        if let Some(v) = get(kv, b'v') {
            r.loops = v;
        }
        Ok(r)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delete {
    /// a/A
    All(bool),

    /// i/I
    Id {
        /// uppercase
        delete: bool,
        /// i
        image_id: u32,
        /// p
        placement_id: u32,
    },

    /// n/N
    Newest {
        delete: bool,
        /// I
        image_number: u32,
        /// p
        placement_id: u32,
    },

    /// c/C
    IntersectCursor(bool),

    /// f/F
    AnimationFrames(bool),

    /// p/P
    IntersectCell { delete: bool, x: u32, y: u32 },

    /// q/Q
    IntersectCellZ {
        delete: bool,
        x: u32,
        y: u32,
        z: i32,
    },

    /// r/R
    Range {
        delete: bool,
        /// x
        first: u32,
        /// y
        last: u32,
    },

    /// x/X
    Column { delete: bool, x: u32 },

    /// y/Y
    Row { delete: bool, y: u32 },

    /// z/Z
    Z { delete: bool, z: i32 },
}

impl Delete {
    fn parse(kv: &Kv) -> Result<Delete, Error> {
        let what: u8 = match get(kv, b'd') {
            None => b'a',
            Some(v) => u8::try_from(v).map_err(|_| Error::InvalidFormat)?,
        };
        let up = what.is_ascii_uppercase();
        let or0 = |k: u8| get(kv, k).unwrap_or(0);

        Ok(match what {
            b'a' | b'A' => Delete::All(what == b'A'),
            b'i' | b'I' => Delete::Id {
                delete: what == b'I',
                image_id: or0(b'i'),
                placement_id: or0(b'p'),
            },
            b'n' | b'N' => Delete::Newest {
                delete: what == b'N',
                image_number: or0(b'I'),
                placement_id: or0(b'p'),
            },
            b'c' | b'C' => Delete::IntersectCursor(what == b'C'),
            b'f' | b'F' => Delete::AnimationFrames(what == b'F'),
            b'p' | b'P' => Delete::IntersectCell {
                delete: up,
                x: or0(b'x'),
                y: or0(b'y'),
            },
            b'q' | b'Q' => Delete::IntersectCellZ {
                delete: up,
                x: or0(b'x'),
                y: or0(b'y'),
                // We can bitcast here because of how we parse it earlier.
                z: or0(b'z') as i32,
            },
            b'r' | b'R' => {
                let x = get(kv, b'x').ok_or(Error::InvalidFormat)?;
                let y = get(kv, b'y').ok_or(Error::InvalidFormat)?;
                if x > y {
                    return Err(Error::InvalidFormat);
                }
                Delete::Range {
                    delete: up,
                    first: x,
                    last: y,
                }
            }
            b'x' | b'X' => Delete::Column {
                delete: up,
                x: or0(b'x'),
            },
            b'y' | b'Y' => Delete::Row {
                delete: up,
                y: or0(b'y'),
            },
            b'z' | b'Z' => Delete::Z {
                delete: up,
                z: or0(b'z') as i32,
            },
            _ => return Err(Error::InvalidFormat),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn parse(input: &str) -> Result<Command, Error> {
        let mut p = Parser::new();
        for c in input.bytes() {
            p.feed(c)?;
        }
        p.complete()
    }

    #[test]
    fn transmission_command() {
        let command = parse("f=24,s=10,v=20").unwrap();
        let Control::Transmit(v) = command.control else {
            panic!("expected transmit")
        };
        assert_eq!(v.format, TransmissionFormat::Rgb);
        assert_eq!(v.width, 10);
        assert_eq!(v.height, 20);
    }

    #[test]
    fn transmission_ignores_m_if_medium_is_not_direct() {
        let command = parse("a=t,t=t,m=1").unwrap();
        let Control::Transmit(v) = command.control else {
            panic!("expected transmit")
        };
        assert_eq!(v.medium, TransmissionMedium::TemporaryFile);
        assert!(!v.more_chunks);
    }

    #[test]
    fn transmission_respects_m_if_medium_is_direct() {
        let command = parse("a=t,t=d,m=1").unwrap();
        let Control::Transmit(v) = command.control else {
            panic!("expected transmit")
        };
        assert_eq!(v.medium, TransmissionMedium::Direct);
        assert!(v.more_chunks);
    }

    #[test]
    fn query_command() {
        let command = parse("i=31,s=1,v=1,a=q,t=d,f=24;QUFBQQ").unwrap();
        let Control::Query(v) = command.control else {
            panic!("expected query")
        };
        assert_eq!(v.medium, TransmissionMedium::Direct);
        assert_eq!(v.width, 1);
        assert_eq!(v.height, 1);
        assert_eq!(v.image_id, 31);
        assert_eq!(command.data, b"AAAA");
    }

    #[test]
    fn display_command() {
        let command = parse("a=p,U=1,i=31,c=80,r=120").unwrap();
        let Control::Display(v) = command.control else {
            panic!("expected display")
        };
        assert_eq!(v.columns, 80);
        assert_eq!(v.rows, 120);
        assert_eq!(v.image_id, 31);
        assert!(v.virtual_placement);
    }

    #[test]
    fn delete_command() {
        let command = parse("a=d,d=p,x=3,y=4").unwrap();
        let Control::Delete(Delete::IntersectCell { delete, x, y }) = command.control else {
            panic!("expected delete intersect_cell")
        };
        assert!(!delete);
        assert_eq!(x, 3);
        assert_eq!(y, 4);
    }

    #[test]
    fn no_control_data() {
        let command = parse(";QUFBQQ").unwrap();
        assert!(matches!(command.control, Control::Transmit(_)));
        assert_eq!(command.data, b"AAAA");
    }

    #[test]
    fn ignore_unknown_keys_long() {
        let command = parse("f=24,s=10,v=20,hello=world").unwrap();
        let Control::Transmit(v) = command.control else {
            panic!("expected transmit")
        };
        assert_eq!(v.format, TransmissionFormat::Rgb);
        assert_eq!(v.width, 10);
        assert_eq!(v.height, 20);
    }

    #[test]
    fn ignore_very_long_values() {
        let command = parse("f=24,s=10,v=2000000000000000000000000000000000000000").unwrap();
        let Control::Transmit(v) = command.control else {
            panic!("expected transmit")
        };
        assert_eq!(v.format, TransmissionFormat::Rgb);
        assert_eq!(v.width, 10);
        assert_eq!(v.height, 0);
    }

    #[test]
    fn ensure_very_large_negative_values_dont_get_skipped() {
        let command = parse("a=p,i=1,z=-2000000000").unwrap();
        let Control::Display(v) = command.control else {
            panic!("expected display")
        };
        assert_eq!(v.image_id, 1);
        assert_eq!(v.z, -2_000_000_000);
    }

    #[test]
    fn ensure_proper_overflow_error_for_u32() {
        assert_eq!(parse("a=p,i=10000000000").unwrap_err(), Error::Overflow);
    }

    #[test]
    fn ensure_proper_overflow_error_for_i32() {
        assert_eq!(parse("a=p,i=1,z=-9999999999").unwrap_err(), Error::Overflow);
    }

    #[test]
    fn all_i32_values() {
        let c = parse("a=p,i=1,z=-1").unwrap();
        let Control::Display(v) = c.control else {
            panic!()
        };
        assert_eq!((v.image_id, v.z), (1, -1));

        let c = parse("a=p,i=1,H=-1").unwrap();
        let Control::Display(v) = c.control else {
            panic!()
        };
        assert_eq!((v.image_id, v.horizontal_offset), (1, -1));

        let c = parse("a=p,i=1,V=-1").unwrap();
        let Control::Display(v) = c.control else {
            panic!()
        };
        assert_eq!((v.image_id, v.vertical_offset), (1, -1));
    }

    fn encode(r: &Response) -> Vec<u8> {
        let mut buf = Vec::new();
        r.encode(&mut buf).unwrap();
        buf
    }

    #[test]
    fn response_encode_nothing_without_id_or_image_number() {
        assert_eq!(encode(&Response::default()), b"");
    }

    #[test]
    fn response_encode_with_only_image_id() {
        let r = Response {
            id: 4,
            ..Default::default()
        };
        assert_eq!(encode(&r), b"\x1b_Gi=4;OK\x1b\\");
    }

    #[test]
    fn response_encode_with_only_image_number() {
        let r = Response {
            image_number: 4,
            ..Default::default()
        };
        assert_eq!(encode(&r), b"\x1b_GI=4;OK\x1b\\");
    }

    #[test]
    fn response_encode_with_image_id_and_number() {
        let r = Response {
            id: 12,
            image_number: 4,
            ..Default::default()
        };
        assert_eq!(encode(&r), b"\x1b_Gi=12,I=4;OK\x1b\\");
    }

    fn range(input: &str) -> Result<(bool, u32, u32), Error> {
        match parse(input)?.control {
            Control::Delete(Delete::Range {
                delete,
                first,
                last,
            }) => Ok((delete, first, last)),
            _ => panic!("expected range"),
        }
    }

    #[test]
    fn delete_range_command_1() {
        assert_eq!(range("a=d,d=r,x=3,y=4"), Ok((false, 3, 4)));
    }

    #[test]
    fn delete_range_command_2() {
        assert_eq!(range("a=d,d=R,x=5,y=11"), Ok((true, 5, 11)));
    }

    #[test]
    fn delete_range_command_3() {
        assert_eq!(range("a=d,d=R,x=5,y=4"), Err(Error::InvalidFormat));
    }

    #[test]
    fn delete_range_command_4() {
        assert_eq!(range("a=d,d=R,x=5"), Err(Error::InvalidFormat));
    }

    #[test]
    fn delete_range_command_5() {
        assert_eq!(range("a=d,d=R,y=5"), Err(Error::InvalidFormat));
    }

    #[test]
    fn base64_decode_variants() {
        assert_eq!(decode_base64(b"aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(decode_base64(b"aGVsbG8gd29ybGQ").unwrap(), b"hello world");
        assert!(decode_base64(b"!!!!").is_err());
    }

    #[test]
    fn parse_string_works() {
        let c = Parser::parse_string(b"a=q,i=1;QUFBQQ").unwrap();
        assert_eq!(c.data, b"AAAA");
    }
}
