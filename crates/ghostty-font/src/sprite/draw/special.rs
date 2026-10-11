//! This file contains glyph drawing functions for all of the
//! non-Unicode sprite glyphs, such as cursors and underlines.
//! Port of `sprite/draw/special.zig`.
//!
//! The function names match the variants of [`Sprite`](crate::sprite::Sprite).

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

use crate::metrics::Metrics;
use crate::sprite::canvas::{Canvas, CapMode, Color, DrawError, Path, Rect, StrokeOptions};

pub fn underline(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We can go beyond the height of the cell a bit, but
    // we want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell below the cell
    // height.
    let y = metrics.underline_position.min(
        height
            .saturating_add(canvas.padding_y)
            .saturating_sub(metrics.underline_thickness),
    );

    canvas.rect(
        Rect {
            x: 0,
            y: y as i32,
            width: width as i32,
            height: metrics.underline_thickness as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn underline_double(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We can go beyond the height of the cell a bit, but
    // we want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell below the cell
    // height.
    let y = metrics.underline_position.min(
        height
            .saturating_add(canvas.padding_y)
            .saturating_sub(2 * metrics.underline_thickness),
    );

    // We place one underline above the underline position, and one below
    // by one thickness, creating a "negative" underline where the single
    // underline would be placed.
    canvas.rect(
        Rect {
            x: 0,
            y: y.saturating_sub(metrics.underline_thickness) as i32,
            width: width as i32,
            height: metrics.underline_thickness as i32,
        },
        Color::ON,
    );
    canvas.rect(
        Rect {
            x: 0,
            y: y.saturating_add(metrics.underline_thickness) as i32,
            width: width as i32,
            height: metrics.underline_thickness as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn underline_dotted(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    let float_pos = f64::from(metrics.underline_position);
    let float_thick = f64::from(metrics.underline_thickness);

    // The diameter will be sqrt2 * the usual underline thickness
    // since otherwise dotted underlines look somewhat anemic.
    let radius = FRAC_1_SQRT_2 * float_thick;

    // We can go beyond the height of the cell a bit, but
    // we want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell below the cell
    // height.
    let padding = f64::from(canvas.padding_y);
    let y = (
        // The center of the underline stem.
        float_pos + 0.5 * float_thick
    )
        .min(
            // The lowest we can go on the canvas and not get clipped.
            float_height + padding - radius.ceil(),
        );

    let dot_count = (
        // We should try to have enough dots that the
        // space between them matches their diameter.
        (float_width / (4.0 * radius))
            .ceil()
            // And not enough that the space between
            // each dot is less than their radius.
            .min((float_width / (3.0 * radius)).floor())
            // And definitely not enough that the space
            // between them is less than a single pixel.
            .min((float_width / (2.0 * radius + 1.0)).floor())
    )
    // And we must have at least one dot per cell.
    .max(1.0);

    // What we essentially do is divide the cell in to
    // dot_count areas with a dot centered in each one.
    let mut path = Path::new();
    let mut x = (float_width / dot_count) / 2.0;
    for _ in 0..(dot_count as usize) {
        path.arc(x, y, radius, 0.0, TAU);
        path.close();
        x += float_width / dot_count;
    }

    canvas.fill_path(&path, Color::ON)
}

pub fn underline_dashed(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We can go beyond the height of the cell a bit, but
    // we want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell below the cell
    // height.
    let y = metrics.underline_position.min(
        height
            .saturating_add(canvas.padding_y)
            .saturating_sub(metrics.underline_thickness),
    );

    let dash_width = width / 3 + 1;
    let dash_count = (width / dash_width) + 1;
    let mut i = 0;
    while i < dash_count {
        let x = i * dash_width;
        canvas.rect(
            Rect {
                x: x as i32,
                y: y as i32,
                width: dash_width as i32,
                height: metrics.underline_thickness as i32,
            },
            Color::ON,
        );
        i += 2;
    }
    Ok(())
}

pub fn underline_curly(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    let float_pos = f64::from(metrics.underline_position);

    // Because of we way we draw the undercurl, we end up making it around 1px
    // thicker than it should be, to fix this we just reduce the thickness by 1.
    //
    // We use a minimum thickness of 0.414 because this empirically produces
    // the nicest undercurls at 1px underline thickness; thinner tends to look
    // too thin compared to straight underlines and has artefacting.
    let line_width = f64::from(metrics.underline_thickness);

    // Empirically this looks good.
    let amplitude = float_width / PI;

    // Make sure we don't exceed the drawable area. This can still be outside
    // of the cell by some amount (one quarter of the height), but we don't
    // want underlines to disappear for fonts with bad metadata or when users
    // set their underline position way too low.
    let padding = f64::from(canvas.padding_y);
    let top = float_pos.min(
        // The lowest we can draw this and not get clipped.
        float_height + padding - amplitude - line_width,
    );
    let bottom = top + amplitude;

    // Curvature multiplier.
    // To my eye, 0.4 creates a nice smooth wiggle.
    let r = 0.4;

    let center = 0.5 * float_width;

    // We create a single cycle of a wave that peaks at the center of the cell.
    let mut path = Path::new();
    path.move_to(0.0, bottom);
    path.curve_to(center * r, bottom, center - center * r, top, center, top);
    path.curve_to(
        center + center * r,
        top,
        float_width - center * r,
        bottom,
        float_width,
        bottom,
    );
    canvas.stroke_path(
        &path,
        StrokeOptions {
            line_width,
            // Rounded caps, adjacent underlines will have these overlap and so not be
            // visible, but it makes the ends look cleaner.
            line_cap_mode: CapMode::Round,
        },
        Color::ON,
    )
}

pub fn strikethrough(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    canvas.rect(
        Rect {
            x: 0,
            y: metrics.strikethrough_position as i32,
            width: width as i32,
            height: metrics.strikethrough_thickness as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn overline(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We can go beyond the top of the cell a bit, but we
    // want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell above the top
    // of the cell.
    let y = metrics.overline_position.max(-(canvas.padding_y as i32));

    canvas.rect(
        Rect {
            x: 0,
            y,
            width: width as i32,
            height: metrics.overline_thickness as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn cursor_rect(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    canvas.rect(
        Rect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn cursor_hollow_rect(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We fill the entire rect and then hollow out the inside, this isn't very
    // efficient but it doesn't need to be and it's the easiest way to write it.
    canvas.rect(
        Rect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        },
        Color::ON,
    );
    canvas.rect(
        Rect {
            x: metrics.cursor_thickness as i32,
            y: metrics.cursor_thickness as i32,
            width: width.saturating_sub(metrics.cursor_thickness * 2) as i32,
            height: height.saturating_sub(metrics.cursor_thickness * 2) as i32,
        },
        Color::OFF,
    );
    Ok(())
}

pub fn cursor_bar(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We place the bar cursor half of its thickness over the left edge of the
    // cell, so that it sits centered between characters, not biased to a side.
    //
    // We round up (add 1 before dividing by 2) because, empirically, having a
    // 1px cursor shifted left a pixel looks better than having it not shifted.
    canvas.rect(
        Rect {
            x: -(metrics.cursor_thickness.div_ceil(2) as i32),
            y: 0,
            width: metrics.cursor_thickness as i32,
            height: height as i32,
        },
        Color::ON,
    );
    Ok(())
}

pub fn cursor_underline(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    // We can go beyond the height of the cell a bit, but
    // we want to be sure never to exceed the height of the
    // canvas, which extends a quarter cell below the cell
    // height.
    let y = metrics.underline_position.min(
        height
            .saturating_add(canvas.padding_y)
            .saturating_sub(metrics.underline_thickness),
    );

    canvas.rect(
        Rect {
            x: 0,
            y: y as i32,
            width: width as i32,
            height: metrics.cursor_thickness as i32,
        },
        Color::ON,
    );
    Ok(())
}
