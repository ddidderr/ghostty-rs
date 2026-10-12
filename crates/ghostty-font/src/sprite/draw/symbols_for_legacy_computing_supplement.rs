//! Symbols for Legacy Computing Supplement | U+1CC00...U+1CEBF
//! <https://en.wikipedia.org/wiki/Symbols_for_Legacy_Computing_Supplement>
//!
//! Port of `sprite/draw/symbols_for_legacy_computing_supplement.zig`.

use std::sync::OnceLock;

use super::{
    box_::{self, Lines, Style},
    common::{Alignment, Corner, DrawFn, Fraction, Range, fill},
    symbols_for_legacy_computing as sflc,
};
use crate::{
    metrics::Metrics,
    sprite::canvas::{Canvas, CapMode, Color, DrawError, Path, StrokeOptions},
};

pub const RANGES: &[Range] = &[
    Range {
        min: 0x1cc1b,
        max: 0x1cc1e,
        draw: draw1cc1b_1cc1e as DrawFn,
    },
    Range {
        min: 0x1cc21,
        max: 0x1cc2f,
        draw: draw1cc21_1cc2f as DrawFn,
    },
    Range {
        min: 0x1cc30,
        max: 0x1cc3f,
        draw: draw1cc30_1cc3f as DrawFn,
    },
    Range {
        min: 0x1cd00,
        max: 0x1cde5,
        draw: draw1cd00_1cde5 as DrawFn,
    },
    Range {
        min: 0x1ce00,
        max: 0x1ce00,
        draw: draw1ce00 as DrawFn,
    },
    Range {
        min: 0x1ce01,
        max: 0x1ce01,
        draw: draw1ce01 as DrawFn,
    },
    Range {
        min: 0x1ce0b,
        max: 0x1ce0b,
        draw: draw1ce0b as DrawFn,
    },
    Range {
        min: 0x1ce0c,
        max: 0x1ce0c,
        draw: draw1ce0c as DrawFn,
    },
    Range {
        min: 0x1ce16,
        max: 0x1ce19,
        draw: draw1ce16_1ce19 as DrawFn,
    },
    Range {
        min: 0x1ce51,
        max: 0x1ce8f,
        draw: draw1ce51_1ce8f as DrawFn,
    },
    Range {
        min: 0x1ce90,
        max: 0x1ceaf,
        draw: draw1ce90_1ceaf as DrawFn,
    },
];

const OCTANT_MIN: u32 = 0x1cd00;
const OCTANT_MAX: u32 = 0x1cde5;

/// Parse the octant data. Each entry is a bitmask where bit `n-1` is set for
/// octant `n` (1..=8). Lines are in the format "BLOCK OCTANT-1235".
fn octants() -> &'static [u8] {
    static OCTANTS: OnceLock<Vec<u8>> = OnceLock::new();
    OCTANTS.get_or_init(|| {
        let mut result = Vec::new();
        for line in include_str!("octants.txt").split('\n') {
            // Skip comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let idx = line.find('-').expect("octant line has '-'");
            let mut bits = 0u8;
            for c in line[idx + 1..].bytes() {
                bits |= 1 << (c - b'1');
            }
            result.push(bits);
        }
        assert_eq!(result.len(), (OCTANT_MAX - OCTANT_MIN + 1) as usize);
        result
    })
}

/// Octants
pub fn draw1cd00_1cde5(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    use Fraction::{End, Full, Half, OneQuarter, ThreeQuarters, TwoQuarters, Zero};

    let oct = octants()[(cp - OCTANT_MIN) as usize];
    let has = |n: u8| oct & (1 << (n - 1)) != 0;
    if has(1) {
        fill(metrics, canvas, Zero, Half, Zero, OneQuarter);
    }
    if has(2) {
        fill(metrics, canvas, Half, Full, Zero, OneQuarter);
    }
    if has(3) {
        fill(metrics, canvas, Zero, Half, OneQuarter, TwoQuarters);
    }
    if has(4) {
        fill(metrics, canvas, Half, Full, OneQuarter, TwoQuarters);
    }
    if has(5) {
        fill(metrics, canvas, Zero, Half, TwoQuarters, ThreeQuarters);
    }
    if has(6) {
        fill(metrics, canvas, Half, Full, TwoQuarters, ThreeQuarters);
    }
    if has(7) {
        fill(metrics, canvas, Zero, Half, ThreeQuarters, End);
    }
    if has(8) {
        fill(metrics, canvas, Half, Full, ThreeQuarters, End);
    }
    Ok(())
}

/// Separated Block Quadrants
/// 𜰡 𜰢 𜰣 𜰤 𜰥 𜰦 𜰧 𜰨 𜰩 𜰪 𜰫 𜰬 𜰭 𜰮 𜰯
pub fn draw1cc21_1cc2f(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    // Bits laid out to match the codepoint order.
    let bits = (cp - 0x1CC20) & 0xF;
    let (tl, tr, bl, br) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);

    let gap = (width / 12).max(1) as i32;

    let mid_gap_x = gap * 2 + (width % 2) as i32;
    let mid_gap_y = gap * 2 + (height % 2) as i32;

    let w = (width as i32 - gap * 2 - mid_gap_x) / 2;
    let h = (height as i32 - gap * 2 - mid_gap_y) / 2;

    if tl {
        canvas.box_(gap, gap, gap + w, gap + h, Color::ON);
    }
    if tr {
        canvas.box_(
            gap + w + mid_gap_x,
            gap,
            gap + w + mid_gap_x + w,
            gap + h,
            Color::ON,
        );
    }
    if bl {
        canvas.box_(
            gap,
            gap + h + mid_gap_y,
            gap + w,
            gap + h + mid_gap_y + h,
            Color::ON,
        );
    }
    if br {
        canvas.box_(
            gap + w + mid_gap_x,
            gap + h + mid_gap_y,
            gap + w + mid_gap_x + w,
            gap + h + mid_gap_y + h,
            Color::ON,
        );
    }
    Ok(())
}

/// Twelfth and Quarter circle pieces.
/// 𜰰 𜰱 𜰲 𜰳 𜰴 𜰵 𜰶 𜰷 𜰸 𜰹 𜰺 𜰻 𜰼 𜰽 𜰾 𜰿
///
/// 𜰰𜰱𜰲𜰳
/// 𜰴𜰵𜰶𜰷
/// 𜰸𜰹𜰺𜰻
/// 𜰼𜰽𜰾𜰿
///
/// These are actually ellipses, sized to touch
/// the edge of their enclosing set of cells.
pub fn draw1cc30_1cc3f(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        0x1CC30 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            0.0,
            2.0,
            2.0,
            Corner::Tl,
        )?,
        0x1CC31 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            1.0,
            0.0,
            2.0,
            2.0,
            Corner::Tl,
        )?,
        0x1CC32 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            2.0,
            0.0,
            2.0,
            2.0,
            Corner::Tr,
        )?,
        0x1CC33 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            3.0,
            0.0,
            2.0,
            2.0,
            Corner::Tr,
        )?,
        0x1CC34 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            1.0,
            2.0,
            2.0,
            Corner::Tl,
        )?,
        0x1CC35 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            0.0,
            1.0,
            1.0,
            Corner::Tl,
        )?,
        0x1CC36 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            1.0,
            0.0,
            1.0,
            1.0,
            Corner::Tr,
        )?,
        0x1CC37 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            3.0,
            1.0,
            2.0,
            2.0,
            Corner::Tr,
        )?,
        0x1CC38 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            2.0,
            2.0,
            2.0,
            Corner::Bl,
        )?,
        0x1CC39 => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            1.0,
            1.0,
            1.0,
            Corner::Bl,
        )?,
        0x1CC3A => circle_piece(
            canvas,
            width,
            height,
            metrics,
            1.0,
            1.0,
            1.0,
            1.0,
            Corner::Br,
        )?,
        0x1CC3B => circle_piece(
            canvas,
            width,
            height,
            metrics,
            3.0,
            2.0,
            2.0,
            2.0,
            Corner::Br,
        )?,
        0x1CC3C => circle_piece(
            canvas,
            width,
            height,
            metrics,
            0.0,
            3.0,
            2.0,
            2.0,
            Corner::Bl,
        )?,
        0x1CC3D => circle_piece(
            canvas,
            width,
            height,
            metrics,
            1.0,
            3.0,
            2.0,
            2.0,
            Corner::Bl,
        )?,
        0x1CC3E => circle_piece(
            canvas,
            width,
            height,
            metrics,
            2.0,
            3.0,
            2.0,
            2.0,
            Corner::Br,
        )?,
        0x1CC3F => circle_piece(
            canvas,
            width,
            height,
            metrics,
            3.0,
            3.0,
            2.0,
            2.0,
            Corner::Br,
        )?,
        _ => unreachable!(),
    }
    Ok(())
}

/// TODO: These two characters should be easy, but it's not clear how they're
///       meant to align with adjacent cells, what characters they're meant to
///       be used with:
///       - 1CC1F 𜰟 BOX DRAWINGS DOUBLE DIAGONAL UPPER RIGHT TO LOWER LEFT
///       - 1CC20 𜰠 BOX DRAWINGS DOUBLE DIAGONAL UPPER LEFT TO LOWER RIGHT
pub fn draw1cc1b_1cc1e(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let w = width as i32;
    let h = height as i32;
    let t = metrics.box_thickness as i32;
    let horiz = Lines {
        left: Style::Light,
        right: Style::Light,
        ..Lines::NONE
    };
    match cp {
        // 𜰛 BOX DRAWINGS LIGHT HORIZONTAL AND UPPER RIGHT
        0x1CC1B => {
            box_::lines_char(metrics, canvas, horiz);
            canvas.box_(w - t, 0, w, h.div_euclid(2), Color::ON);
        }
        // 𜰜 BOX DRAWINGS LIGHT HORIZONTAL AND LOWER RIGHT
        0x1CC1C => {
            box_::lines_char(metrics, canvas, horiz);
            canvas.box_(w - t, h.div_euclid(2), w, h, Color::ON);
        }
        // 𜰝 BOX DRAWINGS LIGHT TOP AND UPPER LEFT
        0x1CC1D => {
            canvas.box_(0, 0, w, t, Color::ON);
            canvas.box_(0, 0, t, h.div_euclid(2), Color::ON);
        }
        // 𜰞 BOX DRAWINGS LIGHT BOTTOM AND LOWER LEFT
        0x1CC1E => {
            canvas.box_(0, h - t, w, h, Color::ON);
            canvas.box_(0, h.div_euclid(2), t, h, Color::ON);
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// 𜸀 RIGHT HALF AND LEFT HALF WHITE CIRCLE
pub fn draw1ce00(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    sflc::circle(metrics, canvas, Alignment::LEFT, false);
    sflc::circle(metrics, canvas, Alignment::RIGHT, false);
    Ok(())
}

/// 𜸁 LOWER HALF AND UPPER HALF WHITE CIRCLE
pub fn draw1ce01(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    sflc::circle(metrics, canvas, Alignment::TOP, false);
    sflc::circle(metrics, canvas, Alignment::BOTTOM, false);
    Ok(())
}

/// 𜸋 LEFT HALF WHITE ELLIPSE
pub fn draw1ce0b(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    circle_piece(
        canvas,
        width,
        height,
        metrics,
        0.0,
        0.0,
        1.0,
        0.5,
        Corner::Tl,
    )?;
    circle_piece(
        canvas,
        width,
        height,
        metrics,
        0.0,
        0.0,
        1.0,
        0.5,
        Corner::Bl,
    )
}

/// 𜸌 RIGHT HALF WHITE ELLIPSE
pub fn draw1ce0c(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    circle_piece(
        canvas,
        width,
        height,
        metrics,
        1.0,
        0.0,
        1.0,
        0.5,
        Corner::Tr,
    )?;
    circle_piece(
        canvas,
        width,
        height,
        metrics,
        1.0,
        0.0,
        1.0,
        0.5,
        Corner::Br,
    )
}

pub fn draw1ce16_1ce19(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let w = width as i32;
    let h = height as i32;
    let t = metrics.box_thickness as i32;
    let vert = Lines {
        up: Style::Light,
        down: Style::Light,
        ..Lines::NONE
    };
    match cp {
        // 𜸖 BOX DRAWINGS LIGHT VERTICAL AND TOP RIGHT
        0x1CE16 => {
            box_::lines_char(metrics, canvas, vert);
            canvas.box_(w.div_euclid(2), 0, w, t, Color::ON);
        }
        // 𜸗 BOX DRAWINGS LIGHT VERTICAL AND BOTTOM RIGHT
        0x1CE17 => {
            box_::lines_char(metrics, canvas, vert);
            canvas.box_(w.div_euclid(2), h - t, w, h, Color::ON);
        }
        // 𜸘 BOX DRAWINGS LIGHT VERTICAL AND TOP LEFT
        0x1CE18 => {
            box_::lines_char(metrics, canvas, vert);
            canvas.box_(0, 0, w.div_euclid(2), t, Color::ON);
        }
        // 𜸙 BOX DRAWINGS LIGHT VERTICAL AND BOTTOM LEFT
        0x1CE19 => {
            box_::lines_char(metrics, canvas, vert);
            canvas.box_(0, h - t, w.div_euclid(2), h, Color::ON);
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Separated Block Sextants
pub fn draw1ce51_1ce8f(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    // Bits laid out to match the codepoint order: tl, tr, ml, mr, bl, br.
    let bits = (cp - 0x1CE50) & 0x3F;
    let (tl, tr, ml, mr, bl, br) = (
        bits & 1 != 0,
        bits & 2 != 0,
        bits & 4 != 0,
        bits & 8 != 0,
        bits & 16 != 0,
        bits & 32 != 0,
    );

    let gap = (width / 12).max(1) as i32;

    let mid_gap_x = gap * 2 + (width % 2) as i32;
    let y_extra = (height % 3) as i32;
    let mid_gap_y = gap * 2 + y_extra.div_euclid(2);

    let w = (width as i32 - gap * 2 - mid_gap_x) / 2;
    let h = (height as i32 - gap * 2 - mid_gap_y * 2).div_euclid(3);
    // Distribute any leftover height in to the middle row of blocks.
    let h_m = height as i32 - gap * 2 - mid_gap_y * 2 - h * 2;

    if tl {
        canvas.box_(gap, gap, gap + w, gap + h, Color::ON);
    }
    if tr {
        canvas.box_(
            gap + w + mid_gap_x,
            gap,
            gap + w + mid_gap_x + w,
            gap + h,
            Color::ON,
        );
    }
    if ml {
        canvas.box_(
            gap,
            gap + h + mid_gap_y,
            gap + w,
            gap + h + mid_gap_y + h_m,
            Color::ON,
        );
    }
    if mr {
        canvas.box_(
            gap + w + mid_gap_x,
            gap + h + mid_gap_y,
            gap + w + mid_gap_x + w,
            gap + h + mid_gap_y + h_m,
            Color::ON,
        );
    }
    if bl {
        canvas.box_(
            gap,
            gap + h + mid_gap_y + h_m + mid_gap_y,
            gap + w,
            gap + h + mid_gap_y + h_m + mid_gap_y + h,
            Color::ON,
        );
    }
    if br {
        canvas.box_(
            gap + w + mid_gap_x,
            gap + h + mid_gap_y + h_m + mid_gap_y,
            gap + w + mid_gap_x + w,
            gap + h + mid_gap_y + h_m + mid_gap_y + h,
            Color::ON,
        );
    }
    Ok(())
}

/// Sixteenth Blocks
pub fn draw1ce90_1ceaf(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let q = Fraction::QUARTERS;
    match cp {
        0x1CE90 => fill(metrics, canvas, q[0], q[1], q[0], q[1]),
        0x1CE91 => fill(metrics, canvas, q[1], q[2], q[0], q[1]),
        0x1CE92 => fill(metrics, canvas, q[2], q[3], q[0], q[1]),
        0x1CE93 => fill(metrics, canvas, q[3], q[4], q[0], q[1]),
        0x1CE94 => fill(metrics, canvas, q[0], q[1], q[1], q[2]),
        0x1CE95 => fill(metrics, canvas, q[1], q[2], q[1], q[2]),
        0x1CE96 => fill(metrics, canvas, q[2], q[3], q[1], q[2]),
        0x1CE97 => fill(metrics, canvas, q[3], q[4], q[1], q[2]),
        0x1CE98 => fill(metrics, canvas, q[0], q[1], q[2], q[3]),
        0x1CE99 => fill(metrics, canvas, q[1], q[2], q[2], q[3]),
        0x1CE9A => fill(metrics, canvas, q[2], q[3], q[2], q[3]),
        0x1CE9B => fill(metrics, canvas, q[3], q[4], q[2], q[3]),
        0x1CE9C => fill(metrics, canvas, q[0], q[1], q[3], q[4]),
        0x1CE9D => fill(metrics, canvas, q[1], q[2], q[3], q[4]),
        0x1CE9E => fill(metrics, canvas, q[2], q[3], q[3], q[4]),
        0x1CE9F => fill(metrics, canvas, q[3], q[4], q[3], q[4]),
        0x1CEA0 => fill(metrics, canvas, q[2], q[4], q[3], q[4]),
        0x1CEA1 => fill(metrics, canvas, q[1], q[4], q[3], q[4]),
        0x1CEA2 => fill(metrics, canvas, q[0], q[3], q[3], q[4]),
        0x1CEA3 => fill(metrics, canvas, q[0], q[2], q[3], q[4]),
        0x1CEA4 => fill(metrics, canvas, q[0], q[1], q[2], q[4]),
        0x1CEA5 => fill(metrics, canvas, q[0], q[1], q[1], q[4]),
        0x1CEA6 => fill(metrics, canvas, q[0], q[1], q[0], q[3]),
        0x1CEA7 => fill(metrics, canvas, q[0], q[1], q[0], q[2]),
        0x1CEA8 => fill(metrics, canvas, q[0], q[2], q[0], q[1]),
        0x1CEA9 => fill(metrics, canvas, q[0], q[3], q[0], q[1]),
        0x1CEAA => fill(metrics, canvas, q[1], q[4], q[0], q[1]),
        0x1CEAB => fill(metrics, canvas, q[2], q[4], q[0], q[1]),
        0x1CEAC => fill(metrics, canvas, q[3], q[4], q[0], q[2]),
        0x1CEAD => fill(metrics, canvas, q[3], q[4], q[0], q[3]),
        0x1CEAE => fill(metrics, canvas, q[3], q[4], q[1], q[4]),
        0x1CEAF => fill(metrics, canvas, q[3], q[4], q[2], q[4]),
        _ => unreachable!(),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn circle_piece(
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    corner: Corner,
) -> Result<(), DrawError> {
    // Radius in pixels of the arc we need to draw.
    let wdth = f64::from(width) * w;
    let hght = f64::from(height) * h;

    // Position in pixels (rather than cells) for x/y
    let xp = f64::from(width) * x;
    let yp = f64::from(height) * y;

    // Set the clip so we don't include anything outside of the cell.
    canvas.clip_left = canvas.padding_x;
    canvas.clip_right = canvas.padding_x;
    canvas.clip_top = canvas.padding_y;
    canvas.clip_bottom = canvas.padding_y;

    // Coefficient for approximating a circular arc.
    let c = (std::f64::consts::SQRT_2 - 1.0) * 4.0 / 3.0;
    let cw = c * wdth;
    let ch = c * hght;

    let thick = f64::from(metrics.box_thickness);
    let ht = thick * 0.5;

    let mut path = Path::new();

    match corner {
        Corner::Tl => {
            path.move_to(wdth - xp, ht - yp);
            path.curve_to(
                wdth - cw - xp,
                ht - yp,
                ht - xp,
                hght - ch - yp,
                ht - xp,
                hght - yp,
            );
        }
        Corner::Tr => {
            path.move_to(wdth - xp, ht - yp);
            path.curve_to(
                wdth + cw - xp,
                ht - yp,
                wdth * 2.0 - ht - xp,
                hght - ch - yp,
                wdth * 2.0 - ht - xp,
                hght - yp,
            );
        }
        Corner::Bl => {
            path.move_to(ht - xp, hght - yp);
            path.curve_to(
                ht - xp,
                hght + ch - yp,
                wdth - cw - xp,
                hght * 2.0 - ht - yp,
                wdth - xp,
                hght * 2.0 - ht - yp,
            );
        }
        Corner::Br => {
            path.move_to(wdth * 2.0 - ht - xp, hght - yp);
            path.curve_to(
                wdth * 2.0 - ht - xp,
                hght + ch - yp,
                wdth + cw - xp,
                hght * 2.0 - ht - yp,
                wdth - xp,
                hght * 2.0 - ht - yp,
            );
        }
    }

    canvas.stroke_path(
        &path,
        StrokeOptions {
            line_cap_mode: CapMode::Butt,
            line_width: f64::from(metrics.box_thickness),
        },
        Color::ON,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octant_table_parses() {
        assert_eq!(octants().len(), 230);
        // BLOCK OCTANT-3
        assert_eq!(0b100, octants()[0]);
        // BLOCK OCTANT-23
        assert_eq!(0b110, octants()[1]);
    }
}
