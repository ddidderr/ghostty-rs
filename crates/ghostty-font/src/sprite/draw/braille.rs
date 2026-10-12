//! Braille Patterns | U+2800...U+28FF
//! <https://en.wikipedia.org/wiki/Braille_Patterns>
//!
//! (6 dot patterns)
//! ⠀ ⠁ ⠂ ⠃ ⠄ ⠅ ⠆ ⠇ ⠈ ⠉ ⠊ ⠋ ⠌ ⠍ ⠎ ⠏
//! ⠐ ⠑ ⠒ ⠓ ⠔ ⠕ ⠖ ⠗ ⠘ ⠙ ⠚ ⠛ ⠜ ⠝ ⠞ ⠟
//! ⠠ ⠡ ⠢ ⠣ ⠤ ⠥ ⠦ ⠧ ⠨ ⠩ ⠪ ⠫ ⠬ ⠭ ⠮ ⠯
//! ⠰ ⠱ ⠲ ⠳ ⠴ ⠵ ⠶ ⠷ ⠸ ⠹ ⠺ ⠻ ⠼ ⠽ ⠾ ⠿
//!
//! (8 dot patterns)
//! ⡀ ⡁ ⡂ ⡃ ⡄ ⡅ ⡆ ⡇ ⡈ ⡉ ⡊ ⡋ ⡌ ⡍ ⡎ ⡏
//! ⡐ ⡑ ⡒ ⡓ ⡔ ⡕ ⡖ ⡗ ⡘ ⡙ ⡚ ⡛ ⡜ ⡝ ⡞ ⡟
//! ⡠ ⡡ ⡢ ⡣ ⡤ ⡥ ⡦ ⡧ ⡨ ⡩ ⡪ ⡫ ⡬ ⡭ ⡮ ⡯
//! ⡰ ⡱ ⡲ ⡳ ⡴ ⡵ ⡶ ⡷ ⡸ ⡹ ⡺ ⡻ ⡼ ⡽ ⡾ ⡿
//! ⢀ ⢁ ⢂ ⢃ ⢄ ⢅ ⢆ ⢇ ⢈ ⢉ ⢊ ⢋ ⢌ ⢍ ⢎ ⢏
//! ⢐ ⢑ ⢒ ⢓ ⢔ ⢕ ⢖ ⢗ ⢘ ⢙ ⢚ ⢛ ⢜ ⢝ ⢞ ⢟
//! ⢠ ⢡ ⢢ ⢣ ⢤ ⢥ ⢦ ⢧ ⢨ ⢩ ⢪ ⢫ ⢬ ⢭ ⢮ ⢯
//! ⢰ ⢱ ⢲ ⢳ ⢴ ⢵ ⢶ ⢷ ⢸ ⢹ ⢺ ⢻ ⢼ ⢽ ⢾ ⢿
//! ⣀ ⣁ ⣂ ⣃ ⣄ ⣅ ⣆ ⣇ ⣈ ⣉ ⣊ ⣋ ⣌ ⣍ ⣎ ⣏
//! ⣐ ⣑ ⣒ ⣓ ⣔ ⣕ ⣖ ⣗ ⣘ ⣙ ⣚ ⣛ ⣜ ⣝ ⣞ ⣟
//! ⣠ ⣡ ⣢ ⣣ ⣤ ⣥ ⣦ ⣧ ⣨ ⣩ ⣪ ⣫ ⣬ ⣭ ⣮ ⣯
//! ⣰ ⣱ ⣲ ⣳ ⣴ ⣵ ⣶ ⣷ ⣸ ⣹ ⣺ ⣻ ⣼ ⣽ ⣾ ⣿
//!
//! Port of `sprite/draw/braille.zig`.

use super::common::{DrawFn, Range};
use crate::{
    metrics::Metrics,
    sprite::canvas::{Canvas, Color, DrawError},
};

pub const RANGES: &[Range] = &[Range {
    min: 0x2800,
    max: 0x28ff,
    draw: draw2800_28ff as DrawFn,
}];

/// A braille pattern.
///
/// Mnemonic:
/// [t]op    - .       .
/// [u]pper  - .       .
/// [l]ower  - .       .
/// [b]ottom - .       .
///            |       |
///           [l]eft, [r]ight
///
/// Bit layout matches bit patterns of unicode codepoints
/// (bit 0 = `tl`, ... bit 7 = `br`).
#[derive(Debug, Clone, Copy)]
struct Pattern {
    tl: bool,
    ul: bool,
    ll: bool,
    tr: bool,
    ur: bool,
    lr: bool,
    bl: bool,
    br: bool,
}

impl Pattern {
    fn from(cp: u32) -> Pattern {
        let b = cp as u8;
        Pattern {
            tl: b & 1 != 0,
            ul: b & (1 << 1) != 0,
            ll: b & (1 << 2) != 0,
            tr: b & (1 << 3) != 0,
            ur: b & (1 << 4) != 0,
            lr: b & (1 << 5) != 0,
            bl: b & (1 << 6) != 0,
            br: b & (1 << 7) != 0,
        }
    }
}

pub fn draw2800_28ff(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let (wi, hi) = (width as i32, height as i32);
    let mut w: i32 = (width / 4).min(height / 8) as i32;
    let mut x_spacing: i32 = (width / 4) as i32;
    let mut y_spacing: i32 = (height / 8) as i32;
    let mut x_margin: i32 = x_spacing.div_euclid(2);
    let mut y_margin: i32 = y_spacing.div_euclid(2);

    let mut x_px_left: i32 = wi - 2 * x_margin - x_spacing - 2 * w;

    let mut y_px_left: i32 = hi - 2 * y_margin - 3 * y_spacing - 4 * w;

    // First, try hard to ensure the DOT width is non-zero
    if x_px_left >= 2 && y_px_left >= 4 && w == 0 {
        w += 1;
        x_px_left -= 2;
        y_px_left -= 4;
    }

    // Second, prefer a non-zero margin
    if x_px_left >= 2 && x_margin == 0 {
        x_margin = 1;
        x_px_left -= 2;
    }
    if y_px_left >= 2 && y_margin == 0 {
        y_margin = 1;
        y_px_left -= 2;
    }

    // Third, increase spacing
    if x_px_left >= 1 {
        x_spacing += 1;
        x_px_left -= 1;
    }
    if y_px_left >= 3 {
        y_spacing += 1;
        y_px_left -= 3;
    }

    // Fourth, margins (“spacing”, but on the sides)
    if x_px_left >= 2 {
        x_margin += 1;
        x_px_left -= 2;
    }
    if y_px_left >= 2 {
        y_margin += 1;
        y_px_left -= 2;
    }

    // Last - increase dot width
    if x_px_left >= 2 && y_px_left >= 4 {
        w += 1;
        x_px_left -= 2;
        y_px_left -= 4;
    }

    debug_assert!(x_px_left <= 1 || y_px_left <= 1);
    debug_assert!(2 * x_margin + 2 * w + x_spacing <= wi);
    debug_assert!(2 * y_margin + 4 * w + 3 * y_spacing <= hi);

    let x = [x_margin, x_margin + w + x_spacing];
    let y0 = y_margin;
    let y1 = y0 + w + y_spacing;
    let y2 = y1 + w + y_spacing;
    let y3 = y2 + w + y_spacing;
    let y = [y0, y1, y2, y3];

    debug_assert!(cp >= 0x2800);
    debug_assert!(cp <= 0x28ff);
    let p = Pattern::from(cp);

    let mut dot = |on: bool, xi: usize, yi: usize| {
        if on {
            canvas.box_(x[xi], y[yi], x[xi] + w, y[yi] + w, Color::ON);
        }
    };
    dot(p.tl, 0, 0);
    dot(p.ul, 0, 1);
    dot(p.ll, 0, 2);
    dot(p.bl, 0, 3);
    dot(p.tr, 1, 0);
    dot(p.ur, 1, 1);
    dot(p.lr, 1, 2);
    dot(p.br, 1, 3);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_bits_match_codepoint() {
        let p = Pattern::from(0x2801);
        assert!(p.tl && !p.ul && !p.br);
        let p = Pattern::from(0x2880);
        assert!(p.br && !p.tl);
        let p = Pattern::from(0x28ff);
        assert!(p.tl && p.ul && p.ll && p.tr && p.ur && p.lr && p.bl && p.br);
    }

    #[test]
    fn all_braille_render_at_common_sizes() {
        // Exercises the (debug) asserts in the layout algorithm. Like Zig, some
        // odd sizes (e.g. 14x23) trip the `x_px_left <= 1 || y_px_left <= 1`
        // assert in debug builds, so only realistic sizes are tested here.
        let m = Metrics::calc(crate::metrics::FaceMetrics::default());
        for (w, h) in [
            (7, 14),
            (8, 16),
            (9, 17),
            (10, 20),
            (11, 21),
            (12, 24),
            (18, 36),
            (20, 40),
        ] {
            let mut c = Canvas::new(w, h, w / 4, h / 4);
            for cp in 0x2800..=0x28ff {
                draw2800_28ff(cp, &mut c, w, h, &m).unwrap();
            }
        }
    }
}
