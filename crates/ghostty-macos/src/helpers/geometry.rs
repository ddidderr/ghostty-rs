//! Plain `CGPoint`/`CGSize`/`CGRect` stand-ins so frame math (NSScreen,
//! NSWindow, LastWindowPosition, fullscreen frames) is testable without
//! AppKit. Conversions to/from the real types live next to the AppKit code.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }
    #[must_use]
    pub fn min_x(&self) -> f64 {
        self.origin.x
    }
    #[must_use]
    pub fn min_y(&self) -> f64 {
        self.origin.y
    }
    #[must_use]
    pub fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }
    #[must_use]
    pub fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }
    #[must_use]
    pub fn width(&self) -> f64 {
        self.size.width
    }
    #[must_use]
    pub fn height(&self) -> f64 {
        self.size.height
    }
    /// `CGRect.contains(_ rect:)`.
    #[must_use]
    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.min_x() >= self.min_x()
            && other.max_x() <= self.max_x()
            && other.min_y() >= self.min_y()
            && other.max_y() <= self.max_y()
    }
}
