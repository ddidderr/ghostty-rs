//! Block Elements | U+2580...U+259F
//! <https://en.wikipedia.org/wiki/Block_Elements>
//!
//! ▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏
//! ▐░▒▓▔▕▖▗▘▙▚▛▜▝▞▟
//!
//! Port of `sprite/draw/block.zig`.

use super::common::{
    Alignment, DrawFn, FIVE_EIGHTHS, Fraction, HALF, HAlign, ONE_EIGHTH, ONE_QUARTER, Quads, Range,
    SEVEN_EIGHTHS, Shade, THREE_EIGHTHS, THREE_QUARTERS, VAlign, fill,
};
use crate::metrics::Metrics;
use crate::sprite::canvas::{Canvas, Color, DrawError, Rect};

pub const RANGES: &[Range] = &[Range {
    min: 0x2580,
    max: 0x259f,
    draw: draw2580_259f as DrawFn,
}];

pub fn draw2580_259f(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let q = |tl, tr, bl, br| Quads { tl, tr, bl, br };
    match cp {
        // '▀' UPPER HALF BLOCK
        0x2580 => block(metrics, canvas, Alignment::UPPER, 1.0, HALF),
        // '▁' LOWER ONE EIGHTH BLOCK
        0x2581 => block(metrics, canvas, Alignment::LOWER, 1.0, ONE_EIGHTH),
        // '▂' LOWER ONE QUARTER BLOCK
        0x2582 => block(metrics, canvas, Alignment::LOWER, 1.0, ONE_QUARTER),
        // '▃' LOWER THREE EIGHTHS BLOCK
        0x2583 => block(metrics, canvas, Alignment::LOWER, 1.0, THREE_EIGHTHS),
        // '▄' LOWER HALF BLOCK
        0x2584 => block(metrics, canvas, Alignment::LOWER, 1.0, HALF),
        // '▅' LOWER FIVE EIGHTHS BLOCK
        0x2585 => block(metrics, canvas, Alignment::LOWER, 1.0, FIVE_EIGHTHS),
        // '▆' LOWER THREE QUARTERS BLOCK
        0x2586 => block(metrics, canvas, Alignment::LOWER, 1.0, THREE_QUARTERS),
        // '▇' LOWER SEVEN EIGHTHS BLOCK
        0x2587 => block(metrics, canvas, Alignment::LOWER, 1.0, SEVEN_EIGHTHS),
        // '█' FULL BLOCK
        0x2588 => full_block_shade(metrics, canvas, Shade::ON),
        // '▉' LEFT SEVEN EIGHTHS BLOCK
        0x2589 => block(metrics, canvas, Alignment::LEFT, SEVEN_EIGHTHS, 1.0),
        // '▊' LEFT THREE QUARTERS BLOCK
        0x258a => block(metrics, canvas, Alignment::LEFT, THREE_QUARTERS, 1.0),
        // '▋' LEFT FIVE EIGHTHS BLOCK
        0x258b => block(metrics, canvas, Alignment::LEFT, FIVE_EIGHTHS, 1.0),
        // '▌' LEFT HALF BLOCK
        0x258c => block(metrics, canvas, Alignment::LEFT, HALF, 1.0),
        // '▍' LEFT THREE EIGHTHS BLOCK
        0x258d => block(metrics, canvas, Alignment::LEFT, THREE_EIGHTHS, 1.0),
        // '▎' LEFT ONE QUARTER BLOCK
        0x258e => block(metrics, canvas, Alignment::LEFT, ONE_QUARTER, 1.0),
        // '▏' LEFT ONE EIGHTH BLOCK
        0x258f => block(metrics, canvas, Alignment::LEFT, ONE_EIGHTH, 1.0),

        // '▐' RIGHT HALF BLOCK
        0x2590 => block(metrics, canvas, Alignment::RIGHT, HALF, 1.0),
        // '░'
        0x2591 => full_block_shade(metrics, canvas, Shade::LIGHT),
        // '▒'
        0x2592 => full_block_shade(metrics, canvas, Shade::MEDIUM),
        // '▓'
        0x2593 => full_block_shade(metrics, canvas, Shade::DARK),
        // '▔' UPPER ONE EIGHTH BLOCK
        0x2594 => block(metrics, canvas, Alignment::UPPER, 1.0, ONE_EIGHTH),
        // '▕' RIGHT ONE EIGHTH BLOCK
        0x2595 => block(metrics, canvas, Alignment::RIGHT, ONE_EIGHTH, 1.0),
        // '▖'
        0x2596 => quadrant(metrics, canvas, q(false, false, true, false)),
        // '▗'
        0x2597 => quadrant(metrics, canvas, q(false, false, false, true)),
        // '▘'
        0x2598 => quadrant(metrics, canvas, q(true, false, false, false)),
        // '▙'
        0x2599 => quadrant(metrics, canvas, q(true, false, true, true)),
        // '▚'
        0x259a => quadrant(metrics, canvas, q(true, false, false, true)),
        // '▛'
        0x259b => quadrant(metrics, canvas, q(true, true, true, false)),
        // '▜'
        0x259c => quadrant(metrics, canvas, q(true, true, false, true)),
        // '▝'
        0x259d => quadrant(metrics, canvas, q(false, true, false, false)),
        // '▞'
        0x259e => quadrant(metrics, canvas, q(false, true, true, false)),
        // '▟'
        0x259f => quadrant(metrics, canvas, q(false, true, true, true)),

        _ => unreachable!(),
    }
    Ok(())
}

pub fn block(
    metrics: &Metrics,
    canvas: &mut Canvas,
    alignment: Alignment,
    width: f64,
    height: f64,
) {
    block_shade(metrics, canvas, alignment, width, height, Shade::ON);
}

pub fn block_shade(
    metrics: &Metrics,
    canvas: &mut Canvas,
    alignment: Alignment,
    width: f64,
    height: f64,
    shade: Shade,
) {
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);

    let w = (float_width * width).round() as u32;
    let h = (float_height * height).round() as u32;

    let x = match alignment.horizontal {
        HAlign::Left => 0,
        HAlign::Right => metrics.cell_width - w,
        HAlign::Center => (metrics.cell_width - w) / 2,
    };
    let y = match alignment.vertical {
        VAlign::Top => 0,
        VAlign::Bottom => metrics.cell_height - h,
        VAlign::Middle => (metrics.cell_height - h) / 2,
    };

    canvas.rect(
        Rect {
            x: x as i32,
            y: y as i32,
            width: w as i32,
            height: h as i32,
        },
        Color(shade.0),
    );
}

pub fn full_block_shade(metrics: &Metrics, canvas: &mut Canvas, shade: Shade) {
    canvas.box_(
        0,
        0,
        metrics.cell_width as i32,
        metrics.cell_height as i32,
        Color(shade.0),
    );
}

fn quadrant(metrics: &Metrics, canvas: &mut Canvas, quads: Quads) {
    use Fraction::{Full, Half, Zero};
    if quads.tl {
        fill(metrics, canvas, Zero, Half, Zero, Half);
    }
    if quads.tr {
        fill(metrics, canvas, Half, Full, Zero, Half);
    }
    if quads.bl {
        fill(metrics, canvas, Zero, Half, Half, Full);
    }
    if quads.br {
        fill(metrics, canvas, Half, Full, Half, Full);
    }
}
