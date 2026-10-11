//! Geometric Shapes | U+25A0...U+25FF
//! <https://en.wikipedia.org/wiki/Geometric_Shapes_(Unicode_block)>
//!
//! ■ □ ▢ ▣ ▤ ▥ ▦ ▧ ▨ ▩ ▪ ▫ ▬ ▭ ▮ ▯
//! ▰ ▱ ▲ △ ▴ ▵ ▶ ▷ ▸ ▹ ► ▻ ▼ ▽ ▾ ▿
//! ◀ ◁ ◂ ◃ ◄ ◅ ◆ ◇ ◈ ◉ ◊ ○ ◌ ◍ ◎ ●
//! ◐ ◑ ◒ ◓ ◔ ◕ ◖ ◗ ◘ ◙ ◚ ◛ ◜ ◝ ◞ ◟
//! ◠ ◡ ◢ ◣ ◤ ◥ ◦ ◧ ◨ ◩ ◪ ◫ ◬ ◭ ◮ ◯
//! ◰ ◱ ◲ ◳ ◴ ◵ ◶ ◷ ◸ ◹ ◺ ◻ ◼ ◽︎◾︎◿
//!
//! Only a subset of this block is viable for sprite drawing; filling
//! out this file to have full coverage of this block is not the goal.
//!
//! Port of `sprite/draw/geometric_shapes.zig`.

use super::common::{Corner, DrawFn, Range, Shade, Thickness};
use crate::metrics::Metrics;
use crate::sprite::canvas::{Canvas, CapMode, Color, DrawError, Path, StrokeOptions};

pub const RANGES: &[Range] = &[
    Range {
        min: 0x25e2,
        max: 0x25e5,
        draw: draw25e2_25e5 as DrawFn,
    },
    Range {
        min: 0x25f8,
        max: 0x25fa,
        draw: draw25f8_25fa as DrawFn,
    },
    Range {
        min: 0x25ff,
        max: 0x25ff,
        draw: draw25ff as DrawFn,
    },
];

/// ◢ ◣ ◤ ◥
pub fn draw25e2_25e5(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        // ◢
        0x25e2 => corner_triangle_shade(metrics, canvas, Corner::Br, Shade::ON),
        // ◣
        0x25e3 => corner_triangle_shade(metrics, canvas, Corner::Bl, Shade::ON),
        // ◤
        0x25e4 => corner_triangle_shade(metrics, canvas, Corner::Tl, Shade::ON),
        // ◥
        0x25e5 => corner_triangle_shade(metrics, canvas, Corner::Tr, Shade::ON),

        _ => unreachable!(),
    }
}

/// ◸ ◹ ◺
pub fn draw25f8_25fa(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        // ◸
        0x25f8 => corner_triangle_outline(metrics, canvas, Corner::Tl),
        // ◹
        0x25f9 => corner_triangle_outline(metrics, canvas, Corner::Tr),
        // ◺
        0x25fa => corner_triangle_outline(metrics, canvas, Corner::Bl),

        _ => unreachable!(),
    }
}

/// ◿
pub fn draw25ff(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    corner_triangle_outline(metrics, canvas, Corner::Br)
}

fn corner_path(metrics: &Metrics, corner: Corner) -> Path {
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);

    let (x0, y0, x1, y1, x2, y2) = match corner {
        Corner::Tl => (0.0, 0.0, 0.0, float_height, float_width, 0.0),
        Corner::Tr => (0.0, 0.0, float_width, float_height, float_width, 0.0),
        Corner::Bl => (0.0, 0.0, 0.0, float_height, float_width, float_height),
        Corner::Br => (
            0.0,
            float_height,
            float_width,
            float_height,
            float_width,
            0.0,
        ),
    };

    let mut path = Path::new();
    path.move_to(x0, y0);
    path.line_to(x1, y1);
    path.line_to(x2, y2);
    path.close();
    path
}

pub fn corner_triangle_shade(
    metrics: &Metrics,
    canvas: &mut Canvas,
    corner: Corner,
    shade: Shade,
) -> Result<(), DrawError> {
    let path = corner_path(metrics, corner);
    canvas.fill_path(&path, Color(shade.0))
}

pub fn corner_triangle_outline(
    metrics: &Metrics,
    canvas: &mut Canvas,
    corner: Corner,
) -> Result<(), DrawError> {
    let float_thick = f64::from(Thickness::Light.height(metrics.box_thickness));
    let path = corner_path(metrics, corner);

    canvas.inner_stroke_path(
        &path,
        StrokeOptions {
            line_cap_mode: CapMode::Butt,
            line_width: float_thick,
        },
        Color::ON,
    )
}
