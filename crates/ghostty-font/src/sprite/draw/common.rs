//! This file contains a set of useful helper functions
//! and types for drawing our sprite font glyphs. These
//! are generally applicable to multiple sets of glyphs
//! rather than being single-use. Port of `sprite/draw/common.zig`.

use crate::metrics::Metrics;
use crate::sprite::canvas::{Canvas, Color, DrawError};

/// A function that draws a glyph on the provided canvas.
pub type DrawFn = fn(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError>;

/// A range of codepoints (inclusive) and the function that draws them.
/// (Zig discovers these at comptime from `draw<MIN>_<MAX>` function names.)
#[derive(Clone, Copy)]
pub struct Range {
    pub min: u32,
    pub max: u32,
    pub draw: DrawFn,
}

// Utility names for common fractions
pub const ONE_EIGHTH: f64 = 0.125;
pub const ONE_QUARTER: f64 = 0.25;
pub const ONE_THIRD: f64 = 1.0 / 3.0;
pub const THREE_EIGHTHS: f64 = 0.375;
pub const HALF: f64 = 0.5;
pub const FIVE_EIGHTHS: f64 = 0.625;
pub const TWO_THIRDS: f64 = 2.0 / 3.0;
pub const THREE_QUARTERS: f64 = 0.75;
pub const SEVEN_EIGHTHS: f64 = 0.875;

/// The thickness of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thickness {
    SuperLight,
    Light,
    Heavy,
}

impl Thickness {
    /// Calculate the real height of a line based on its
    /// thickness and a base thickness value. The base
    /// thickness value is expected to be in pixels.
    pub fn height(self, base: u32) -> u32 {
        match self {
            Thickness::SuperLight => (base / 2).max(1),
            Thickness::Light => base,
            Thickness::Heavy => base * 2,
        }
    }
}

/// Shades. Zig's is a non-exhaustive `enum(u8)`, so any alpha value is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shade(pub u8);

impl Shade {
    pub const OFF: Shade = Shade(0x00);
    pub const LIGHT: Shade = Shade(0x40);
    pub const MEDIUM: Shade = Shade(0x80);
    pub const DARK: Shade = Shade(0xc0);
    pub const ON: Shade = Shade(0xff);

    /// The canvas color with the same alpha value.
    pub fn color(self) -> Color {
        Color(self.0)
    }
}

/// Applicable to any set of glyphs with features
/// that may be present or not in each quadrant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quads {
    pub tl: bool,
    pub tr: bool,
    pub bl: bool,
    pub br: bool,
}

/// A corner of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    Tl,
    Tr,
    Bl,
    Br,
}

/// An edge of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Left,
    Bottom,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Bottom,
    Middle,
}

/// Alignment of a figure within a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment {
    pub horizontal: HAlign,
    pub vertical: VAlign,
}

impl Alignment {
    pub const CENTER: Alignment = Alignment {
        horizontal: HAlign::Center,
        vertical: VAlign::Middle,
    };
    pub const UPPER: Alignment = Alignment {
        horizontal: HAlign::Center,
        vertical: VAlign::Top,
    };
    pub const LOWER: Alignment = Alignment {
        horizontal: HAlign::Center,
        vertical: VAlign::Bottom,
    };
    pub const LEFT: Alignment = Alignment {
        horizontal: HAlign::Left,
        vertical: VAlign::Middle,
    };
    pub const RIGHT: Alignment = Alignment {
        horizontal: HAlign::Right,
        vertical: VAlign::Middle,
    };

    pub const UPPER_LEFT: Alignment = Alignment {
        horizontal: HAlign::Left,
        vertical: VAlign::Top,
    };
    pub const UPPER_RIGHT: Alignment = Alignment {
        horizontal: HAlign::Right,
        vertical: VAlign::Top,
    };
    pub const LOWER_LEFT: Alignment = Alignment {
        horizontal: HAlign::Left,
        vertical: VAlign::Bottom,
    };
    pub const LOWER_RIGHT: Alignment = Alignment {
        horizontal: HAlign::Right,
        vertical: VAlign::Bottom,
    };

    pub const UPPER_CENTER: Alignment = Self::UPPER;
    pub const LOWER_CENTER: Alignment = Self::LOWER;
    pub const MIDDLE_LEFT: Alignment = Self::LEFT;
    pub const MIDDLE_RIGHT: Alignment = Self::RIGHT;
    pub const MIDDLE_CENTER: Alignment = Self::CENTER;

    pub const TOP: Alignment = Self::UPPER;
    pub const BOTTOM: Alignment = Self::LOWER;
    pub const CENTER_TOP: Alignment = Self::TOP;
    pub const CENTER_BOTTOM: Alignment = Self::BOTTOM;

    pub const TOP_LEFT: Alignment = Self::UPPER_LEFT;
    pub const TOP_RIGHT: Alignment = Self::UPPER_RIGHT;
    pub const BOTTOM_LEFT: Alignment = Self::LOWER_LEFT;
    pub const BOTTOM_RIGHT: Alignment = Self::LOWER_RIGHT;
}

impl Default for Alignment {
    fn default() -> Self {
        Alignment::CENTER
    }
}

/// A value that indicates some fraction across
/// the cell either horizontally or vertically.
///
/// This has some redundant names in it so that you can
/// use whichever one feels most semantically appropriate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fraction {
    // Names for the min edge
    Start,
    Left,
    Top,
    Zero,

    // Names based on eighths
    Eighth,
    OneEighth,
    TwoEighths,
    ThreeEighths,
    FourEighths,
    FiveEighths,
    SixEighths,
    SevenEighths,

    // Names based on quarters
    Quarter,
    OneQuarter,
    TwoQuarters,
    ThreeQuarters,

    // Names based on thirds
    Third,
    OneThird,
    TwoThirds,

    // Names based on halves
    Half,
    OneHalf,

    // Alternative names for 1/2
    Center,
    Middle,

    // Names for the max edge
    End,
    Right,
    Bottom,
    One,
    Full,
}

impl Fraction {
    /// This can be indexed to get the fraction for `i/8`.
    pub const EIGHTHS: [Fraction; 9] = [
        Fraction::Zero,
        Fraction::OneEighth,
        Fraction::TwoEighths,
        Fraction::ThreeEighths,
        Fraction::FourEighths,
        Fraction::FiveEighths,
        Fraction::SixEighths,
        Fraction::SevenEighths,
        Fraction::One,
    ];

    /// This can be indexed to get the fraction for `i/4`.
    pub const QUARTERS: [Fraction; 5] = [
        Fraction::Zero,
        Fraction::OneQuarter,
        Fraction::TwoQuarters,
        Fraction::ThreeQuarters,
        Fraction::One,
    ];

    /// This can be indexed to get the fraction for `i/3`.
    pub const THIRDS: [Fraction; 4] = [
        Fraction::Zero,
        Fraction::OneThird,
        Fraction::TwoThirds,
        Fraction::One,
    ];

    /// This can be indexed to get the fraction for `i/2`.
    pub const HALVES: [Fraction; 3] = [Fraction::Zero, Fraction::OneHalf, Fraction::One];

    /// Get the x position for this fraction across a particular
    /// size (width or height), assuming it will be used as the
    /// min (left/top) coordinate for a block.
    pub fn min(self, size: u32) -> i32 {
        let s = f64::from(size);
        // For min coordinates, we want to align with the complementary
        // fraction taken from the end, this ensures that rounding evens
        // out, so that for example, if `size` is `7`, and we're looking
        // at the `half` line, `size - round((1 - 0.5) * size)` => `3`;
        // whereas the max coordinate directly rounds, which means that
        // both `start` -> `half` and `half` -> `end` will be 4px, from
        // `0` -> `4` and `3` -> `7`.
        (s - ((1.0 - self.fraction()) * s).round()) as i32
    }

    /// Get the x position for this fraction across a particular
    /// size (width or height), assuming it will be used as the
    /// max (right/bottom) coordinate for a block.
    pub fn max(self, size: u32) -> i32 {
        let s = f64::from(size);
        // See explanation of why these are different in `min`.
        (self.fraction() * s).round() as i32
    }

    /// Get this fraction across a particular size (width/height).
    /// If you need an integer, use `min` or `max` instead, since
    /// they contain special logic for consistent alignment. This
    /// is for when you're drawing with paths and don't care about
    /// pixel alignment.
    pub fn float(self, size: u32) -> f64 {
        self.fraction() * f64::from(size)
    }

    /// Get a float for the fraction this represents.
    pub fn fraction(self) -> f64 {
        #[allow(clippy::enum_glob_use)]
        use Fraction::*;
        match self {
            Start | Left | Top | Zero => 0.0,
            Eighth | OneEighth => 0.125,
            Quarter | OneQuarter | TwoEighths => 0.25,
            Third | OneThird => 1.0 / 3.0,
            ThreeEighths => 0.375,
            Half | OneHalf | TwoQuarters | FourEighths | Center | Middle => 0.5,
            FiveEighths => 0.625,
            TwoThirds => 2.0 / 3.0,
            ThreeQuarters | SixEighths => 0.75,
            SevenEighths => 0.875,
            End | Right | Bottom | One | Full => 1.0,
        }
    }
}

/// Fill a section of the cell, specified by a
/// horizontal and vertical pair of fraction lines.
pub fn fill(
    metrics: &Metrics,
    canvas: &mut Canvas,
    x0: Fraction,
    x1: Fraction,
    y0: Fraction,
    y1: Fraction,
) {
    canvas.box_(
        x0.min(metrics.cell_width),
        y0.min(metrics.cell_height),
        x1.max(metrics.cell_width),
        y1.max(metrics.cell_height),
        Color::ON,
    );
}

/// Centered vertical line of the provided thickness.
pub fn vline_middle(metrics: &Metrics, canvas: &mut Canvas, thickness: Thickness) {
    let thick_px = thickness.height(metrics.box_thickness);
    vline(
        canvas,
        0,
        metrics.cell_height as i32,
        (metrics.cell_width.saturating_sub(thick_px) / 2) as i32,
        thick_px,
    );
}

/// Centered horizontal line of the provided thickness.
pub fn hline_middle(metrics: &Metrics, canvas: &mut Canvas, thickness: Thickness) {
    let thick_px = thickness.height(metrics.box_thickness);
    hline(
        canvas,
        0,
        metrics.cell_width as i32,
        (metrics.cell_height.saturating_sub(thick_px) / 2) as i32,
        thick_px,
    );
}

/// Vertical line with the left edge at `x`, between `y1` and `y2`.
pub fn vline(canvas: &mut Canvas, y1: i32, y2: i32, x: i32, thickness_px: u32) {
    canvas.box_(x, y1, x + thickness_px as i32, y2, Color::ON);
}

/// Horizontal line with the top edge at `y`, between `x1` and `x2`.
pub fn hline(canvas: &mut Canvas, x1: i32, x2: i32, y: i32, thickness_px: u32) {
    canvas.box_(x1, y, x2, y + thickness_px as i32, Color::ON);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_min_max_even_out() {
        // size 7, half: min = 7 - round(3.5) = 3, max = round(3.5) = 4
        assert_eq!(3, Fraction::Half.min(7));
        assert_eq!(4, Fraction::Half.max(7));
        assert_eq!(0, Fraction::Zero.min(7));
        assert_eq!(7, Fraction::Full.max(7));
    }

    #[test]
    fn thickness_heights() {
        assert_eq!(1, Thickness::SuperLight.height(1));
        assert_eq!(2, Thickness::SuperLight.height(4));
        assert_eq!(3, Thickness::Light.height(3));
        assert_eq!(6, Thickness::Heavy.height(3));
    }
}
