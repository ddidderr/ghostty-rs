//! Symbols for Legacy Computing | U+1FB00...U+1FBFF
//! <https://en.wikipedia.org/wiki/Symbols_for_Legacy_Computing>
//!
//! Port of `sprite/draw/symbols_for_legacy_computing.zig`.

use super::{
    block,
    box_,
    common::{
        Alignment,
        Corner,
        DrawFn,
        Edge,
        FIVE_EIGHTHS,
        Fraction,
        HALF,
        HAlign,
        ONE_EIGHTH,
        ONE_QUARTER,
        ONE_THIRD,
        Quads,
        Range,
        SEVEN_EIGHTHS,
        Shade,
        THREE_EIGHTHS,
        THREE_QUARTERS,
        TWO_THIRDS,
        Thickness,
        VAlign,
        fill,
    },
    geometric_shapes as geo,
};
use crate::{
    metrics::Metrics,
    sprite::canvas::{Canvas, Color, DrawError, Line, Path, Point, Rect, StrokeOptions},
};

const _: (f64, f64) = (ONE_THIRD, TWO_THIRDS);

pub const RANGES: &[Range] = &[
    Range {
        min: 0x1FB00,
        max: 0x1FB3B,
        draw: draw1fb00_1fb3b as DrawFn,
    },
    Range {
        min: 0x1FB3C,
        max: 0x1FB67,
        draw: draw1fb3c_1fb67 as DrawFn,
    },
    Range {
        min: 0x1FB68,
        max: 0x1FB6F,
        draw: draw1fb68_1fb6f as DrawFn,
    },
    Range {
        min: 0x1FB70,
        max: 0x1FB75,
        draw: draw1fb70_1fb75 as DrawFn,
    },
    Range {
        min: 0x1FB76,
        max: 0x1FB7B,
        draw: draw1fb76_1fb7b as DrawFn,
    },
    Range {
        min: 0x1FB7C,
        max: 0x1FB97,
        draw: draw1fb7c_1fb97 as DrawFn,
    },
    Range {
        min: 0x1FB98,
        max: 0x1FB98,
        draw: draw1fb98 as DrawFn,
    },
    Range {
        min: 0x1FB99,
        max: 0x1FB99,
        draw: draw1fb99 as DrawFn,
    },
    Range {
        min: 0x1FB9A,
        max: 0x1FB9F,
        draw: draw1fb9a_1fb9f as DrawFn,
    },
    Range {
        min: 0x1FBA0,
        max: 0x1FBAE,
        draw: draw1fba0_1fbae as DrawFn,
    },
    Range {
        min: 0x1FBAF,
        max: 0x1FBAF,
        draw: draw1fbaf as DrawFn,
    },
    Range {
        min: 0x1FBBD,
        max: 0x1FBBD,
        draw: draw1fbbd as DrawFn,
    },
    Range {
        min: 0x1FBBE,
        max: 0x1FBBE,
        draw: draw1fbbe as DrawFn,
    },
    Range {
        min: 0x1FBBF,
        max: 0x1FBBF,
        draw: draw1fbbf as DrawFn,
    },
    Range {
        min: 0x1FBCE,
        max: 0x1FBCE,
        draw: draw1fbce as DrawFn,
    },
    Range {
        min: 0x1FBCF,
        max: 0x1FBCF,
        draw: draw1fbcf as DrawFn,
    },
    Range {
        min: 0x1FBD0,
        max: 0x1FBDF,
        draw: draw1fbd0_1fbdf as DrawFn,
    },
    Range {
        min: 0x1FBE0,
        max: 0x1FBEF,
        draw: draw1fbe0_1fbef as DrawFn,
    },
];

/// Set the clip so we don't include anything outside of the cell.
fn clip_to_cell(canvas: &mut Canvas) {
    canvas.clip_left = canvas.padding_x;
    canvas.clip_right = canvas.padding_x;
    canvas.clip_top = canvas.padding_y;
    canvas.clip_bottom = canvas.padding_y;
}

#[derive(Debug, Clone, Copy, Default)]
struct SmoothMosaic {
    tl: bool,
    ul: bool,
    ll: bool,
    bl: bool,
    bc: bool,
    br: bool,
    lr: bool,
    ur: bool,
    tr: bool,
    tc: bool,
}

impl SmoothMosaic {
    /// `rows` are the four 3-character rows of the pattern; this mirrors
    /// indexing the Zig 15-byte `"...\n...\n...\n..."` pattern.
    fn from(rows: (&str, &str, &str, &str)) -> SmoothMosaic {
        let mut p = [b' '; 15];
        for (i, r) in [rows.0, rows.1, rows.2, rows.3].iter().enumerate() {
            p[i * 4..i * 4 + 3].copy_from_slice(r.as_bytes());
            if i < 3 {
                p[i * 4 + 3] = b'\n';
            }
        }
        let h = |i: usize| p[i] == b'#';
        SmoothMosaic {
            tl: h(0),
            ul: h(4) && (!h(0) || !h(8)),
            ll: h(8) && (!h(4) || !h(12)),
            bl: h(12),
            bc: h(13) && (!h(12) || !h(14)),
            br: h(14),
            lr: h(10) && (!h(14) || !h(6)),
            ur: h(6) && (!h(10) || !h(2)),
            tr: h(2),
            tc: h(1) && (!h(2) || !h(0)),
        }
    }
}

/// Sextants
pub fn draw1fb00_1fb3b(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    use Fraction::{End, Full, Half, OneThird, TwoThirds, Zero};
    assert!((0x1fb00..=0x1fb3b).contains(&cp));
    let idx = cp - 0x1fb00;
    // Packed struct(u6): tl, tr, ml, mr, bl, br from the low bit up.
    let sex = idx + (idx / 0x14) + 1;
    let bit = |n: u32| sex & (1 << n) != 0;
    if bit(0) {
        fill(metrics, canvas, Zero, Half, Zero, OneThird);
    }
    if bit(1) {
        fill(metrics, canvas, Half, Full, Zero, OneThird);
    }
    if bit(2) {
        fill(metrics, canvas, Zero, Half, OneThird, TwoThirds);
    }
    if bit(3) {
        fill(metrics, canvas, Half, Full, OneThird, TwoThirds);
    }
    if bit(4) {
        fill(metrics, canvas, Zero, Half, TwoThirds, End);
    }
    if bit(5) {
        fill(metrics, canvas, Half, Full, TwoThirds, End);
    }
    Ok(())
}

/// Smooth Mosaics
pub fn draw1fb3c_1fb67(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // Hand written lookup table for these shapes since I couldn't
    // determine any sort of mathematical pattern in the codepoints.
    let pattern = match cp {
        0x1fb3c => ("...", "...", "#..", "##."),
        0x1fb3d => ("...", "...", "#\\.", "###"),
        0x1fb3e => ("...", "#..", "#\\.", "##."),
        0x1fb3f => ("...", "#..", "##.", "###"),
        0x1fb40 => ("#..", "#..", "##.", "##."),
        0x1fb41 => ("/##", "###", "###", "###"),
        0x1fb42 => ("./#", "###", "###", "###"),
        0x1fb43 => (".##", ".##", "###", "###"),
        0x1fb44 => ("..#", ".##", "###", "###"),
        0x1fb45 => (".##", ".##", ".##", "###"),
        0x1fb46 => ("...", "./#", "###", "###"),
        0x1fb47 => ("...", "...", "..#", ".##"),
        0x1fb48 => ("...", "...", "./#", "###"),
        0x1fb49 => ("...", "..#", "./#", ".##"),
        0x1fb4a => ("...", "..#", ".##", "###"),
        0x1fb4b => ("..#", "..#", ".##", ".##"),
        0x1fb4c => ("##\\", "###", "###", "###"),
        0x1fb4d => ("#\\.", "###", "###", "###"),
        0x1fb4e => ("##.", "##.", "###", "###"),
        0x1fb4f => ("#..", "##.", "###", "###"),
        0x1fb50 => ("##.", "##.", "##.", "###"),
        0x1fb51 => ("...", "#\\.", "###", "###"),
        0x1fb52 => ("###", "###", "###", "\\##"),
        0x1fb53 => ("###", "###", "###", ".\\#"),
        0x1fb54 => ("###", "###", ".##", ".##"),
        0x1fb55 => ("###", "###", ".##", "..#"),
        0x1fb56 => ("###", ".##", ".##", ".##"),
        0x1fb57 => ("##.", "#..", "...", "..."),
        0x1fb58 => ("###", "#/.", "...", "..."),
        0x1fb59 => ("##.", "#/.", "#..", "..."),
        0x1fb5a => ("###", "##.", "#..", "..."),
        0x1fb5b => ("##.", "##.", "#..", "#.."),
        0x1fb5c => ("###", "###", "#/.", "..."),
        0x1fb5d => ("###", "###", "###", "##/"),
        0x1fb5e => ("###", "###", "###", "#/."),
        0x1fb5f => ("###", "###", "##.", "##."),
        0x1fb60 => ("###", "###", "##.", "#.."),
        0x1fb61 => ("###", "##.", "##.", "##."),
        0x1fb62 => (".##", "..#", "...", "..."),
        0x1fb63 => ("###", ".\\#", "...", "..."),
        0x1fb64 => (".##", ".\\#", "..#", "..."),
        0x1fb65 => ("###", ".##", "..#", "..."),
        0x1fb66 => (".##", ".##", "..#", "..#"),
        0x1fb67 => ("###", "###", ".\\#", "..."),
        _ => unreachable!(),
    };
    let mosaic = SmoothMosaic::from(pattern);

    let top = 0.0;
    let upper = Fraction::OneThird.float(metrics.cell_height);
    let lower = Fraction::TwoThirds.float(metrics.cell_height);
    let bottom = f64::from(metrics.cell_height);
    let left = 0.0;
    let center = Fraction::Half.float(metrics.cell_width);
    let right = f64::from(metrics.cell_width);

    let mut path = Path::new();
    if mosaic.tl {
        path.line_to(left, top);
    }
    if mosaic.ul {
        path.line_to(left, upper);
    }
    if mosaic.ll {
        path.line_to(left, lower);
    }
    if mosaic.bl {
        path.line_to(left, bottom);
    }
    if mosaic.bc {
        path.line_to(center, bottom);
    }
    if mosaic.br {
        path.line_to(right, bottom);
    }
    if mosaic.lr {
        path.line_to(right, lower);
    }
    if mosaic.ur {
        path.line_to(right, upper);
    }
    if mosaic.tr {
        path.line_to(right, top);
    }
    if mosaic.tc {
        path.line_to(center, top);
    }
    path.close();

    canvas.fill_path(&path, Color::ON)
}

pub fn draw1fb68_1fb6f(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        // '🭨'
        0x1fb68 => {
            edge_triangle(metrics, canvas, Edge::Left)?;
            canvas.invert();
            clip_to_cell(canvas);
        }
        // '🭩'
        0x1fb69 => {
            edge_triangle(metrics, canvas, Edge::Top)?;
            canvas.invert();
            clip_to_cell(canvas);
        }
        // '🭪'
        0x1fb6a => {
            edge_triangle(metrics, canvas, Edge::Right)?;
            canvas.invert();
            clip_to_cell(canvas);
        }
        // '🭫'
        0x1fb6b => {
            edge_triangle(metrics, canvas, Edge::Bottom)?;
            canvas.invert();
            clip_to_cell(canvas);
        }
        // '🭬'
        0x1fb6c => edge_triangle(metrics, canvas, Edge::Left)?,
        // '🭭'
        0x1fb6d => edge_triangle(metrics, canvas, Edge::Top)?,
        // '🭮'
        0x1fb6e => edge_triangle(metrics, canvas, Edge::Right)?,
        // '🭯'
        0x1fb6f => edge_triangle(metrics, canvas, Edge::Bottom)?,

        _ => unreachable!(),
    }
    Ok(())
}

/// Vertical one eighth blocks
pub fn draw1fb70_1fb75(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let n = (cp + 1 - 0x1fb70) as usize;

    fill(
        metrics,
        canvas,
        Fraction::EIGHTHS[n],
        Fraction::EIGHTHS[n + 1],
        Fraction::Top,
        Fraction::Bottom,
    );
    Ok(())
}

/// Horizontal one eighth blocks
pub fn draw1fb76_1fb7b(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let n = (cp + 1 - 0x1fb76) as usize;

    fill(
        metrics,
        canvas,
        Fraction::Left,
        Fraction::Right,
        Fraction::EIGHTHS[n],
        Fraction::EIGHTHS[n + 1],
    );
    Ok(())
}

pub fn draw1fb7c_1fb97(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let blk = block::block;
    match cp {
        // '🭼' LEFT AND LOWER ONE EIGHTH BLOCK
        0x1fb7c => {
            blk(metrics, canvas, Alignment::LEFT, ONE_EIGHTH, 1.0);
            blk(metrics, canvas, Alignment::LOWER, 1.0, ONE_EIGHTH);
        }
        // '🭽' LEFT AND UPPER ONE EIGHTH BLOCK
        0x1fb7d => {
            blk(metrics, canvas, Alignment::LEFT, ONE_EIGHTH, 1.0);
            blk(metrics, canvas, Alignment::UPPER, 1.0, ONE_EIGHTH);
        }
        // '🭾' RIGHT AND UPPER ONE EIGHTH BLOCK
        0x1fb7e => {
            blk(metrics, canvas, Alignment::RIGHT, ONE_EIGHTH, 1.0);
            blk(metrics, canvas, Alignment::UPPER, 1.0, ONE_EIGHTH);
        }
        // '🭿' RIGHT AND LOWER ONE EIGHTH BLOCK
        0x1fb7f => {
            blk(metrics, canvas, Alignment::RIGHT, ONE_EIGHTH, 1.0);
            blk(metrics, canvas, Alignment::LOWER, 1.0, ONE_EIGHTH);
        }
        // '🮀' UPPER AND LOWER ONE EIGHTH BLOCK
        0x1fb80 => {
            blk(metrics, canvas, Alignment::UPPER, 1.0, ONE_EIGHTH);
            blk(metrics, canvas, Alignment::LOWER, 1.0, ONE_EIGHTH);
        }
        // '🮁' Horizontal One Eighth Block 1358
        0x1fb81 => {
            // We just call the draw function for each of the relevant codepoints.
            // The first codepoint is actually a lie, it's before the range, but
            // we need it to get the first (0th) block position. This might be a
            // bit brittle, oh well, if it breaks we can fix it.
            draw1fb76_1fb7b(0x1fb74 + 1, canvas, width, height, metrics)?;
            draw1fb76_1fb7b(0x1fb74 + 3, canvas, width, height, metrics)?;
            draw1fb76_1fb7b(0x1fb74 + 5, canvas, width, height, metrics)?;
            draw1fb76_1fb7b(0x1fb74 + 8, canvas, width, height, metrics)?;
        }

        // '🮂' UPPER ONE QUARTER BLOCK
        0x1fb82 => blk(metrics, canvas, Alignment::UPPER, 1.0, ONE_QUARTER),
        // '🮃' UPPER THREE EIGHTHS BLOCK
        0x1fb83 => blk(metrics, canvas, Alignment::UPPER, 1.0, THREE_EIGHTHS),
        // '🮄' UPPER FIVE EIGHTHS BLOCK
        0x1fb84 => blk(metrics, canvas, Alignment::UPPER, 1.0, FIVE_EIGHTHS),
        // '🮅' UPPER THREE QUARTERS BLOCK
        0x1fb85 => blk(metrics, canvas, Alignment::UPPER, 1.0, THREE_QUARTERS),
        // '🮆' UPPER SEVEN EIGHTHS BLOCK
        0x1fb86 => blk(metrics, canvas, Alignment::UPPER, 1.0, SEVEN_EIGHTHS),

        // '🮇' RIGHT ONE QUARTER BLOCK
        0x1fb87 => blk(metrics, canvas, Alignment::RIGHT, ONE_QUARTER, 1.0),
        // '🮈' RIGHT THREE EIGHTHS BLOCK
        0x1fb88 => blk(metrics, canvas, Alignment::RIGHT, THREE_EIGHTHS, 1.0),
        // '🮉' RIGHT FIVE EIGHTHS BLOCK
        0x1fb89 => blk(metrics, canvas, Alignment::RIGHT, FIVE_EIGHTHS, 1.0),
        // '🮊' RIGHT THREE QUARTERS BLOCK
        0x1fb8a => blk(metrics, canvas, Alignment::RIGHT, THREE_QUARTERS, 1.0),
        // '🮋' RIGHT SEVEN EIGHTHS BLOCK/
        0x1fb8b => blk(metrics, canvas, Alignment::RIGHT, SEVEN_EIGHTHS, 1.0),

        // '🮌'
        0x1fb8c => block::block_shade(metrics, canvas, Alignment::LEFT, HALF, 1.0, Shade::MEDIUM),
        // '🮍'
        0x1fb8d => block::block_shade(metrics, canvas, Alignment::RIGHT, HALF, 1.0, Shade::MEDIUM),
        // '🮎'
        0x1fb8e => block::block_shade(metrics, canvas, Alignment::UPPER, 1.0, HALF, Shade::MEDIUM),
        // '🮏'
        0x1fb8f => block::block_shade(metrics, canvas, Alignment::LOWER, 1.0, HALF, Shade::MEDIUM),

        // '🮐'
        0x1fb90 => block::full_block_shade(metrics, canvas, Shade::MEDIUM),
        // '🮑'
        0x1fb91 => {
            block::full_block_shade(metrics, canvas, Shade::MEDIUM);
            blk(metrics, canvas, Alignment::UPPER, 1.0, HALF);
        }
        // '🮒'
        0x1fb92 => {
            block::full_block_shade(metrics, canvas, Shade::MEDIUM);
            blk(metrics, canvas, Alignment::LOWER, 1.0, HALF);
        }
        0x1fb93 => {
            // NOTE: This codepoint is currently un-allocated, it's a hole
            //       in the unicode block, so it's safe to just render it
            //       as an empty glyph, probably.
        }
        // '🮔'
        0x1fb94 => {
            block::full_block_shade(metrics, canvas, Shade::MEDIUM);
            blk(metrics, canvas, Alignment::RIGHT, HALF, 1.0);
        }
        // '🮕'
        0x1fb95 => checkerboard_fill(metrics, canvas, 0),
        // '🮖'
        0x1fb96 => checkerboard_fill(metrics, canvas, 1),
        // '🮗'
        0x1fb97 => {
            canvas.box_(
                0,
                (height / 4) as i32,
                width as i32,
                (2 * height / 4) as i32,
                Color::ON,
            );
            canvas.box_(
                0,
                (3 * height / 4) as i32,
                width as i32,
                height as i32,
                Color::ON,
            );
        }

        _ => unreachable!(),
    }
    Ok(())
}

/// Draws the diagonal hatch lines shared by U+1FB98 and U+1FB99.
/// `upper_left_to_lower_right` selects the direction.
fn diagonal_fill(metrics: &Metrics, canvas: &mut Canvas, upper_left_to_lower_right: bool) {
    // Set the clip so we don't include anything outside of the cell.
    clip_to_cell(canvas);

    // TODO: This doesn't align properly for most cell sizes, fix that.

    let thick_px = Thickness::Light.height(metrics.box_thickness);
    let line_count = metrics.cell_width / (2 * thick_px);

    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);
    let float_thick = f64::from(thick_px);
    let stride = (float_width / f64::from(line_count)).round();

    for i in 0..=(line_count * 2) {
        let i = i as i32 - line_count as i32;
        let (top_x, bottom_x) = if upper_left_to_lower_right {
            let top_x = f64::from(i) * stride;
            (top_x, float_width + top_x)
        } else {
            let bottom_x = f64::from(i) * stride;
            (float_width + bottom_x, bottom_x)
        };
        let _ = canvas.line(
            Line {
                p0: Point { x: top_x, y: 0.0 },
                p1: Point {
                    x: bottom_x,
                    y: float_height,
                },
            },
            float_thick,
            Color::ON,
        );
    }
}

/// Upper Left to Lower Right Fill
/// 🮘
pub fn draw1fb98(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    diagonal_fill(metrics, canvas, true);
    Ok(())
}

/// Upper Right to Lower Left Fill
/// 🮙
pub fn draw1fb99(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    diagonal_fill(metrics, canvas, false);
    Ok(())
}

pub fn draw1fb9a_1fb9f(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        // '🮚'
        0x1fb9a => {
            edge_triangle(metrics, canvas, Edge::Top)?;
            edge_triangle(metrics, canvas, Edge::Bottom)?;
        }
        // '🮛'
        0x1fb9b => {
            edge_triangle(metrics, canvas, Edge::Left)?;
            edge_triangle(metrics, canvas, Edge::Right)?;
        }
        // '🮜'
        0x1fb9c => geo::corner_triangle_shade(metrics, canvas, Corner::Tl, Shade::MEDIUM)?,
        // '🮝'
        0x1fb9d => geo::corner_triangle_shade(metrics, canvas, Corner::Tr, Shade::MEDIUM)?,
        // '🮞'
        0x1fb9e => geo::corner_triangle_shade(metrics, canvas, Corner::Br, Shade::MEDIUM)?,
        // '🮟'
        0x1fb9f => geo::corner_triangle_shade(metrics, canvas, Corner::Bl, Shade::MEDIUM)?,

        _ => unreachable!(),
    }
    Ok(())
}

pub fn draw1fba0_1fbae(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let q = |tl, tr, bl, br| Quads { tl, tr, bl, br };
    let c = match cp {
        // '🮠'
        0x1fba0 => q(true, false, false, false),
        // '🮡'
        0x1fba1 => q(false, true, false, false),
        // '🮢'
        0x1fba2 => q(false, false, true, false),
        // '🮣'
        0x1fba3 => q(false, false, false, true),
        // '🮤'
        0x1fba4 => q(true, false, true, false),
        // '🮥'
        0x1fba5 => q(false, true, false, true),
        // '🮦'
        0x1fba6 => q(false, false, true, true),
        // '🮧'
        0x1fba7 => q(true, true, false, false),
        // '🮨'
        0x1fba8 => q(true, false, false, true),
        // '🮩'
        0x1fba9 => q(false, true, true, false),
        // '🮪'
        0x1fbaa => q(false, true, true, true),
        // '🮫'
        0x1fbab => q(true, false, true, true),
        // '🮬'
        0x1fbac => q(true, true, false, true),
        // '🮭'
        0x1fbad => q(true, true, true, false),
        // '🮮'
        0x1fbae => q(true, true, true, true),

        _ => unreachable!(),
    };
    corner_diagonal_lines(metrics, canvas, c);
    Ok(())
}

/// 🮯
pub fn draw1fbaf(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::lines_char(
        metrics,
        canvas,
        box_::Lines {
            up: box_::Style::Heavy,
            down: box_::Style::Heavy,
            left: box_::Style::Light,
            right: box_::Style::Light,
        },
    );
    Ok(())
}

/// 🮽
pub fn draw1fbbd(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::light_diagonal_cross(metrics, canvas);
    canvas.invert();
    clip_to_cell(canvas);
    Ok(())
}

/// 🮾
pub fn draw1fbbe(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    corner_diagonal_lines(
        metrics,
        canvas,
        Quads {
            br: true,
            ..Quads::default()
        },
    );
    canvas.invert();
    clip_to_cell(canvas);
    Ok(())
}

/// 🮿
pub fn draw1fbbf(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    corner_diagonal_lines(
        metrics,
        canvas,
        Quads {
            tl: true,
            tr: true,
            bl: true,
            br: true,
        },
    );
    canvas.invert();
    clip_to_cell(canvas);
    Ok(())
}

/// 🯎
pub fn draw1fbce(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    block::block(metrics, canvas, Alignment::LEFT, TWO_THIRDS, 1.0);
    Ok(())
}

/// 🯏
pub fn draw1fbcf(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    block::block(metrics, canvas, Alignment::LEFT, ONE_THIRD, 1.0);
    Ok(())
}

/// Cell diagonals.
pub fn draw1fbd0_1fbdf(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    use Alignment as A;
    let mut d = |from: Alignment, to: Alignment| cell_diagonal(metrics, canvas, from, to);
    match cp {
        // '🯐'
        0x1fbd0 => d(A::MIDDLE_RIGHT, A::LOWER_LEFT),
        // '🯑'
        0x1fbd1 => d(A::UPPER_RIGHT, A::MIDDLE_LEFT),
        // '🯒'
        0x1fbd2 => d(A::UPPER_LEFT, A::MIDDLE_RIGHT),
        // '🯓'
        0x1fbd3 => d(A::MIDDLE_LEFT, A::LOWER_RIGHT),
        // '🯔'
        0x1fbd4 => d(A::UPPER_LEFT, A::LOWER_CENTER),
        // '🯕'
        0x1fbd5 => d(A::UPPER_CENTER, A::LOWER_RIGHT),
        // '🯖'
        0x1fbd6 => d(A::UPPER_RIGHT, A::LOWER_CENTER),
        // '🯗'
        0x1fbd7 => d(A::UPPER_CENTER, A::LOWER_LEFT),
        // '🯘'
        0x1fbd8 => {
            d(A::UPPER_LEFT, A::MIDDLE_CENTER);
            d(A::MIDDLE_CENTER, A::UPPER_RIGHT);
        }
        // '🯙'
        0x1fbd9 => {
            d(A::UPPER_RIGHT, A::MIDDLE_CENTER);
            d(A::MIDDLE_CENTER, A::LOWER_RIGHT);
        }
        // '🯚'
        0x1fbda => {
            d(A::LOWER_LEFT, A::MIDDLE_CENTER);
            d(A::MIDDLE_CENTER, A::LOWER_RIGHT);
        }
        // '🯛'
        0x1fbdb => {
            d(A::UPPER_LEFT, A::MIDDLE_CENTER);
            d(A::MIDDLE_CENTER, A::LOWER_LEFT);
        }
        // '🯜'
        0x1fbdc => {
            d(A::UPPER_LEFT, A::LOWER_CENTER);
            d(A::LOWER_CENTER, A::UPPER_RIGHT);
        }
        // '🯝'
        0x1fbdd => {
            d(A::UPPER_RIGHT, A::MIDDLE_LEFT);
            d(A::MIDDLE_LEFT, A::LOWER_RIGHT);
        }
        // '🯞'
        0x1fbde => {
            d(A::LOWER_LEFT, A::UPPER_CENTER);
            d(A::UPPER_CENTER, A::LOWER_RIGHT);
        }
        // '🯟'
        0x1fbdf => {
            d(A::UPPER_LEFT, A::MIDDLE_RIGHT);
            d(A::MIDDLE_RIGHT, A::LOWER_LEFT);
        }

        _ => unreachable!(),
    }
    Ok(())
}

pub fn draw1fbe0_1fbef(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    use Alignment as A;
    match cp {
        // '🯠'
        0x1fbe0 => circle(metrics, canvas, A::TOP, false),
        // '🯡'
        0x1fbe1 => circle(metrics, canvas, A::RIGHT, false),
        // '🯢'
        0x1fbe2 => circle(metrics, canvas, A::BOTTOM, false),
        // '🯣'
        0x1fbe3 => circle(metrics, canvas, A::LEFT, false),
        // '🯤'
        0x1fbe4 => block::block(metrics, canvas, A::UPPER_CENTER, 0.5, 0.5),
        // '🯥'
        0x1fbe5 => block::block(metrics, canvas, A::LOWER_CENTER, 0.5, 0.5),
        // '🯦'
        0x1fbe6 => block::block(metrics, canvas, A::MIDDLE_LEFT, 0.5, 0.5),
        // '🯧'
        0x1fbe7 => block::block(metrics, canvas, A::MIDDLE_RIGHT, 0.5, 0.5),
        // '🯨'
        0x1fbe8 => circle(metrics, canvas, A::TOP, true),
        // '🯩'
        0x1fbe9 => circle(metrics, canvas, A::RIGHT, true),
        // '🯪'
        0x1fbea => circle(metrics, canvas, A::BOTTOM, true),
        // '🯫'
        0x1fbeb => circle(metrics, canvas, A::LEFT, true),
        // '🯬'
        0x1fbec => circle(metrics, canvas, A::TOP_RIGHT, true),
        // '🯭'
        0x1fbed => circle(metrics, canvas, A::BOTTOM_LEFT, true),
        // '🯮'
        0x1fbee => circle(metrics, canvas, A::BOTTOM_RIGHT, true),
        // '🯯'
        0x1fbef => circle(metrics, canvas, A::TOP_LEFT, true),

        _ => unreachable!(),
    }
    Ok(())
}

fn edge_triangle(metrics: &Metrics, canvas: &mut Canvas, edge: Edge) -> Result<(), DrawError> {
    let upper = 0.0;
    let middle = (f64::from(metrics.cell_height) / 2.0).round();
    let lower = f64::from(metrics.cell_height);
    let left = 0.0;
    let center = (f64::from(metrics.cell_width) / 2.0).round();
    let right = f64::from(metrics.cell_width);

    let (x0, y0, x1, y1) = match edge {
        Edge::Top => (right, upper, left, upper),
        Edge::Left => (left, upper, left, lower),
        Edge::Bottom => (left, lower, right, lower),
        Edge::Right => (right, lower, right, upper),
    };

    let mut path = Path::new();
    path.move_to(center, middle);
    path.line_to(x0, y0);
    path.line_to(x1, y1);
    path.close();

    canvas.fill_path(&path, Color::ON)
}

fn corner_diagonal_lines(metrics: &Metrics, canvas: &mut Canvas, corners: Quads) {
    let thick_px = Thickness::Light.height(metrics.box_thickness);

    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);
    let float_thick = f64::from(thick_px);
    let center_x = f64::from(metrics.cell_width / 2 + metrics.cell_width % 2);
    let center_y = f64::from(metrics.cell_height / 2 + metrics.cell_height % 2);

    let mut line = |p0: (f64, f64), p1: (f64, f64)| {
        let _ = canvas.line(
            Line {
                p0: Point { x: p0.0, y: p0.1 },
                p1: Point { x: p1.0, y: p1.1 },
            },
            float_thick,
            Color::ON,
        );
    };

    if corners.tl {
        line((center_x, 0.0), (0.0, center_y));
    }
    if corners.tr {
        line((center_x, 0.0), (float_width, center_y));
    }
    if corners.bl {
        line((center_x, float_height), (0.0, center_y));
    }
    if corners.br {
        line((center_x, float_height), (float_width, center_y));
    }
}

fn cell_diagonal(metrics: &Metrics, canvas: &mut Canvas, from: Alignment, to: Alignment) {
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);

    let hx = |a: HAlign| match a {
        HAlign::Left => 0.0,
        HAlign::Right => float_width,
        HAlign::Center => float_width / 2.0,
    };
    let vy = |a: VAlign| match a {
        VAlign::Top => 0.0,
        VAlign::Bottom => float_height,
        VAlign::Middle => float_height / 2.0,
    };

    let _ = canvas.line(
        Line {
            p0: Point {
                x: hx(from.horizontal),
                y: vy(from.vertical),
            },
            p1: Point {
                x: hx(to.horizontal),
                y: vy(to.vertical),
            },
        },
        f64::from(Thickness::Light.height(metrics.box_thickness)),
        Color::ON,
    );
}

fn checkerboard_fill(metrics: &Metrics, canvas: &mut Canvas, parity: usize) {
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);
    let cell_width = metrics.cell_width as usize;
    let cell_height = metrics.cell_height as usize;
    let x_size: usize = 4;
    let y_size: usize = (4.0 * (float_height / float_width)).round() as usize;
    for x in 0..x_size {
        let x0 = (cell_width * x) / x_size;
        let x1 = (cell_width * (x + 1)) / x_size;
        for y in 0..y_size {
            let y0 = (cell_height * y) / y_size;
            let y1 = (cell_height * (y + 1)) / y_size;
            if (x + y) % 2 == parity {
                canvas.rect(
                    Rect {
                        x: x0 as i32,
                        y: y0 as i32,
                        width: x1.saturating_sub(x0) as i32,
                        height: y1.saturating_sub(y0) as i32,
                    },
                    Color::ON,
                );
            }
        }
    }
}

pub fn circle(metrics: &Metrics, canvas: &mut Canvas, position: Alignment, filled: bool) {
    // Set the clip so we don't include anything outside of the cell.
    clip_to_cell(canvas);

    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);

    let x = match position.horizontal {
        HAlign::Left => 0.0,
        HAlign::Right => float_width,
        HAlign::Center => float_width / 2.0,
    };
    let y = match position.vertical {
        VAlign::Top => 0.0,
        VAlign::Bottom => float_height,
        VAlign::Middle => float_height / 2.0,
    };
    let r = 0.5 * float_width.min(float_height);

    let line_width = f64::from(Thickness::Light.height(metrics.box_thickness));

    let mut path = Path::new();
    if filled {
        path.arc(x, y, r, 0.0, std::f64::consts::PI * 2.0);
        path.close();
        let _ = canvas.fill_path(&path, Color::ON);
    } else {
        path.arc(x, y, r - line_width / 2.0, 0.0, std::f64::consts::PI * 2.0);
        path.close();
        let _ = canvas.stroke_path(
            &path,
            StrokeOptions {
                line_width,
                ..StrokeOptions::default()
            },
            Color::ON,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        let mut m = Metrics::calc(crate::metrics::FaceMetrics {
            px_per_em: 16.0,
            cell_width: 12.0,
            ascent: 20.0,
            descent: -4.0,
            line_gap: 0.0,
            underline_thickness: Some(2.0),
            strikethrough_thickness: Some(2.0),
            ..Default::default()
        });
        m.box_thickness = 2;
        m
    }

    #[test]
    fn mosaic_table_complete() {
        let m = metrics();
        for cp in 0x1fb3c..=0x1fb67 {
            let mut c = Canvas::new(m.cell_width, m.cell_height, 3, 6);
            draw1fb3c_1fb67(cp, &mut c, m.cell_width, m.cell_height, &m).expect("draw");
            assert!(c.sfc.iter().any(|v| *v != 0), "cp {cp:x} drew nothing");
        }
    }

    #[test]
    fn sextants_draw() {
        let m = metrics();
        for cp in 0x1fb00..=0x1fb3b {
            let mut c = Canvas::new(m.cell_width, m.cell_height, 3, 6);
            draw1fb00_1fb3b(cp, &mut c, m.cell_width, m.cell_height, &m).expect("draw");
            assert!(c.sfc.iter().any(|v| *v != 0));
        }
    }
}
