//! ANSI types (ZIG/src/terminal/ansi.zig).

/// C0 (7-bit) control characters from ANSI.
///
/// This is not complete, control characters are only added to this
/// as the terminal emulator handles them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum C0 {
    /// Null
    Nul = 0x00,
    /// Start of heading
    Soh = 0x01,
    /// Start of text
    Stx = 0x02,
    /// Enquiry
    Enq = 0x05,
    /// Bell
    Bel = 0x07,
    /// Backspace
    Bs = 0x08,
    /// Horizontal tab
    Ht = 0x09,
    /// Line feed
    Lf = 0x0A,
    /// Vertical Tab
    Vt = 0x0B,
    /// Form feed
    Ff = 0x0C,
    /// Carriage return
    Cr = 0x0D,
    /// Shift out
    So = 0x0E,
    /// Shift in
    Si = 0x0F,
}

impl C0 {
    /// Convert from a raw byte; unknown values return `None` (the Zig enum
    /// is non-exhaustive).
    pub fn from_int(v: u8) -> Option<Self> {
        Some(match v {
            0x00 => Self::Nul,
            0x01 => Self::Soh,
            0x02 => Self::Stx,
            0x05 => Self::Enq,
            0x07 => Self::Bel,
            0x08 => Self::Bs,
            0x09 => Self::Ht,
            0x0A => Self::Lf,
            0x0B => Self::Vt,
            0x0C => Self::Ff,
            0x0D => Self::Cr,
            0x0E => Self::So,
            0x0F => Self::Si,
            _ => return None,
        })
    }
}

/// The SGR rendition aspects that can be set, sometimes known as attributes.
/// The value corresponds to the parameter value for the SGR command (ESC [ m).
/// Non-exhaustive in Zig; `Other` carries unknown values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenditionAspect {
    Default,
    Bold,
    DefaultFg,
    DefaultBg,
    Other(u16),
}

impl RenditionAspect {
    pub fn from_int(v: u16) -> Self {
        match v {
            0 => Self::Default,
            1 => Self::Bold,
            39 => Self::DefaultFg,
            49 => Self::DefaultBg,
            v => Self::Other(v),
        }
    }
}

/// The device attribute request type (ESC [ c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAttributeReq {
    /// Blank
    Primary,
    /// >
    Secondary,
    /// =
    Tertiary,
}

/// Possible cursor styles (ESC [ q)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Default,
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderline,
    SteadyUnderline,
    BlinkingBar,
    SteadyBar,
}

/// The status line type for DECSSDT. Non-exhaustive in Zig.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLineType {
    None,
    Indicator,
    HostWritable,
    Other(u16),
}

impl StatusLineType {
    pub fn from_int(v: u16) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Indicator,
            2 => Self::HostWritable,
            v => Self::Other(v),
        }
    }
}

/// The display to target for status updates (DECSASD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDisplay {
    Main,
    StatusLine,
}

/// The possible modify key formats to ESC[>{a};{b}m
/// Note: this is not complete, we should add more as we support more
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyKeyFormat {
    Legacy,
    CursorKeys,
    FunctionKeys,
    OtherKeysNone,
    OtherKeysNumericExcept,
    OtherKeysNumeric,
}

/// The protection modes that can be set for the terminal. See DECSCA and
/// ESC V, W.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedMode {
    Off,
    /// ESC V, W
    Iso,
    /// CSI Ps " q
    Dec,
}
