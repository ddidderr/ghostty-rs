//! Powerline + Powerline Extra Symbols | U+E0B0...U+E0D4
//! <https://github.com/ryanoasis/powerline-extra-symbols>
//!
//!                
//!                  
//!      
//!
//! We implement the more geometric glyphs here, but not the stylized ones.
//!
//! Port of `sprite/draw/powerline.zig`.

use std::f64::consts::SQRT_2;

use super::box_;
use super::common::{DrawFn, Range, Thickness};
use crate::metrics::Metrics;
use crate::sprite::canvas::{
    Canvas, CapMode, Color, DrawError, Path, Point, StrokeOptions, Triangle,
};

pub const RANGES: &[Range] = &[
    Range {
        min: 0xe0b0,
        max: 0xe0b0,
        draw: drawe0b0 as DrawFn,
    },
    Range {
        min: 0xe0b1,
        max: 0xe0b1,
        draw: drawe0b1 as DrawFn,
    },
    Range {
        min: 0xe0b2,
        max: 0xe0b2,
        draw: drawe0b2 as DrawFn,
    },
    Range {
        min: 0xe0b3,
        max: 0xe0b3,
        draw: drawe0b3 as DrawFn,
    },
    Range {
        min: 0xe0b4,
        max: 0xe0b4,
        draw: drawe0b4 as DrawFn,
    },
    Range {
        min: 0xe0b5,
        max: 0xe0b5,
        draw: drawe0b5 as DrawFn,
    },
    Range {
        min: 0xe0b6,
        max: 0xe0b6,
        draw: drawe0b6 as DrawFn,
    },
    Range {
        min: 0xe0b7,
        max: 0xe0b7,
        draw: drawe0b7 as DrawFn,
    },
    Range {
        min: 0xe0b8,
        max: 0xe0b8,
        draw: drawe0b8 as DrawFn,
    },
    Range {
        min: 0xe0b9,
        max: 0xe0b9,
        draw: drawe0b9 as DrawFn,
    },
    Range {
        min: 0xe0ba,
        max: 0xe0ba,
        draw: drawe0ba as DrawFn,
    },
    Range {
        min: 0xe0bb,
        max: 0xe0bb,
        draw: drawe0bb as DrawFn,
    },
    Range {
        min: 0xe0bc,
        max: 0xe0bc,
        draw: drawe0bc as DrawFn,
    },
    Range {
        min: 0xe0bd,
        max: 0xe0bd,
        draw: drawe0bd as DrawFn,
    },
    Range {
        min: 0xe0be,
        max: 0xe0be,
        draw: drawe0be as DrawFn,
    },
    Range {
        min: 0xe0bf,
        max: 0xe0bf,
        draw: drawe0bf as DrawFn,
    },
    Range {
        min: 0xe0d2,
        max: 0xe0d2,
        draw: drawe0d2 as DrawFn,
    },
    Range {
        min: 0xe0d4,
        max: 0xe0d4,
        draw: drawe0d4 as DrawFn,
    },
];

/// E0B0
pub fn drawe0b0(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point { x: 0.0, y: 0.0 },
            p1: Point {
                x: float_width,
                y: float_height / 2.0,
            },
            p2: Point {
                x: 0.0,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0B2
pub fn drawe0b2(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point {
                x: float_width,
                y: 0.0,
            },
            p1: Point {
                x: 0.0,
                y: float_height / 2.0,
            },
            p2: Point {
                x: float_width,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0B8
pub fn drawe0b8(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point { x: 0.0, y: 0.0 },
            p1: Point {
                x: float_width,
                y: float_height,
            },
            p2: Point {
                x: 0.0,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0BA
pub fn drawe0ba(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point {
                x: float_width,
                y: 0.0,
            },
            p1: Point {
                x: float_width,
                y: float_height,
            },
            p2: Point {
                x: 0.0,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0BC
pub fn drawe0bc(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point { x: 0.0, y: 0.0 },
            p1: Point {
                x: float_width,
                y: 0.0,
            },
            p2: Point {
                x: 0.0,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0BE
pub fn drawe0be(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    canvas.triangle(
        Triangle {
            p0: Point { x: 0.0, y: 0.0 },
            p1: Point {
                x: float_width,
                y: 0.0,
            },
            p2: Point {
                x: float_width,
                y: float_height,
            },
        },
        Color::ON,
    )
}

/// E0B9
pub fn drawe0b9(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::light_diagonal_upper_left_to_lower_right(metrics, canvas);
    Ok(())
}

/// E0BB
pub fn drawe0bb(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::light_diagonal_upper_right_to_lower_left(metrics, canvas);
    Ok(())
}

/// E0BD
pub fn drawe0bd(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::light_diagonal_upper_right_to_lower_left(metrics, canvas);
    Ok(())
}

/// E0BF
pub fn drawe0bf(
    _cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    box_::light_diagonal_upper_left_to_lower_right(metrics, canvas);
    Ok(())
}

/// E0B1
pub fn drawe0b1(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);

    let mut path = Path::new();
    path.move_to(0.0, 0.0);
    path.line_to(float_width, float_height / 2.0);
    path.line_to(0.0, float_height);

    canvas.stroke_path(
        &path,
        StrokeOptions {
            line_cap_mode: CapMode::Butt,
            line_width: f64::from(Thickness::Light.height(metrics.box_thickness)),
        },
        Color::ON,
    )
}

/// E0B3
pub fn drawe0b3(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    drawe0b1(cp, canvas, width, height, metrics)?;
    canvas.flip_horizontal();
    Ok(())
}

/// Path for the rounded "half circle" shapes E0B4 / E0B5 (open path; E0B4
/// closes it itself).
fn rounded_path(width: u32, height: u32) -> Path {
    let float_width = f64::from(width);
    let float_height = f64::from(height);

    // Coefficient for approximating a circular arc.
    let c: f64 = (SQRT_2 - 1.0) * 4.0 / 3.0;

    let radius: f64 = float_width.min(float_height / 2.0);

    let mut path = Path::new();
    path.move_to(0.0, 0.0);
    path.curve_to(radius * c, 0.0, radius, radius - radius * c, radius, radius);
    path.line_to(radius, float_height - radius);
    path.curve_to(
        radius,
        float_height - radius + radius * c,
        radius * c,
        float_height,
        0.0,
        float_height,
    );
    path
}

/// E0B4
pub fn drawe0b4(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    _metrics: &Metrics,
) -> Result<(), DrawError> {
    let mut path = rounded_path(width, height);
    path.close();
    canvas.fill_path(&path, Color::ON)
}

/// E0B5
pub fn drawe0b5(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let path = rounded_path(width, height);
    canvas.inner_stroke_path(
        &path,
        StrokeOptions {
            line_width: f64::from(metrics.box_thickness),
            line_cap_mode: CapMode::Butt,
        },
        Color::ON,
    )
}

/// E0B6
pub fn drawe0b6(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    drawe0b4(cp, canvas, width, height, metrics)?;
    canvas.flip_horizontal();
    Ok(())
}

/// E0B7
pub fn drawe0b7(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    drawe0b5(cp, canvas, width, height, metrics)?;
    canvas.flip_horizontal();
    Ok(())
}

/// E0D2
pub fn drawe0d2(
    _cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    let float_width = f64::from(width);
    let float_height = f64::from(height);
    let float_thick = f64::from(metrics.box_thickness);

    // Top piece
    {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.line_to(float_width, 0.0);
        path.line_to(float_width / 2.0, float_height / 2.0 - float_thick / 2.0);
        path.line_to(0.0, float_height / 2.0 - float_thick / 2.0);
        path.close();

        canvas.fill_path(&path, Color::ON)?;
    }

    // Bottom piece
    {
        let mut path = Path::new();
        path.move_to(0.0, float_height);
        path.line_to(float_width, float_height);
        path.line_to(float_width / 2.0, float_height / 2.0 + float_thick / 2.0);
        path.line_to(0.0, float_height / 2.0 + float_thick / 2.0);
        path.close();

        canvas.fill_path(&path, Color::ON)?;
    }
    Ok(())
}

/// E0D4
pub fn drawe0d4(
    cp: u32,
    canvas: &mut Canvas,
    width: u32,
    height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    drawe0d2(cp, canvas, width, height, metrics)?;
    canvas.flip_horizontal();
    Ok(())
}
