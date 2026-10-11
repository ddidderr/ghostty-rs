//! This exposes primitives to draw 2D graphics and export the graphic to
//! a font atlas. Port of `font/sprite/canvas.zig`.
//!
//! Ghostty uses the `z2d` library for antialiased path drawing; this port
//! uses `tiny-skia` to rasterize path coverage (see [`Canvas::fill_path`]).

use std::ops::Sub;

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::atlas::{self, Atlas, Format, Region};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Line<T> {
    pub p0: Point<T>,
    pub p1: Point<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Box<T> {
    pub p0: Point<T>,
    pub p1: Point<T>,
}

impl<T: Copy + PartialOrd + Sub<Output = T>> Box<T> {
    pub fn rect(&self) -> Rect<T> {
        let min = |a: T, b: T| if b < a { b } else { a };
        let max = |a: T, b: T| if b > a { b } else { a };
        let tl_x = min(self.p0.x, self.p1.x);
        let tl_y = min(self.p0.y, self.p1.y);
        let br_x = max(self.p0.x, self.p1.x);
        let br_y = max(self.p0.y, self.p1.y);

        Rect {
            x: tl_x,
            y: tl_y,
            width: br_x - tl_x,
            height: br_y - tl_y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect<T> {
    pub x: T,
    pub y: T,
    pub width: T,
    pub height: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Triangle<T> {
    pub p0: Point<T>,
    pub p1: Point<T>,
    pub p2: Point<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quad<T> {
    pub p0: Point<T>,
    pub p1: Point<T>,
    pub p2: Point<T>,
    pub p3: Point<T>,
}

/// We only use alpha-channel so a pixel is an 8-bit alpha value. Zig models
/// this as a non-exhaustive `enum(u8)`; `ON` = 255, `OFF` = 0, and any other
/// value is a partial alpha (used for shades).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color(pub u8);

impl Color {
    pub const ON: Color = Color(255);
    pub const OFF: Color = Color(0);
}

/// Errors that drawing functions can produce.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawError {
    /// Something went wrong while doing math.
    #[error("math error")]
    MathError,
    #[error("atlas error: {0}")]
    Atlas(#[from] atlas::Error),
    #[error("path error")]
    Path,
}

/// A node of a [`Path`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum PathNode {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    CurveTo(f64, f64, f64, f64, f64, f64),
    ClosePath,
}

/// A path in cell-relative coordinates (the canvas applies the padding
/// translation when the path is filled or stroked, like z2d's
/// `staticPath` transformation).
#[derive(Debug, Clone, Default)]
pub struct Path {
    nodes: Vec<PathNode>,
    current: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.nodes.push(PathNode::MoveTo(x, y));
        self.current = Some((x, y));
        self.start = Some((x, y));
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        if self.current.is_none() {
            self.move_to(x, y);
            return;
        }
        self.nodes.push(PathNode::LineTo(x, y));
        self.current = Some((x, y));
    }

    pub fn curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) {
        if self.current.is_none() {
            self.move_to(x1, y1);
        }
        self.nodes.push(PathNode::CurveTo(x1, y1, x2, y2, x3, y3));
        self.current = Some((x3, y3));
    }

    /// Close the current subpath. Like z2d, the current point returns to
    /// the start of the subpath.
    pub fn close(&mut self) {
        if self.current.is_none() {
            return;
        }
        self.nodes.push(PathNode::ClosePath);
        self.current = self.start;
    }

    /// Add a circular arc centered at `(x, y)` from angle `a0` to `a1`
    /// (radians, clockwise in canvas space with +y down). If there is a
    /// current point, a line is drawn to the start of the arc, otherwise
    /// the arc starts a new subpath.
    pub fn arc(&mut self, x: f64, y: f64, radius: f64, a0: f64, a1: f64) {
        use std::f64::consts::FRAC_PI_2;

        let (sx, sy) = (x + radius * a0.cos(), y + radius * a0.sin());
        if self.current.is_some() {
            self.line_to(sx, sy);
        } else {
            self.move_to(sx, sy);
        }

        let mut sweep = a1 - a0;
        // Split into segments of at most a quarter turn.
        let segs = ((sweep.abs() / FRAC_PI_2).ceil() as usize).max(1);
        sweep /= segs as f64;
        let k = 4.0 / 3.0 * (sweep / 4.0).tan();
        let mut a = a0;
        for _ in 0..segs {
            let (s0, c0) = a.sin_cos();
            let (s1, c1) = (a + sweep).sin_cos();
            self.curve_to(
                x + radius * (c0 - k * s0),
                y + radius * (s0 + k * c0),
                x + radius * (c1 + k * s1),
                y + radius * (s1 - k * c1),
                x + radius * c1,
                y + radius * s1,
            );
            a += sweep;
        }
    }

    fn to_skia(&self, dx: f64, dy: f64) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        let f = |v: f64, d: f64| (v + d) as f32;
        for n in &self.nodes {
            match *n {
                PathNode::MoveTo(x, y) => pb.move_to(f(x, dx), f(y, dy)),
                PathNode::LineTo(x, y) => pb.line_to(f(x, dx), f(y, dy)),
                PathNode::CurveTo(x1, y1, x2, y2, x3, y3) => pb.cubic_to(
                    f(x1, dx),
                    f(y1, dy),
                    f(x2, dx),
                    f(y2, dy),
                    f(x3, dx),
                    f(y3, dy),
                ),
                PathNode::ClosePath => pb.close(),
            }
        }
        pb.finish()
    }
}

/// Line cap modes (subset of z2d's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapMode {
    #[default]
    Butt,
    Round,
    Square,
}

/// Stroke options (subset of z2d's `StrokeOptions`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeOptions {
    pub line_width: f64,
    pub line_cap_mode: CapMode,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        StrokeOptions {
            line_width: 2.0,
            line_cap_mode: CapMode::Butt,
        }
    }
}

/// The canvas is an 8-bit alpha surface with padding on all sides, onto
/// which sprite glyphs are drawn.
#[derive(Debug)]
pub struct Canvas {
    /// The underlying alpha8 surface, row-major, `sfc_width * sfc_height`.
    pub sfc: Vec<u8>,
    pub sfc_width: u32,
    pub sfc_height: u32,

    pub padding_x: u32,
    pub padding_y: u32,

    pub clip_top: u32,
    pub clip_left: u32,
    pub clip_right: u32,
    pub clip_bottom: u32,

    /// Scratch pixmap used to rasterize path coverage.
    scratch: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32, padding_x: u32, padding_y: u32) -> Canvas {
        // We add padding to both sides (hence `2 *`)
        let w = width + 2 * padding_x;
        let h = height + 2 * padding_y;
        Canvas {
            sfc: vec![0; w as usize * h as usize],
            sfc_width: w,
            sfc_height: h,
            padding_x,
            padding_y,
            clip_top: 0,
            clip_left: 0,
            clip_right: 0,
            clip_bottom: 0,
            // tiny-skia can't make zero sized pixmaps; sprites are never empty
            // but be defensive.
            scratch: Pixmap::new(w.max(1), h.max(1)).expect("non-zero pixmap size"),
        }
    }

    /// Write the data in this drawing to the atlas.
    pub fn write_atlas(&mut self, atlas: &mut Atlas) -> Result<Region, atlas::Error> {
        assert_eq!(atlas.format, Format::Grayscale);

        self.trim();

        let sfc_width = self.sfc_width;
        let sfc_height = self.sfc_height;

        // Subtract our clip margins from the
        // width and height to get region size.
        let region_width = sfc_width
            .saturating_sub(self.clip_left)
            .saturating_sub(self.clip_right);
        let region_height = sfc_height
            .saturating_sub(self.clip_top)
            .saturating_sub(self.clip_bottom);

        // Allocate our texture atlas region
        let region = atlas.reserve(region_width, region_height)?;

        if region.width > 0 && region.height > 0 {
            // Write the glyph information into the atlas
            assert_eq!(region.width, region_width);
            assert_eq!(region.height, region_height);
            atlas.set_from_larger(region, &self.sfc, sfc_width, self.clip_left, self.clip_top);
        }

        Ok(region)
    }

    /// Adjust clip boundaries to trim off any fully transparent rows or columns.
    fn trim(&mut self) {
        let width = self.sfc_width;
        let height = self.sfc_height;
        let buf = &self.sfc;
        let px = |x: u32, y: u32| buf[y as usize * width as usize + x as usize];

        while self.clip_top < height - self.clip_bottom {
            let y = self.clip_top;
            let x0 = self.clip_left;
            let x1 = width - self.clip_right;
            if (x0..x1).any(|x| px(x, y) != 0) {
                break;
            }
            self.clip_top += 1;
        }

        while self.clip_bottom < height - self.clip_top {
            let y = (height - self.clip_bottom).saturating_sub(1);
            let x0 = self.clip_left;
            let x1 = width - self.clip_right;
            if (x0..x1).any(|x| px(x, y) != 0) {
                break;
            }
            self.clip_bottom += 1;
        }

        while self.clip_left < width - self.clip_right {
            let x = self.clip_left;
            let y0 = self.clip_top;
            let y1 = height - self.clip_bottom;
            if (y0..y1).any(|y| px(x, y) != 0) {
                break;
            }
            self.clip_left += 1;
        }

        while self.clip_right < width - self.clip_left {
            let x = (width - self.clip_right).saturating_sub(1);
            let y0 = self.clip_top;
            let y1 = height - self.clip_bottom;
            if (y0..y1).any(|y| px(x, y) != 0) {
                break;
            }
            self.clip_right += 1;
        }
    }

    /// Only really useful for test purposes, since the clipping region is
    /// automatically excluded when writing to an atlas with `write_atlas`.
    pub fn clear_clipping_regions(&mut self) {
        let width = self.sfc_width as usize;
        let height = self.sfc_height as usize;

        for y in 0..height {
            for x in 0..self.clip_left as usize {
                self.sfc[y * width + x] = 0;
            }
        }

        for y in 0..height {
            for x in (width - self.clip_right as usize)..width {
                self.sfc[y * width + x] = 0;
            }
        }

        for y in 0..self.clip_top as usize {
            for x in 0..width {
                self.sfc[y * width + x] = 0;
            }
        }

        for y in (height - self.clip_bottom as usize)..height {
            for x in 0..width {
                self.sfc[y * width + x] = 0;
            }
        }
    }

    /// Reset the canvas (pixels and clip margins) for reuse. Used by tests
    /// that render many glyphs onto a single canvas.
    pub fn reset(&mut self) {
        self.sfc.fill(0);
        self.clip_top = 0;
        self.clip_left = 0;
        self.clip_right = 0;
        self.clip_bottom = 0;
    }

    /// Draw and fill a single pixel (out-of-bounds pixels are ignored).
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        let x = i64::from(x) + i64::from(self.padding_x);
        let y = i64::from(y) + i64::from(self.padding_y);
        if x < 0 || y < 0 || x >= i64::from(self.sfc_width) || y >= i64::from(self.sfc_height) {
            return;
        }
        self.sfc[y as usize * self.sfc_width as usize + x as usize] = color.0;
    }

    /// Draw and fill a rectangle. This is the main primitive for drawing
    /// lines as well (which are just generally skinny rectangles...)
    pub fn rect(&mut self, v: Rect<i32>, color: Color) {
        let mut y = v.y;
        while y < v.y + v.height {
            let mut x = v.x;
            while x < v.x + v.width {
                self.pixel(x, y, color);
                x += 1;
            }
            y += 1;
        }
    }

    /// Convenience wrapper for `Canvas::rect`
    pub fn box_(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        self.rect(
            Box {
                p0: Point { x: x0, y: y0 },
                p1: Point { x: x1, y: y1 },
            }
            .rect(),
            color,
        );
    }

    /// Draw and fill a quad.
    pub fn quad(&mut self, q: Quad<f64>, color: Color) -> Result<(), DrawError> {
        let mut path = Path::new();
        path.move_to(q.p0.x, q.p0.y);
        path.line_to(q.p1.x, q.p1.y);
        path.line_to(q.p2.x, q.p2.y);
        path.line_to(q.p3.x, q.p3.y);
        path.close();
        self.fill_path(&path, color)
    }

    /// Draw and fill a triangle.
    pub fn triangle(&mut self, t: Triangle<f64>, color: Color) -> Result<(), DrawError> {
        let mut path = Path::new();
        path.move_to(t.p0.x, t.p0.y);
        path.line_to(t.p1.x, t.p1.y);
        path.line_to(t.p2.x, t.p2.y);
        path.close();
        self.fill_path(&path, color)
    }

    /// Stroke a line.
    pub fn line(&mut self, l: Line<f64>, thickness: f64, color: Color) -> Result<(), DrawError> {
        let mut path = Path::new();
        path.move_to(l.p0.x, l.p0.y);
        path.line_to(l.p1.x, l.p1.y);
        self.stroke_path(
            &path,
            StrokeOptions {
                line_cap_mode: CapMode::Butt,
                line_width: thickness,
            },
            color,
        )
    }

    /// Rasterize `path` (filled or stroked) into the scratch pixmap and
    /// return nothing; the coverage is then in `self.scratch`'s alpha.
    fn rasterize(&mut self, path: &Path, stroke: Option<StrokeOptions>) -> bool {
        self.scratch.fill(tiny_skia::Color::TRANSPARENT);
        let Some(p) = path.to_skia(0.0, 0.0) else {
            return false;
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(0, 0, 0, 255);
        paint.anti_alias = true;
        let ts = Transform::from_translate(self.padding_x as f32, self.padding_y as f32);
        match stroke {
            None => {
                self.scratch
                    .fill_path(&p, &paint, FillRule::Winding, ts, None);
            }
            Some(opts) => {
                let s = Stroke {
                    width: opts.line_width as f32,
                    miter_limit: 10.0,
                    line_cap: match opts.line_cap_mode {
                        CapMode::Butt => LineCap::Butt,
                        CapMode::Round => LineCap::Round,
                        CapMode::Square => LineCap::Square,
                    },
                    line_join: LineJoin::Miter,
                    dash: None,
                };
                self.scratch.stroke_path(&p, &paint, &s, ts, None);
            }
        }
        true
    }

    /// Composite the scratch coverage onto the canvas using `src_over`
    /// with an opaque pattern of alpha `color`.
    fn composite_scratch(&mut self, color: Color) {
        let a = u32::from(color.0);
        for (dst, px) in self
            .sfc
            .iter_mut()
            .zip(self.scratch.data().as_chunks::<4>().0.iter())
        {
            let cov = u32::from(px[3]);
            if cov == 0 {
                continue;
            }
            let src = (cov * a + 127) / 255;
            let d = u32::from(*dst);
            *dst = (src + (d * (255 - src) + 127) / 255).min(255) as u8;
        }
    }

    /// Stroke a path.
    pub fn stroke_path(
        &mut self,
        path: &Path,
        opts: StrokeOptions,
        color: Color,
    ) -> Result<(), DrawError> {
        if self.rasterize(path, Some(opts)) {
            self.composite_scratch(color);
        }
        Ok(())
    }

    /// Do an inner stroke on a path: the shape is filled to make a mask,
    /// then stroked with double width, and the two are multiplied so only
    /// the part of the stroke inside the shape remains.
    pub fn inner_stroke_path(
        &mut self,
        path: &Path,
        opts: StrokeOptions,
        color: Color,
    ) -> Result<(), DrawError> {
        // Make a closed version of the path for our fill, so
        // that we can support open paths for inner stroke.
        let mut closed = path.clone();
        closed.close();

        let mut fill = vec![0u8; self.sfc.len()];
        if self.rasterize(&closed, None) {
            for (d, px) in fill
                .iter_mut()
                .zip(self.scratch.data().as_chunks::<4>().0.iter())
            {
                *d = px[3];
            }
        }

        // Stroke the shape with double the desired width.
        let mut stroke = vec![0u8; self.sfc.len()];
        let mut mut_opts = opts;
        mut_opts.line_width *= 2.0;
        if self.rasterize(path, Some(mut_opts)) {
            let a = u32::from(color.0);
            for (d, px) in stroke
                .iter_mut()
                .zip(self.scratch.data().as_chunks::<4>().0.iter())
            {
                *d = ((u32::from(px[3]) * a + 127) / 255) as u8;
            }
        }

        // Multiply the stroke onto the fill, then composite over the canvas.
        for ((dst, f), s) in self.sfc.iter_mut().zip(&fill).zip(&stroke) {
            let m = (255.0 * (f64::from(*s) / 255.0) * (f64::from(*f) / 255.0)).round() as u32;
            let d = u32::from(*dst);
            *dst = (m + (d * (255 - m) + 127) / 255).min(255) as u8;
        }
        Ok(())
    }

    /// Fill a path (non-zero winding).
    pub fn fill_path(&mut self, path: &Path, color: Color) -> Result<(), DrawError> {
        if self.rasterize(path, None) {
            self.composite_scratch(color);
        }
        Ok(())
    }

    /// Invert all pixels on the canvas.
    pub fn invert(&mut self) {
        for v in &mut self.sfc {
            *v = 255 - *v;
        }
    }

    /// Mirror the canvas horizontally.
    pub fn flip_horizontal(&mut self) {
        let width = self.sfc_width as usize;
        for row in self.sfc.chunks_exact_mut(width) {
            row.reverse();
        }
        std::mem::swap(&mut self.clip_left, &mut self.clip_right);
    }

    /// Mirror the canvas vertically.
    pub fn flip_vertical(&mut self) {
        let width = self.sfc_width as usize;
        let height = self.sfc_height as usize;
        for y in 0..height / 2 {
            let (top, bottom) = self.sfc.split_at_mut((height - y - 1) * width);
            top[y * width..(y + 1) * width].swap_with_slice(&mut bottom[..width]);
        }
        std::mem::swap(&mut self.clip_top, &mut self.clip_bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(c: &Canvas, x: u32, y: u32) -> u8 {
        c.sfc[y as usize * c.sfc_width as usize + x as usize]
    }

    #[test]
    fn box_rect_normalizes() {
        let b = Box {
            p0: Point { x: 5, y: 9 },
            p1: Point { x: 2, y: 3 },
        };
        assert_eq!(
            Rect {
                x: 2,
                y: 3,
                width: 3,
                height: 6
            },
            b.rect()
        );
    }

    #[test]
    fn rect_and_padding() {
        let mut c = Canvas::new(4, 4, 1, 1);
        assert_eq!((6, 6), (c.sfc_width, c.sfc_height));
        c.rect(
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            Color::ON,
        );
        assert_eq!(255, at(&c, 1, 1));
        assert_eq!(255, at(&c, 2, 1));
        assert_eq!(0, at(&c, 3, 1));
        assert_eq!(0, at(&c, 1, 2));
        // out of bounds is ignored
        c.rect(
            Rect {
                x: -10,
                y: -10,
                width: 100,
                height: 100,
            },
            Color::OFF,
        );
        assert!(c.sfc.iter().all(|v| *v == 0));
    }

    #[test]
    fn trim_and_write_atlas() {
        let mut c = Canvas::new(8, 8, 2, 2);
        c.rect(
            Rect {
                x: 1,
                y: 2,
                width: 3,
                height: 2,
            },
            Color::ON,
        );
        let mut atlas = Atlas::new(32, Format::Grayscale);
        let r = c.write_atlas(&mut atlas).unwrap();
        assert_eq!((3, 2), (r.width, r.height));
        assert_eq!(2 + 1, c.clip_left);
        assert_eq!(2 + 2, c.clip_top);
        assert_eq!(atlas.data[(r.y * 32 + r.x) as usize], 255);
    }

    #[test]
    fn empty_canvas_writes_empty_region() {
        let mut c = Canvas::new(8, 8, 2, 2);
        let mut atlas = Atlas::new(32, Format::Grayscale);
        let r = c.write_atlas(&mut atlas).unwrap();
        assert_eq!((0, 0), (r.width, r.height));
    }

    #[test]
    fn flips() {
        let mut c = Canvas::new(4, 2, 0, 0);
        c.pixel(0, 0, Color::ON);
        c.flip_horizontal();
        assert_eq!(255, at(&c, 3, 0));
        c.flip_vertical();
        assert_eq!(255, at(&c, 3, 1));
    }

    #[test]
    fn invert_all() {
        let mut c = Canvas::new(2, 2, 0, 0);
        c.pixel(0, 0, Color::ON);
        c.invert();
        assert_eq!(0, at(&c, 0, 0));
        assert_eq!(255, at(&c, 1, 1));
    }

    #[test]
    fn fill_triangle_covers_interior() {
        let mut c = Canvas::new(20, 20, 5, 5);
        c.triangle(
            Triangle {
                p0: Point { x: 0.0, y: 0.0 },
                p1: Point { x: 20.0, y: 0.0 },
                p2: Point { x: 0.0, y: 20.0 },
            },
            Color::ON,
        )
        .unwrap();
        assert_eq!(255, at(&c, 5 + 2, 5 + 2));
        assert_eq!(0, at(&c, 5 + 18, 5 + 18));
    }

    #[test]
    fn stroke_line_thickness() {
        let mut c = Canvas::new(20, 20, 5, 5);
        c.line(
            Line {
                p0: Point { x: 0.0, y: 10.0 },
                p1: Point { x: 20.0, y: 10.0 },
            },
            4.0,
            Color::ON,
        )
        .unwrap();
        // rows 8..12 inclusive of padding offset
        assert_eq!(255, at(&c, 15, 5 + 8));
        assert_eq!(255, at(&c, 15, 5 + 11));
        assert_eq!(0, at(&c, 15, 5 + 7));
        assert_eq!(0, at(&c, 15, 5 + 12));
    }

    #[test]
    fn arc_full_circle_fills() {
        let mut c = Canvas::new(20, 20, 0, 0);
        let mut p = Path::new();
        p.arc(10.0, 10.0, 8.0, 0.0, std::f64::consts::TAU);
        p.close();
        c.fill_path(&p, Color::ON).unwrap();
        assert_eq!(255, at(&c, 10, 10));
        assert_eq!(0, at(&c, 0, 0));
    }

    #[test]
    fn inner_stroke_stays_inside() {
        let mut c = Canvas::new(20, 20, 0, 0);
        let mut p = Path::new();
        p.move_to(2.0, 2.0);
        p.line_to(18.0, 2.0);
        p.line_to(18.0, 18.0);
        p.line_to(2.0, 18.0);
        p.close();
        c.inner_stroke_path(
            &p,
            StrokeOptions {
                line_width: 2.0,
                ..Default::default()
            },
            Color::ON,
        )
        .unwrap();
        assert_eq!(0, at(&c, 1, 10)); // outside
        assert_eq!(255, at(&c, 3, 10)); // inside stroke
        assert_eq!(0, at(&c, 10, 10)); // interior
    }
}
