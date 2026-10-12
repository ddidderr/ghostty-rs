//! Platform-independent math extracted from the CoreText face
//! (`font/face/coretext.zig`) so it can be unit tested on every platform.
//!
//! The CoreText face itself only exists on Apple targets; everything here is
//! plain arithmetic on values that the face reads from CoreText / the
//! OpenType tables:
//!
//! * [`face_metrics`]: the `getMetrics` table-priority logic (OS/2 typo
//!   metrics vs. hhea vs. OS/2 win metrics, underline/strikethrough
//!   fallbacks, cap/ex heights).
//! * [`synthetic_bold_line_width`]: the synthetic bold stroke width.
//! * [`glyph_layout`]: the bearing / canvas-size computation of
//!   `renderGlyph`.

use super::{Constraint, ConstraintSize, GlyphSize, RenderOptions};
use crate::{
    metrics::FaceMetrics,
    opentype::{Hhea, OS2, Post},
};

/// The matrix applied to a regular font to auto-italicize it
/// (`Face.italic_skew`): `a=1, b=0, c=tan(15deg) (approx), d=1, tx=ty=0`.
pub const ITALIC_SKEW_C: f64 = 0.267949;

/// Metrics CoreText itself reports (used as fallbacks when a table is
/// missing). `descent` is CoreText's positive-down value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CtMetrics {
    pub ascent: f64,
    /// `CTFontGetDescent`: positive distance below the baseline.
    pub descent: f64,
    pub leading: f64,
    pub cap_height: f64,
    pub x_height: f64,
}

/// Measurements taken by drawing/measuring glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Measurements {
    /// The widest advance of the visible ASCII characters.
    pub cell_width: f64,
    /// Height of the overall bounding box of the visible ASCII characters.
    pub ascii_height: f64,
    /// Advance of U+6C34 (None if absent or considered invalid).
    pub ic_width: Option<f64>,
}

/// The line width used for synthetic bold given the font size in points.
///
/// To determine our synthetic bold line width we get a multiplier from the
/// font size in points. This is a heuristic that is based on the fact that
/// a line width of 1 looks good at a certain point size. We want to scale
/// that up roughly linearly with the font size.
pub fn synthetic_bold_line_width(points: f64) -> f64 {
    (points / 14.0).max(1.0)
}

/// Returns `(ascent, descent, line_gap)` in pixels, +Y up (`descent` is
/// normally negative), following the table priority of `getMetrics`.
pub fn vertical_metrics(
    hhea: Option<&Hhea>,
    os2: Option<&OS2>,
    px_per_unit: f64,
    ct: &CtMetrics,
) -> (f64, f64, f64) {
    // If we couldn't get the hhea table, rely on metrics from CoreText.
    let Some(hhea) = hhea else {
        return (ct.ascent, -ct.descent, ct.leading);
    };

    let hhea_ascent = f64::from(hhea.ascender);
    let hhea_descent = f64::from(hhea.descender);
    let hhea_line_gap = f64::from(hhea.lineGap);

    // If our font has no OS/2 table, then we just
    // blindly use the metrics from the hhea table.
    let Some(os2) = os2 else {
        return (
            hhea_ascent * px_per_unit,
            hhea_descent * px_per_unit,
            hhea_line_gap * px_per_unit,
        );
    };

    let os2_ascent = f64::from(os2.sTypoAscender);
    let os2_descent = f64::from(os2.sTypoDescender);
    let os2_line_gap = f64::from(os2.sTypoLineGap);

    // If the font says to use typo metrics, trust it.
    if os2.fsSelection.use_typo_metrics {
        return (
            os2_ascent * px_per_unit,
            os2_descent * px_per_unit,
            os2_line_gap * px_per_unit,
        );
    }

    // Otherwise we prefer the height metrics from 'hhea' if they
    // are available, or else OS/2 sTypo* metrics, and if all else
    // fails then we use OS/2 usWin* metrics.
    //
    // This is not "standard" behavior, but it's our best bet to
    // account for fonts being... just weird. It's pretty much what
    // FreeType does to get its generic ascent and descent metrics.
    if hhea.ascender != 0 || hhea.descender != 0 {
        return (
            hhea_ascent * px_per_unit,
            hhea_descent * px_per_unit,
            hhea_line_gap * px_per_unit,
        );
    }

    if os2_ascent != 0.0 || os2_descent != 0.0 {
        return (
            os2_ascent * px_per_unit,
            os2_descent * px_per_unit,
            os2_line_gap * px_per_unit,
        );
    }

    let win_ascent = f64::from(os2.usWinAscent);
    let win_descent = f64::from(os2.usWinDescent);
    (
        win_ascent * px_per_unit,
        // usWinDescent is *positive* -> down unlike sTypoDescender
        // and hhea.Descender, so we flip its sign to fix this.
        -win_descent * px_per_unit,
        0.0,
    )
}

/// Compute the `FaceMetrics` from the (optionally present) OpenType tables,
/// CoreText's own metrics and the measurements. `px_per_em` is
/// `CTFontGetSize` and `units_per_em` the `head` table value (or
/// `CTFontGetUnitsPerEm` if there is no `head`/`bhed` table).
pub fn face_metrics(
    px_per_em: f64,
    units_per_em: f64,
    hhea: Option<&Hhea>,
    os2: Option<&OS2>,
    post: Option<&Post>,
    ct: &CtMetrics,
    measured: Measurements,
) -> FaceMetrics {
    let px_per_unit = px_per_em / units_per_em;

    let (ascent, descent, line_gap) = vertical_metrics(hhea, os2, px_per_unit, ct);

    let (underline_position, underline_thickness) = match post {
        None => (None, None),
        Some(post) => {
            // Some fonts have degenerate 'post' tables where the underline
            // thickness (and often position) are 0. We consider them null
            // if this is the case and use our own fallbacks when we calculate.
            let has_broken_underline = post.underlineThickness == 0;

            // If the underline position isn't 0 then we do use it,
            // even if the thickness is't properly specified.
            let pos = if has_broken_underline && post.underlinePosition == 0 {
                None
            } else {
                Some(f64::from(post.underlinePosition) * px_per_unit)
            };

            let thick = if has_broken_underline {
                None
            } else {
                Some(f64::from(post.underlineThickness) * px_per_unit)
            };

            (pos, thick)
        }
    };

    // Similar logic to the underline above.
    let (strikethrough_position, strikethrough_thickness) = match os2 {
        None => (None, None),
        Some(os2) => {
            let has_broken_strikethrough = os2.yStrikeoutSize == 0;

            let pos = if has_broken_strikethrough && os2.yStrikeoutPosition == 0 {
                None
            } else {
                Some(f64::from(os2.yStrikeoutPosition) * px_per_unit)
            };

            let thick = if has_broken_strikethrough {
                None
            } else {
                Some(f64::from(os2.yStrikeoutSize) * px_per_unit)
            };

            (pos, thick)
        }
    };

    // We fall back to whatever CoreText does if the
    // OS/2 table doesn't specify a cap or ex height.
    let (cap_height, ex_height) = match os2 {
        None => (ct.cap_height, ct.x_height),
        Some(os2) => (
            os2.sCapHeight
                .map_or(ct.cap_height, |v| f64::from(v) * px_per_unit),
            os2.sxHeight
                .map_or(ct.x_height, |v| f64::from(v) * px_per_unit),
        ),
    };

    FaceMetrics {
        px_per_em,

        cell_width: measured.cell_width,

        ascent,
        descent,
        line_gap,

        underline_position,
        underline_thickness,

        strikethrough_position,
        strikethrough_thickness,

        cap_height: Some(cap_height),
        ex_height: Some(ex_height),
        ascii_height: Some(measured.ascii_height),
        ic_width: measured.ic_width,
    }
}

/// Is the advance of the "水" glyph usable as the ic width? If the advance
/// is less than the width of the actual glyph then it is probably wrong
/// (e.g. a CJK font patched by the nerd fonts patcher) and using it for size
/// normalization would make the font way too big.
pub fn ic_width_is_valid(glyph_width: f64, advance: f64) -> bool {
    glyph_width <= advance || glyph_width.is_nan() || advance.is_nan()
}

/// A CoreText bounding rect (origin is bottom-left, +Y up).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The result of laying out a glyph for rasterization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphLayout {
    /// Whole pixel bearings of the final glyph.
    pub px_x: i32,
    pub px_y: i32,
    /// Canvas size (includes padding and the fractional offset).
    pub px_width: u32,
    pub px_height: u32,
    /// Extra pixels around the glyph for font smoothing.
    pub canvas_padding: u32,
    /// Fractional part of the pixel bearings, added as an offset when
    /// rasterizing.
    pub frac_x: f64,
    pub frac_y: f64,
    /// Scale applied to the context so the glyph is stretched to the
    /// constrained size.
    pub scale_x: f64,
    pub scale_y: f64,
    /// The (possibly bold-adjusted) bounding rect origin; the glyph is drawn
    /// at the negated origin.
    pub rect_x: f64,
    pub rect_y: f64,
}

impl GlyphLayout {
    /// `Glyph.offset_y`: distance from the bottom of the cell to the top of
    /// the glyph's bounding box.
    pub fn offset_y(&self) -> i32 {
        self.px_y + self.px_height as i32
    }

    /// `Glyph.offset_x`: distance from the left of the cell to the left of
    /// the glyph's bounding box.
    pub fn offset_x(&self) -> i32 {
        self.px_x
    }
}

/// The bearing / size computation from `renderGlyph`.
///
/// `rect` is `CTFontGetBoundingRectsForGlyphs` for the glyph. Returns `None`
/// if the glyph is too small to render (the caller then returns a zero-sized
/// `Glyph`).
pub fn glyph_layout(
    mut rect: Rect,
    is_sbix: bool,
    synthetic_bold: Option<f64>,
    opts: &RenderOptions,
) -> Option<GlyphLayout> {
    // If we're rendering a synthetic bold then we will gain 50% of
    // the line width on every edge, which means we should increase
    // our width and height by the line width and subtract half from
    // our origin points.
    //
    // We don't add extra size if it's a sbix color font though,
    // since bitmaps aren't affected by synthetic bold.
    if !is_sbix && let Some(line_width) = synthetic_bold {
        rect.width += line_width;
        rect.height += line_width;
        rect.x -= line_width / 2.0;
        rect.y -= line_width / 2.0;
    }

    // If our rect is smaller than a quarter pixel in either axis
    // then it has no outlines or they're too small to render.
    if rect.width < 0.25 || rect.height < 0.25 {
        return None;
    }

    let metrics = &opts.grid_metrics;
    let cell_width = f64::from(metrics.cell_width);
    let cell_height = f64::from(metrics.cell_height);

    // Next we apply any constraints to get the final size of the glyph.
    let constraint: &Constraint = &opts.constraint;

    // We need to add the baseline position before passing to the constrain
    // function since it operates on cell-relative positions, not baseline.
    let cell_baseline = f64::from(metrics.cell_baseline);

    let glyph_size = constraint.constrain(
        GlyphSize {
            width: rect.width,
            height: rect.height,
            x: rect.x,
            y: rect.y + cell_baseline,
        },
        metrics,
        opts.constraint_width,
    );

    let mut x = glyph_size.x;
    let mut y = glyph_size.y;
    let mut width = glyph_size.width;
    let mut height = glyph_size.height;

    // We center all glyphs within the pixel-rounded and adjusted
    // cell width if it's larger than the face width, so that they
    // aren't weirdly off to the left.
    //
    // We don't do this if the glyph has a stretch constraint,
    // since in that case the position was already calculated with the
    // new cell width in mind.
    if constraint.size != ConstraintSize::Stretch {
        // We add half the difference to re-center.
        let dx = (cell_width - metrics.face_width) / 2.0;
        x += dx;
        if dx < 0.0 {
            // For negative diff (cell narrower than advance), we remove the
            // integer part and only keep the fractional adjustment needed
            // for consistent subpixel positioning.
            x -= dx.trunc();
        }
    }

    // If this is a bitmap glyph, it will always render as full pixels,
    // not fractional pixels, so we need to quantize its position and
    // size accordingly to align to full pixels so we get good results.
    if is_sbix {
        width = cell_width - (cell_width - width - x).round() - x.round();
        height = cell_height - (cell_height - height - y).round() - y.round();
        x = x.round();
        y = y.round();
    }

    // We make an assumption that font smoothing ("thicken")
    // adds no more than 1 extra pixel to any edge. We don't
    // add extra size if it's a sbix color font though, since
    // bitmaps aren't affected by smoothing.
    let canvas_padding: u32 = u32::from(opts.thicken && !is_sbix);

    // Our whole-pixel bearings for the final glyph.
    // The fractional portion will be included in the rasterized position.
    let px_x = (x.floor() as i32) - canvas_padding as i32;
    let px_y = (y.floor() as i32) - canvas_padding as i32;

    // We keep track of the fractional part of the pixel bearings, which
    // we will add as an offset when rasterizing to make sure we get the
    // correct sub-pixel position.
    let frac_x = x - x.floor();
    let frac_y = y - y.floor();

    // Add the fractional pixel to the width and height and take
    // the ceiling to get a canvas size that will definitely fit
    // our drawn glyph, including the fractional offset and font smoothing.
    let px_width = ((width + frac_x).ceil() as u32) + (2 * canvas_padding);
    let px_height = ((height + frac_y).ceil() as u32) + (2 * canvas_padding);

    Some(GlyphLayout {
        px_x,
        px_y,
        px_width,
        px_height,
        canvas_padding,
        frac_x,
        frac_y,
        scale_x: width / rect.width,
        scale_y: height / rect.height,
        rect_x: rect.x,
        rect_y: rect.y,
    })
}

/// `kCTFontTraitBold` bit of the symbolic traits.
pub const TRAIT_BOLD: u32 = 1 << 1;
/// `kCTFontTraitItalic` bit of the symbolic traits.
pub const TRAIT_ITALIC: u32 = 1;
/// `kCTFontTraitMonoSpace` bit of the symbolic traits.
pub const TRAIT_MONOSPACE: u32 = 1 << 10;
/// `kCTFontTraitColorGlyphs` bit of the symbolic traits.
pub const TRAIT_COLOR_GLYPHS: u32 = 1 << 13;

/// The symbolic traits value for a descriptor (0 if no trait is set, in
/// which case the traits attribute is not added; see
/// `Descriptor.toCoreTextDescriptor`).
pub fn symbolic_traits_value(bold: bool, italic: bool, monospace: bool) -> u32 {
    (if bold { TRAIT_BOLD } else { 0 })
        | (if italic { TRAIT_ITALIC } else { 0 })
        | (if monospace { TRAIT_MONOSPACE } else { 0 })
}

/// Number of bytes per pixel and whether the glyph is rendered in color,
/// for the atlas format check in `renderGlyph`.
pub fn bitmap_depth(is_color: bool) -> u32 {
    if is_color { 4 } else { 1 }
}

/// Decode a UTF-32 codepoint into UTF-16 code units
/// (`stringGetSurrogatePairForLongCharacter`). Returns the units and how many
/// are valid (1 or 2). Codepoints outside the Unicode range yield a single
/// replacement unit.
pub fn utf16_units(cp: u32) -> ([u16; 2], usize) {
    match char::from_u32(cp) {
        Some(c) => {
            let mut buf = [0u16; 2];
            let n = c.encode_utf16(&mut buf).len();
            (buf, n)
        }
        None => ([0xFFFD, 0], 1),
    }
}

/// The score used to sort CoreText discovery results, as a plain integer
/// (`Score.int()` in discovery.zig). Fields from least to most significant:
/// `glyph_count: u16`, `fuzzy_style: u8`, `bold`, `italic`, `exact_style`,
/// `monospace`, `codepoint` (1 bit each). Higher is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Score {
    pub glyph_count: u16,
    pub fuzzy_style: u8,
    pub bold: bool,
    pub italic: bool,
    pub exact_style: bool,
    pub monospace: bool,
    pub codepoint: bool,
}

impl Score {
    pub fn int(self) -> u32 {
        u32::from(self.glyph_count)
            | (u32::from(self.fuzzy_style) << 16)
            | (u32::from(self.bold) << 24)
            | (u32::from(self.italic) << 25)
            | (u32::from(self.exact_style) << 26)
            | (u32::from(self.monospace) << 27)
            | (u32::from(self.codepoint) << 28)
    }
}

/// The style-name strings the discovery score matches against (the first
/// is used for the exact match, all are used for the fuzzy match).
pub fn desired_styles(style: Option<&str>, bold: bool, italic: bool) -> Vec<&str> {
    if let Some(s) = style {
        return vec![s];
    }

    // If we don't have an explicitly desired style name, we base
    // it on the bold and italic properties, this isn't ideal since
    // fonts may use style names other than these, but it helps in
    // some edge cases.
    if bold {
        if italic {
            return vec!["bold italic", "bold", "italic", "oblique"];
        }
        return vec!["bold", "upright"];
    } else if italic {
        return vec!["italic", "regular", "oblique"];
    }
    vec!["regular", "upright"]
}

/// Compute `(exact_style, fuzzy_style)` for a font's style string against
/// the desired styles (see `Score.score`).
pub fn style_scores(style_str: &str, desired: &[&str]) -> (bool, u8) {
    let exact = style_str.eq_ignore_ascii_case(desired[0]);

    // Our "fuzzy match" score is 0 if the desired style isn't present
    // in the string, otherwise we give higher priority for styles that
    // have fewer characters not in the desired_styles list.
    let mut fuzzy: u8 = style_str.len().min(255) as u8;
    let lower = style_str.to_ascii_lowercase();
    for s in desired {
        if lower.contains(&s.to_ascii_lowercase()) {
            fuzzy = fuzzy.saturating_sub(s.len().min(255) as u8);
        }
    }
    (exact, u8::MAX.saturating_sub(fuzzy))
}

/// Derive `(is_bold, is_italic)` for a variation axis list entry, following
/// the thresholds in `Score.score`. `axes` yields `(name, value)`.
pub fn bold_italic_from_axes(
    mut is_bold: bool,
    mut is_italic: bool,
    axes: impl IntoIterator<Item = (String, f64)>,
) -> (bool, bool) {
    // If we see the 'ital' value then we ignore 'slnt'.
    let mut ital_seen = false;
    for (name, val) in axes {
        if name == "wght" {
            // Somewhat subjective threshold, we consider fonts
            // bold if they have a 'wght' set greater than 600.
            is_bold = val > 600.0;
            continue;
        }
        if name == "ital" {
            is_italic = val > 0.5;
            ital_seen = true;
            continue;
        }
        if !ital_seen && name == "slnt" {
            // Arbitrary threshold of anything more than a 5
            // degree clockwise slant is considered italic.
            is_italic = val <= -5.0;
        }
    }
    (is_bold, is_italic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        face::{DesiredSize, RenderOptions},
        metrics::Metrics,
        opentype::{
            os2::FSSelection,
            sfnt::{Fixed, Version16Dot16},
        },
    };

    fn hhea(ascender: i16, descender: i16, line_gap: i16) -> Hhea {
        Hhea {
            majorVersion: 1,
            minorVersion: 0,
            ascender,
            descender,
            lineGap: line_gap,
            advanceWidthMax: 0,
            minLeftSideBearing: 0,
            minRightSideBearing: 0,
            xMaxExtent: 0,
            caretSlopeRise: 0,
            caretSlopeRun: 0,
            caretOffset: 0,
            reserved0: 0,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            metricDataFormat: 0,
            numberOfHMetrics: 0,
        }
    }

    fn os2(typo: (i16, i16, i16), win: (u16, u16), use_typo: bool) -> OS2 {
        OS2 {
            version: 4,
            xAvgCharWidth: 0,
            usWeightClass: 400,
            usWidthClass: 5,
            fsType: 0,
            ySubscriptXSize: 0,
            ySubscriptYSize: 0,
            ySubscriptXOffset: 0,
            ySubscriptYOffset: 0,
            ySuperscriptXSize: 0,
            ySuperscriptYSize: 0,
            ySuperscriptXOffset: 0,
            ySuperscriptYOffset: 0,
            yStrikeoutSize: 50,
            yStrikeoutPosition: 300,
            sFamilyClass: 0,
            panose: [0; 10],
            ulUnicodeRange1: 0,
            ulUnicodeRange2: 0,
            ulUnicodeRange3: 0,
            ulUnicodeRange4: 0,
            achVendID: [0; 4],
            fsSelection: FSSelection {
                use_typo_metrics: use_typo,
                ..FSSelection::from_u16(0)
            },
            usFirstCharIndex: 0,
            usLastCharIndex: 0,
            sTypoAscender: typo.0,
            sTypoDescender: typo.1,
            sTypoLineGap: typo.2,
            usWinAscent: win.0,
            usWinDescent: win.1,
            ulCodePageRange1: None,
            ulCodePageRange2: None,
            sxHeight: Some(500),
            sCapHeight: Some(700),
            usDefaultChar: None,
            usBreakChar: None,
            usMaxContext: None,
            usLowerOpticalPointSize: None,
            usUpperOpticalPointSize: None,
        }
    }

    fn post(pos: i16, thick: i16) -> Post {
        Post {
            version: Version16Dot16::from_u32(0x0002_0000),
            italicAngle: Fixed(0),
            underlinePosition: pos,
            underlineThickness: thick,
            isFixedPitch: 1,
            minMemType42: 0,
            maxMemType42: 0,
            minMemType1: 0,
            maxMemType1: 0,
        }
    }

    #[test]
    fn symbolic_traits() {
        assert_eq!(symbolic_traits_value(false, false, false), 0);
        assert_eq!(symbolic_traits_value(true, false, false), 2);
        assert_eq!(symbolic_traits_value(false, true, false), 1);
        assert_eq!(symbolic_traits_value(true, true, true), 2 | 1 | 1024);
    }

    #[test]
    fn bold_line_width() {
        assert_eq!(synthetic_bold_line_width(12.0), 1.0);
        assert_eq!(synthetic_bold_line_width(14.0), 1.0);
        assert_eq!(synthetic_bold_line_width(28.0), 2.0);
    }

    #[test]
    fn vertical_no_hhea_uses_coretext() {
        let ct = CtMetrics {
            ascent: 10.0,
            descent: 3.0,
            leading: 1.0,
            ..Default::default()
        };
        assert_eq!(vertical_metrics(None, None, 0.01, &ct), (10.0, -3.0, 1.0));
    }

    #[test]
    fn vertical_hhea_only() {
        let h = hhea(800, -200, 100);
        let (a, d, g) = vertical_metrics(Some(&h), None, 0.01, &CtMetrics::default());
        assert_eq!((a, d, g), (8.0, -2.0, 1.0));
    }

    #[test]
    fn vertical_typo_when_flagged() {
        let h = hhea(800, -200, 100);
        let o = os2((900, -300, 0), (1000, 400), true);
        let (a, d, g) = vertical_metrics(Some(&h), Some(&o), 0.01, &CtMetrics::default());
        assert_eq!((a, d, g), (9.0, -3.0, 0.0));
    }

    #[test]
    fn vertical_prefers_hhea_then_typo_then_win() {
        let o = os2((900, -300, 20), (1000, 400), false);
        // hhea non-zero wins over typo.
        let h = hhea(800, -200, 100);
        let (a, ..) = vertical_metrics(Some(&h), Some(&o), 0.01, &CtMetrics::default());
        assert_eq!(a, 8.0);
        // hhea zero -> typo.
        let h = hhea(0, 0, 0);
        let (a, d, g) = vertical_metrics(Some(&h), Some(&o), 0.01, &CtMetrics::default());
        assert_eq!((a, d, g), (9.0, -3.0, 0.2));
        // both zero -> win metrics with flipped descent sign.
        let o = os2((0, 0, 20), (1000, 400), false);
        let (a, d, g) = vertical_metrics(Some(&h), Some(&o), 0.01, &CtMetrics::default());
        assert_eq!((a, d, g), (10.0, -4.0, 0.0));
    }

    #[test]
    fn metrics_underline_and_strikeout() {
        let h = hhea(800, -200, 0);
        let o = os2((0, 0, 0), (0, 0), false);
        let p = post(-100, 50);
        let m = face_metrics(
            10.0,
            1000.0,
            Some(&h),
            Some(&o),
            Some(&p),
            &CtMetrics::default(),
            Measurements {
                cell_width: 6.0,
                ascii_height: 9.0,
                ic_width: Some(12.0),
            },
        );
        assert_eq!(m.underline_position, Some(-1.0));
        assert_eq!(m.underline_thickness, Some(0.5));
        assert_eq!(m.strikethrough_position, Some(3.0));
        assert_eq!(m.strikethrough_thickness, Some(0.5));
        assert_eq!(m.cap_height, Some(7.0));
        assert_eq!(m.ex_height, Some(5.0));
        assert_eq!(m.cell_width, 6.0);
        assert_eq!(m.ascii_height, Some(9.0));
        assert_eq!(m.ic_width, Some(12.0));
    }

    #[test]
    fn metrics_broken_underline_is_none() {
        let p = post(0, 0);
        let m = face_metrics(
            10.0,
            1000.0,
            None,
            None,
            Some(&p),
            &CtMetrics {
                cap_height: 7.0,
                x_height: 5.0,
                ..Default::default()
            },
            Measurements::default(),
        );
        assert_eq!(m.underline_position, None);
        assert_eq!(m.underline_thickness, None);
        // position set but thickness broken: keep position only
        let p = post(-100, 0);
        let m = face_metrics(
            10.0,
            1000.0,
            None,
            None,
            Some(&p),
            &CtMetrics::default(),
            Measurements::default(),
        );
        assert_eq!(m.underline_position, Some(-1.0));
        assert_eq!(m.underline_thickness, None);
        assert_eq!(m.strikethrough_position, None);
        // no OS/2: CoreText cap/ex heights.
        assert_eq!(m.cap_height, Some(0.0));
    }

    #[test]
    fn ic_width_validity() {
        assert!(ic_width_is_valid(10.0, 12.0));
        assert!(ic_width_is_valid(12.0, 12.0));
        assert!(!ic_width_is_valid(13.0, 12.0));
    }

    fn opts() -> RenderOptions {
        let mut m = Metrics::init();
        m.cell_width = 10;
        m.cell_height = 20;
        m.cell_baseline = 4;
        m.face_width = 10.0;
        m.face_height = 20.0;
        let _ = DesiredSize::new(12.0);
        RenderOptions::new(m)
    }

    #[test]
    fn layout_tiny_glyph_is_none() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.2,
            height: 5.0,
        };
        assert!(glyph_layout(r, false, None, &opts()).is_none());
    }

    #[test]
    fn layout_basic() {
        let r = Rect {
            x: 1.5,
            y: -2.25,
            width: 6.0,
            height: 10.0,
        };
        let l = glyph_layout(r, false, None, &opts()).unwrap();
        assert_eq!(l.px_x, 1);
        // y is cell-relative: -2.25 + cell_baseline (4) = 1.75
        assert_eq!(l.px_y, 1);
        assert!((l.frac_x - 0.5).abs() < 1e-9);
        assert!((l.frac_y - 0.75).abs() < 1e-9);
        assert_eq!(l.px_width, 7);
        assert_eq!(l.px_height, 11);
        assert_eq!(l.canvas_padding, 0);
        assert_eq!(l.scale_x, 1.0);
        assert_eq!(l.offset_y(), 12);
        assert_eq!(l.offset_x(), 1);
    }

    #[test]
    fn layout_thicken_pads_canvas() {
        let r = Rect {
            x: 1.0,
            y: 0.0,
            width: 5.0,
            height: 5.0,
        };
        let mut o = opts();
        o.thicken = true;
        let l = glyph_layout(r, false, None, &o).unwrap();
        assert_eq!(l.canvas_padding, 1);
        assert_eq!(l.px_x, 0);
        assert_eq!(l.px_width, 7);
        // sbix glyphs never pad
        let l = glyph_layout(r, true, None, &o).unwrap();
        assert_eq!(l.canvas_padding, 0);
    }

    #[test]
    fn layout_synthetic_bold_grows_rect() {
        let r = Rect {
            x: 2.0,
            y: 2.0,
            width: 4.0,
            height: 4.0,
        };
        let l = glyph_layout(r, false, Some(2.0), &opts()).unwrap();
        assert_eq!(l.rect_x, 1.0);
        assert_eq!(l.rect_y, 1.0);
        assert_eq!(l.px_width, 6);
        // sbix ignores synthetic bold
        let l = glyph_layout(r, true, Some(2.0), &opts()).unwrap();
        assert_eq!(l.rect_x, 2.0);
    }

    #[test]
    fn layout_centers_in_wider_cell() {
        let mut o = opts();
        o.grid_metrics.cell_width = 12;
        o.grid_metrics.face_width = 10.0;
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 5.0,
            height: 5.0,
        };
        let l = glyph_layout(r, false, None, &o).unwrap();
        // dx = (12 - 10)/2 = 1
        assert_eq!(l.px_x, 1);
    }

    #[test]
    fn utf16() {
        assert_eq!(utf16_units('A' as u32), (['A' as u16, 0], 1));
        let (u, n) = utf16_units(0x1F978);
        assert_eq!(n, 2);
        assert_eq!(u, [0xD83E, 0xDD78]);
        assert_eq!(utf16_units(0xD800).1, 1);
    }

    #[test]
    fn score_ordering() {
        let a = Score {
            codepoint: true,
            ..Default::default()
        };
        let b = Score {
            monospace: true,
            glyph_count: u16::MAX,
            fuzzy_style: 255,
            bold: true,
            italic: true,
            exact_style: true,
            ..Default::default()
        };
        assert!(a.int() > b.int());
        let c = Score {
            glyph_count: 5,
            ..Default::default()
        };
        let d = Score {
            glyph_count: 7,
            ..Default::default()
        };
        assert!(d.int() > c.int());
    }

    #[test]
    fn desired_style_lists() {
        assert_eq!(desired_styles(Some("Light"), true, true), vec!["Light"]);
        assert_eq!(
            desired_styles(None, true, true),
            vec!["bold italic", "bold", "italic", "oblique"]
        );
        assert_eq!(desired_styles(None, true, false), vec!["bold", "upright"]);
        assert_eq!(
            desired_styles(None, false, true),
            vec!["italic", "regular", "oblique"]
        );
        assert_eq!(
            desired_styles(None, false, false),
            vec!["regular", "upright"]
        );
    }

    #[test]
    fn style_score_fuzzy_and_exact() {
        let d = desired_styles(None, false, false);
        let (exact, fuzzy) = style_scores("Regular", &d);
        assert!(exact);
        assert_eq!(fuzzy, 255);
        let (exact, fuzzy) = style_scores("Semi Regular Extra", &d);
        assert!(!exact);
        // len 18, "regular" (7) subtracted => 11 => 255 - 11
        assert_eq!(fuzzy, 255 - 11);
        let (_, fuzzy) = style_scores("Light", &d);
        assert_eq!(fuzzy, 255 - 5);
    }

    #[test]
    fn axes_bold_italic() {
        let r = bold_italic_from_axes(
            false,
            false,
            [("wght".to_owned(), 700.0), ("slnt".to_owned(), -10.0)],
        );
        assert_eq!(r, (true, true));
        // ital seen => slnt ignored
        let r = bold_italic_from_axes(
            true,
            true,
            [("ital".to_owned(), 0.0), ("slnt".to_owned(), -10.0)],
        );
        assert_eq!(r, (true, false));
        let r = bold_italic_from_axes(false, false, [("wght".to_owned(), 600.0)]);
        assert_eq!(r, (false, false));
    }
}
