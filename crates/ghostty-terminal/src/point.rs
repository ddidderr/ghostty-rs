//! Coordinates relative to the active area, viewport, screen, or history.
/// Reference frame for a terminal coordinate.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Tag {
    /// Editable bottommost screen area, including unwritten rows.
    Active,
    /// Visible area, including user scrolling.
    Viewport,
    /// Entire screen including scrollback.
    Screen,
    /// Scrollback preceding the active area.
    History,
}
/// An x/y coordinate; history may exceed a single page's row count.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Coordinate {
    /// Column.
    pub x: u16,
    /// Row relative to the point's reference frame.
    pub y: u32,
}
impl Coordinate {
    /// Compare both coordinates.
    pub fn eql(self, other: Self) -> bool {
        self == other
    }
}
/// A coordinate with its reference frame.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Point {
    /// Editable screen coordinate.
    Active(Coordinate),
    /// Visible viewport coordinate.
    Viewport(Coordinate),
    /// Whole-screen coordinate.
    Screen(Coordinate),
    /// Scrollback coordinate.
    History(Coordinate),
}
impl Point {
    /// Construct a point in a given frame.
    pub const fn new(tag: Tag, coord: Coordinate) -> Self {
        match tag {
            Tag::Active => Self::Active(coord),
            Tag::Viewport => Self::Viewport(coord),
            Tag::Screen => Self::Screen(coord),
            Tag::History => Self::History(coord),
        }
    }
    /// Extract its coordinate.
    pub const fn coord(self) -> Coordinate {
        match self {
            Self::Active(v) | Self::Viewport(v) | Self::Screen(v) | Self::History(v) => v,
        }
    }
    /// Extract its reference frame.
    pub const fn tag(self) -> Tag {
        match self {
            Self::Active(_) => Tag::Active,
            Self::Viewport(_) => Tag::Viewport,
            Self::Screen(_) => Tag::Screen,
            Self::History(_) => Tag::History,
        }
    }
}
