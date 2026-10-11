//! CSI command enums (ZIG/src/terminal/csi.zig).

/// Modes for the ED CSI command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EraseDisplay {
    Below = 0,
    Above = 1,
    Complete = 2,
    Scrollback = 3,
    /// This is an extension added by Kitty to move the viewport into the
    /// scrollback and then erase the display.
    ScrollComplete = 22,
}

impl EraseDisplay {
    /// Non-exhaustive in spirit: unknown values yield `None`.
    pub fn from_int(v: u16) -> Option<Self> {
        Some(match v {
            0 => Self::Below,
            1 => Self::Above,
            2 => Self::Complete,
            3 => Self::Scrollback,
            22 => Self::ScrollComplete,
            _ => return None,
        })
    }
}

/// Modes for the EL CSI command. Non-exhaustive in Zig so that conversion
/// never fails since the inputs are user-generated; `Other` carries the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseLine {
    Right,
    Left,
    Complete,
    RightUnlessPendingWrap,
    Other(u8),
}

impl EraseLine {
    pub fn from_int(v: u8) -> Self {
        match v {
            0 => Self::Right,
            1 => Self::Left,
            2 => Self::Complete,
            4 => Self::RightUnlessPendingWrap,
            v => Self::Other(v),
        }
    }
}

/// Modes for the TBC (tab clear) command. Non-exhaustive like [`EraseLine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabClear {
    Current,
    All,
    Other(u8),
}

impl TabClear {
    pub fn from_int(v: u8) -> Self {
        match v {
            0 => Self::Current,
            3 => Self::All,
            v => Self::Other(v),
        }
    }
}

/// Style formats for terminal size reports (XTWINOPS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeReportStyle {
    Csi14T,
    Csi16T,
    Csi18T,
    Csi21T,
}

/// XTWINOPS CSI 22/23
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitlePushPop {
    pub op: TitlePushPopOp,
    pub index: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlePushPopOp {
    Push,
    Pop,
}
