//! Port of z2d 0.10's stroke geometry (`internal/tess/stroke_plotter.zig`,
//! `Face.zig`, `Pen.zig`, `Slope.zig`, `Spline.zig`, `point_buffer.zig`,
//! `Polygon.zig` contours) for the identity transformation and no dashes,
//! which is all the sprite canvas uses.
//!
//! The result is a list of polygon edges (unscaled; the multisample
//! rasterizer scales them) to be filled with the non-zero rule.

#![allow(clippy::float_cmp)] // exact comparisons, like z2d

use std::collections::VecDeque;

pub type P = (f64, f64);

pub use super::canvas::CapMode;

/// A path node, as in z2d `path_nodes.zig`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Node {
    MoveTo(P),
    LineTo(P),
    CurveTo(P, P, P),
    ClosePath,
}

#[derive(Clone, Copy, PartialEq)]
enum JoinMode {
    Miter,
    Round,
}

#[derive(Clone, Copy)]
struct Slope {
    dx: f64,
    dy: f64,
}

fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

impl Slope {
    fn new(a: P, b: P) -> Slope {
        Slope {
            dx: b.0 - a.0,
            dy: b.1 - a.1,
        }
    }

    fn normalize(&mut self) -> f64 {
        let (rdx, rdy, mag);
        if self.dx == 0.0 {
            rdx = 0.0;
            if self.dy > 0.0 {
                mag = self.dy;
                rdy = 1.0;
            } else {
                mag = -self.dy;
                rdy = -1.0;
            }
        } else if self.dy == 0.0 {
            rdy = 0.0;
            if self.dx > 0.0 {
                mag = self.dx;
                rdx = 1.0;
            } else {
                mag = -self.dx;
                rdx = -1.0;
            }
        } else {
            mag = self.dx.hypot(self.dy);
            rdx = self.dx / mag;
            rdy = self.dy / mag;
        }
        self.dx = rdx;
        self.dy = rdy;
        mag
    }

    fn compare(a: Slope, b: Slope) -> i32 {
        let bdy = if (b.dy - a.dy).abs() > f64::EPSILON {
            b.dy
        } else {
            a.dy
        };
        let bdx = if (b.dx - a.dx).abs() > f64::EPSILON {
            b.dx
        } else {
            a.dx
        };
        let cmp = sign(a.dy * bdx - bdy * a.dx);
        if cmp != 0.0 {
            return cmp as i32;
        }
        if a.dx == 0.0 && a.dy == 0.0 && bdx == 0.0 && bdy == 0.0 {
            return 0;
        }
        if a.dx == 0.0 && a.dy == 0.0 {
            return 1;
        }
        if bdx == 0.0 && bdy == 0.0 {
            return -1;
        }
        if sign(a.dx) != sign(bdx) || sign(a.dy) != sign(bdy) {
            return if a.dx > 0.0 || (a.dx == 0.0 && a.dy > 0.0) {
                -1
            } else {
                1
            };
        }
        0
    }

    fn compare_for_miter_limit(in_slope: Slope, out_slope: Slope, miter_limit: f64) -> bool {
        let mut i = in_slope;
        i.normalize();
        let mut o = out_slope;
        o.normalize();
        let in_dot_out = i.dx * o.dx + i.dy * o.dy;
        2.0 <= miter_limit * miter_limit * (1.0 + in_dot_out)
    }
}

/// z2d `Face` for the identity CTM.
#[derive(Clone, Copy)]
struct Face {
    p1: P,
    dev_slope: Slope,
    half_width: f64,
    p0_cw: P,
    p0_ccw: P,
    p1_cw: P,
    p1_ccw: P,
}

impl Face {
    fn new(p0: P, p1: P, thickness: f64) -> Face {
        let mut dev_slope = Slope::new(p0, p1);
        dev_slope.normalize();
        let half_width = thickness / 2.0;
        let ox = -dev_slope.dy * half_width;
        let oy = dev_slope.dx * half_width;
        Face {
            p1,
            dev_slope,
            half_width,
            p0_cw: (p0.0 + ox, p0.1 + oy),
            p0_ccw: (p0.0 + -ox, p0.1 + -oy),
            p1_cw: (p1.0 + ox, p1.1 + oy),
            p1_ccw: (p1.0 + -ox, p1.1 + -oy),
        }
    }

    fn intersect(&self, out: &Face, clockwise: bool) -> P {
        let in_point = if clockwise { self.p1_ccw } else { self.p1_cw };
        let out_point = if clockwise { out.p0_ccw } else { out.p0_cw };
        let mut in_slope = self.dev_slope;
        in_slope.normalize();
        let mut out_slope = out.dev_slope;
        out_slope.normalize();

        let result_y = ((out_point.0 - in_point.0) * in_slope.dy * out_slope.dy
            - out_point.1 * out_slope.dx * in_slope.dy
            + in_point.1 * in_slope.dx * out_slope.dy)
            / (in_slope.dx * out_slope.dy - out_slope.dx * in_slope.dy);
        let result_x = if in_slope.dy.abs() >= out_slope.dy.abs() {
            (result_y - in_point.1) * in_slope.dx / in_slope.dy + in_point.0
        } else {
            (result_y - out_point.1) * out_slope.dx / out_slope.dy + out_point.0
        };
        (result_x, result_y)
    }

    fn cap(&self, out: &mut dyn FnMut(P), mode: CapMode, clockwise: bool, pen: Option<&Pen>) {
        match mode {
            CapMode::Butt => {
                if clockwise {
                    out(self.p1_ccw);
                    out(self.p1_cw);
                } else {
                    out(self.p1_cw);
                    out(self.p1_ccw);
                }
            }
            CapMode::Square => {
                let ox = self.dev_slope.dx * self.half_width;
                let oy = self.dev_slope.dy * self.half_width;
                if clockwise {
                    out(self.p1_ccw);
                    out((self.p1_ccw.0 + ox, self.p1_ccw.1 + oy));
                    out((self.p1_cw.0 + ox, self.p1_cw.1 + oy));
                    out(self.p1_cw);
                } else {
                    out(self.p1_cw);
                    out((self.p1_cw.0 + ox, self.p1_cw.1 + oy));
                    out((self.p1_ccw.0 + ox, self.p1_ccw.1 + oy));
                    out(self.p1_ccw);
                }
            }
            CapMode::Round => {
                let pen = pen.expect("round caps need a pen");
                let mut vit = pen.vertex_iterator_for(
                    self.dev_slope,
                    Slope {
                        dx: -self.dev_slope.dx,
                        dy: -self.dev_slope.dy,
                    },
                    clockwise,
                );
                out(if clockwise { self.p1_ccw } else { self.p1_cw });
                while let Some(v) = vit.next() {
                    out((self.p1.0 + v.point.0, self.p1.1 + v.point.1));
                }
                out(if clockwise { self.p1_cw } else { self.p1_ccw });
            }
        }
    }
}

struct PenVertex {
    point: P,
    slope_cw: Slope,
    slope_ccw: Slope,
}

struct Pen {
    vertices: Vec<PenVertex>,
}

struct VertexIterator<'a> {
    pen: &'a Pen,
    end: usize,
    idx: usize,
    clockwise: bool,
}

impl<'a> VertexIterator<'a> {
    fn next(&mut self) -> Option<&'a PenVertex> {
        if self.idx == self.end {
            return None;
        }
        let result = &self.pen.vertices[self.idx];
        if self.clockwise {
            self.idx += 1;
            if self.idx == self.pen.vertices.len() {
                self.idx = 0;
            }
        } else {
            if self.idx == 0 {
                self.idx = self.pen.vertices.len();
            }
            self.idx -= 1;
        }
        Some(result)
    }
}

impl Pen {
    fn new(thickness: f64, tolerance: f64) -> Pen {
        let radius = thickness / 2.0;
        // Identity CTM: the transformed major axis is the radius itself.
        let major_axis = radius;
        let num: i32 = if tolerance >= major_axis * 4.0 {
            1
        } else if tolerance >= major_axis {
            4
        } else {
            let delta = (1.0 - tolerance / major_axis).acos();
            if delta == 0.0 {
                4
            } else {
                let n = (2.0 * std::f64::consts::PI / delta).ceil() as i32;
                if n < 4 {
                    4
                } else if n % 2 != 0 {
                    n + 1
                } else {
                    n
                }
            }
        };
        let num = num.max(0) as usize;
        let mut pts = Vec::with_capacity(num);
        for i in 0..num {
            let theta = 2.0 * std::f64::consts::PI * i as f64 / num as f64;
            pts.push((radius * theta.cos(), radius * theta.sin()));
        }
        let mut vertices = Vec::with_capacity(num);
        for i in 0..num {
            let next = if i + 1 >= num { 0 } else { i + 1 };
            let prev = if i == 0 { num - 1 } else { i - 1 };
            vertices.push(PenVertex {
                point: pts[i],
                slope_cw: Slope::new(pts[prev], pts[i]),
                slope_ccw: Slope::new(pts[i], pts[next]),
            });
        }
        Pen { vertices }
    }

    fn vertex_iterator_for(&self, from: Slope, to: Slope, clockwise: bool) -> VertexIterator<'_> {
        let n = self.vertices.len() as i32;
        let v = |i: i32| &self.vertices[i as usize];
        let (start, end);
        if clockwise {
            let (mut low, mut high) = (0i32, n);
            let mut i = (low + high) >> 1;
            while high - low > 1 {
                if Slope::compare(v(i).slope_cw, from) < 0 {
                    low = i;
                } else {
                    high = i;
                }
                i = (low + high) >> 1;
            }
            if Slope::compare(v(i).slope_cw, from) < 0 {
                i += 1;
                if i == n {
                    i = 0;
                }
            }
            start = i;
            if Slope::compare(to, v(i).slope_ccw) >= 0 {
                low = i;
                high = i + n;
                i = (low + high) >> 1;
                while high - low > 1 {
                    let j = if i >= n { i - n } else { i };
                    if Slope::compare(v(j).slope_cw, to) > 0 {
                        high = i;
                    } else {
                        low = i;
                    }
                    i = (low + high) >> 1;
                }
                if i >= n {
                    i -= n;
                }
            }
            end = i;
        } else {
            let (mut low, mut high) = (0i32, n);
            let mut i = (low + high) >> 1;
            while high - low > 1 {
                if Slope::compare(from, v(i).slope_ccw) < 0 {
                    low = i;
                } else {
                    high = i;
                }
                i = (low + high) >> 1;
            }
            if Slope::compare(from, v(i).slope_ccw) < 0 {
                i += 1;
                if i == n {
                    i = 0;
                }
            }
            start = i;
            if Slope::compare(v(i).slope_cw, to) <= 0 {
                low = i;
                high = i + n;
                i = (low + high) >> 1;
                while high - low > 1 {
                    let j = if i >= n { i - n } else { i };
                    if Slope::compare(to, v(j).slope_ccw) > 0 {
                        high = i;
                    } else {
                        low = i;
                    }
                    i = (low + high) >> 1;
                }
                if i >= n {
                    i -= n;
                }
            }
            end = i;
        }
        VertexIterator {
            pen: self,
            end: end.max(0) as usize,
            idx: start.max(0) as usize,
            clockwise,
        }
    }
}

/// `PointBuffer(2, 5)`.
#[derive(Default)]
struct PointBuffer {
    items: [P; 5],
    len: usize,
}

impl PointBuffer {
    fn add(&mut self, p: P) {
        if self.len < 5 {
            self.items[self.len] = p;
            self.len += 1;
        } else {
            for idx in 2..4 {
                self.items[idx] = self.items[idx + 1];
            }
            self.items[4] = p;
        }
    }
    fn head(&self, n: usize) -> P {
        self.items[n]
    }
    fn tail(&self, n: usize) -> P {
        self.items[self.len - n]
    }
}

type Contour = VecDeque<P>;

/// Stroke options after z2d's `painter.stroke` clamping.
pub struct StrokeParams {
    pub thickness: f64,
    pub cap_mode: CapMode,
    pub miter_limit: f64,
    pub tolerance: f64,
}

struct Plotter {
    p: StrokeParams,
    join_mode: JoinMode,
    pen: Option<Pen>,
    points: PointBuffer,
    clockwise: Option<bool>,
    outer: Contour,
    inner: Contour,
    edges: Vec<(P, P)>,
}

/// Plot the stroke polygon of `nodes`; returns the polygon as a list of
/// directed edges (point pairs).
pub fn plot(nodes: &[Node], p: StrokeParams) -> Vec<(P, P)> {
    let pen = if p.cap_mode == CapMode::Round {
        Some(Pen::new(p.thickness, p.tolerance))
    } else {
        None
    };
    let mut pl = Plotter {
        p,
        join_mode: JoinMode::Miter,
        pen,
        points: PointBuffer::default(),
        clockwise: None,
        outer: Contour::new(),
        inner: Contour::new(),
        edges: Vec::new(),
    };
    for n in nodes {
        match *n {
            Node::MoveTo(pt) => {
                if pl.points.len > 0 {
                    pl.finish();
                }
                pl.points.len = 0;
                pl.points.add(pt);
            }
            Node::LineTo(pt) => pl.run_line_to(JoinMode::Miter, pt),
            Node::CurveTo(b, c, d) => pl.run_curve_to(b, c, d),
            Node::ClosePath => pl.run_close_path(),
        }
    }
    pl.finish();
    pl.edges
}

fn add_contour_edges(edges: &mut Vec<(P, P)>, c: &Contour) {
    let mut last: Option<P> = None;
    for &pt in c {
        if let Some(l) = last {
            edges.push((l, pt));
        }
        last = Some(pt);
    }
    if let (Some(first), Some(l)) = (c.front(), last) {
        edges.push((l, *first));
    }
}

impl Plotter {
    fn run_line_to(&mut self, mode: JoinMode, pt: P) {
        if self.points.len == 0 {
            return;
        }
        let cur = self.points.tail(1);
        if pt == cur {
            return;
        }
        self.points.add(pt);
        if self.points.len > 2 {
            self.join(
                mode,
                self.points.tail(3),
                self.points.tail(2),
                self.points.tail(1),
            );
        }
    }

    fn run_curve_to(&mut self, b: P, c: P, d: P) {
        if self.points.len == 0 {
            return;
        }
        let a = self.points.tail(1);
        if self.pen.is_none() {
            self.pen = Some(Pen::new(self.p.thickness, self.p.tolerance));
        }
        // z2d `Spline.decompose` with tolerance^2.
        if a == b && c == d {
            self.run_line_to(JoinMode::Round, d);
            return;
        }
        let tol = self.p.tolerance * self.p.tolerance;
        let mut k = [a, b, c, d];
        self.decompose_into(&mut k, a, tol);
        self.run_line_to(JoinMode::Round, d);
    }

    fn decompose_into(&mut self, k: &mut [P; 4], start: P, tol: f64) {
        if knots_error_sq(k) < tol {
            if k[0] != start {
                self.run_line_to(JoinMode::Round, k[0]);
            }
            return;
        }
        let mut s2 = de_casteljau(k);
        self.decompose_into(k, start, tol);
        self.decompose_into(&mut s2, start, tol);
    }

    fn run_close_path(&mut self) {
        match self.points.len {
            0 => {}
            1 => self.plot_dotted(self.points.head(0)),
            2 => self.plot_single(self.points.head(0), self.points.head(1)),
            _ => self.plot_closed_joined(
                self.points.head(0),
                self.points.head(1),
                self.points.tail(2),
                self.points.tail(1),
            ),
        }
        self.points.len = 0;
    }

    fn finish(&mut self) {
        match self.points.len {
            0 | 1 => {}
            2 => self.plot_single(self.points.head(0), self.points.head(1)),
            _ => self.plot_open_joined(
                self.points.head(0),
                self.points.head(1),
                self.points.tail(2),
                self.points.tail(1),
            ),
        }
    }

    fn plot_dotted(&mut self, point: P) {
        if self.p.cap_mode == CapMode::Round {
            let pen = self.pen.as_ref().expect("round caps need a pen");
            for v in &pen.vertices {
                self.outer
                    .push_back((point.0 + v.point.0, point.1 + v.point.1));
            }
            add_contour_edges(&mut self.edges, &self.outer);
            self.outer.clear();
            self.clockwise = None;
        }
    }

    fn plot_single(&mut self, start: P, end: P) {
        let f = Face::new(start, end, self.p.thickness);
        let rev = Face::new(end, start, self.p.thickness);
        let mut sink = |pt: P| self.outer.push_back(pt);
        rev.cap(&mut sink, self.p.cap_mode, true, self.pen.as_ref());
        f.cap(&mut sink, self.p.cap_mode, true, self.pen.as_ref());
        add_contour_edges(&mut self.edges, &self.outer);
        self.outer.clear();
        self.clockwise = None;
    }

    fn plot_open_joined(&mut self, start0: P, end0: P, start1: P, end1: P) {
        let t = self.p.thickness;
        let start_rev = Face::new(end0, start0, t);
        let end_face = Face::new(start1, end1, t);
        let clockwise = self.clockwise.unwrap_or(true);

        // Start cap: inserted before the first corner of the outer contour.
        let mut cursor = 0usize;
        let outer = &mut self.outer;
        start_rev.cap(
            &mut |pt| {
                outer.insert(cursor, pt);
                cursor += 1;
            },
            self.p.cap_mode,
            clockwise,
            self.pen.as_ref(),
        );
        // End cap: appended.
        end_face.cap(
            &mut |pt| outer.push_back(pt),
            self.p.cap_mode,
            clockwise,
            self.pen.as_ref(),
        );
        // Concatenate the inner contour onto the outer.
        let inner = std::mem::take(&mut self.inner);
        self.outer.extend(inner);
        add_contour_edges(&mut self.edges, &self.outer);
        self.outer.clear();
        self.clockwise = None;
    }

    fn plot_closed_joined(&mut self, initial0: P, initial1: P, p1: P, p2: P) {
        let mode = self.join_mode;
        if p2 == initial0 {
            self.join(mode, p1, initial0, initial1);
        } else {
            self.join(mode, p1, p2, initial0);
            self.join(mode, p2, initial0, initial1);
        }
        add_contour_edges(&mut self.edges, &self.outer);
        add_contour_edges(&mut self.edges, &self.inner);
        self.outer.clear();
        self.inner.clear();
        self.clockwise = None;
    }

    fn join(&mut self, join_mode: JoinMode, p0: P, p1: P, p2: P) {
        if p0 == p1 || p1 == p2 {
            if self.clockwise.is_none() {
                self.clockwise = Some(false);
            }
            return;
        }
        let t = self.p.thickness;
        let inf = Face::new(p0, p1, t);
        let out = Face::new(p1, p2, t);
        let join_clockwise = Slope::compare(inf.dev_slope, out.dev_slope) < 0;
        let poly_clockwise = self.clockwise.unwrap_or(join_clockwise);
        let switched = join_clockwise != poly_clockwise;

        // `outer_joiner` plots to the outer contour (append) unless the
        // direction switched, in which case it plots to the inner (prepend).
        let plot_outer = |pl: &mut Plotter, pt: P, to_outer: bool| {
            if to_outer {
                pl.outer.push_back(pt);
            } else {
                pl.inner.push_front(pt);
            }
        };

        if Slope::compare(inf.dev_slope, out.dev_slope) == 0 {
            plot_outer(
                self,
                if join_clockwise {
                    inf.p1_ccw
                } else {
                    inf.p1_cw
                },
                !switched,
            );
            plot_outer(
                self,
                if join_clockwise {
                    inf.p1_cw
                } else {
                    inf.p1_ccw
                },
                switched,
            );
            if self.clockwise.is_none() {
                self.clockwise = Some(poly_clockwise);
            }
            return;
        }

        match join_mode {
            JoinMode::Miter => {
                if Slope::compare_for_miter_limit(inf.dev_slope, out.dev_slope, self.p.miter_limit)
                {
                    let pt = inf.intersect(&out, join_clockwise);
                    plot_outer(self, pt, !switched);
                } else {
                    plot_outer(
                        self,
                        if join_clockwise {
                            inf.p1_ccw
                        } else {
                            inf.p1_cw
                        },
                        !switched,
                    );
                    plot_outer(
                        self,
                        if join_clockwise {
                            out.p0_ccw
                        } else {
                            out.p0_cw
                        },
                        !switched,
                    );
                }
            }
            JoinMode::Round => {
                plot_outer(
                    self,
                    if join_clockwise {
                        inf.p1_ccw
                    } else {
                        inf.p1_cw
                    },
                    !switched,
                );
                let pen = self.pen.take().expect("round joins need a pen");
                let mut vit = pen.vertex_iterator_for(inf.dev_slope, out.dev_slope, join_clockwise);
                while let Some(v) = vit.next() {
                    plot_outer(self, (p1.0 + v.point.0, p1.1 + v.point.1), !switched);
                }
                self.pen = Some(pen);
                plot_outer(
                    self,
                    if join_clockwise {
                        out.p0_ccw
                    } else {
                        out.p0_cw
                    },
                    !switched,
                );
            }
        }

        plot_outer(
            self,
            if join_clockwise {
                inf.p1_cw
            } else {
                inf.p1_ccw
            },
            switched,
        );
        plot_outer(self, p1, switched);
        plot_outer(
            self,
            if join_clockwise {
                out.p0_cw
            } else {
                out.p0_ccw
            },
            switched,
        );
        if self.clockwise.is_none() {
            self.clockwise = Some(poly_clockwise);
        }
    }
}

fn lerp_half(a: P, b: P) -> P {
    (a.0 + (b.0 - a.0) / 2.0, a.1 + (b.1 - a.1) / 2.0)
}

/// z2d `Spline.Knots.errorSq`.
#[allow(clippy::float_cmp)] // exact comparison, like z2d
pub fn knots_error_sq(k: &[P; 4]) -> f64 {
    let [a, b, c, d] = *k;
    let (mut bx, mut by) = (b.0 - a.0, b.1 - a.1);
    let (mut cx, mut cy) = (c.0 - a.0, c.1 - a.1);
    if a.0 != d.0 || a.1 != d.1 {
        let (dx, dy) = (d.0 - a.0, d.1 - a.1);
        let dd = dx * dx + dy * dy;
        let bd = bx * dx + by * dy;
        if bd >= dd {
            bx -= dx;
            by -= dy;
        } else {
            bx -= bd / dd * dx;
            by -= bd / dd * dy;
        }
        let cd = cx * dx + cy * dy;
        if cd >= dd {
            cx -= dx;
            cy -= dy;
        } else {
            cx -= cd / dd * dx;
            cy -= cd / dd * dy;
        }
    }
    let (be, ce) = (bx * bx + by * by, cx * cx + cy * cy);
    if be > ce { be } else { ce }
}

/// z2d `Spline.Knots.deCasteljau`: `k` becomes the first half, the second
/// half is returned.
pub fn de_casteljau(k: &mut [P; 4]) -> [P; 4] {
    let [a, b, c, d] = *k;
    let ab = lerp_half(a, b);
    let bc = lerp_half(b, c);
    let cd = lerp_half(c, d);
    let abbc = lerp_half(ab, bc);
    let bccd = lerp_half(bc, cd);
    let fin = lerp_half(abbc, bccd);
    *k = [a, ab, abbc, fin];
    [fin, bccd, cd, d]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> StrokeParams {
        StrokeParams {
            thickness: 2.0,
            cap_mode: CapMode::Butt,
            miter_limit: 10.0,
            tolerance: 0.01,
        }
    }

    // Ports of the z2d `stroke_plotter` tests.
    #[test]
    fn degenerate_move_line_then_good_line() {
        let nodes = [
            Node::MoveTo((10.0, 10.0)),
            Node::LineTo((10.0, 10.0)),
            Node::LineTo((20.0, 20.0)),
        ];
        assert_eq!(plot(&nodes, params()).len(), 4);
        let nodes = [
            Node::MoveTo((10.0, 10.0)),
            Node::LineTo((20.0, 20.0)),
            Node::LineTo((20.0, 20.0)),
        ];
        assert_eq!(plot(&nodes, params()).len(), 4);
    }

    #[test]
    fn slope_difference_below_epsilon_is_finite() {
        let nodes = [
            Node::MoveTo((2.8641015625e2, 1.43154296875e1)),
            Node::LineTo((2.822548828125e2, 1.6e1)),
            Node::LineTo((2.80990234375e2, 1.65126953125e1)),
        ];
        let edges = plot(&nodes, params());
        assert_eq!(edges.len(), 6);
        for (a, b) in edges {
            assert!(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite());
        }
    }

    #[test]
    fn round_cap_dot_and_curve_do_not_panic() {
        let mut p = params();
        p.cap_mode = CapMode::Round;
        let nodes = [Node::MoveTo((5.0, 5.0)), Node::ClosePath];
        assert!(plot(&nodes, p).len() > 4);
        let nodes = [
            Node::MoveTo((0.0, 0.0)),
            Node::CurveTo((5.0, 0.0), (10.0, 5.0), (10.0, 10.0)),
        ];
        assert!(plot(&nodes, params()).len() > 6);
    }
}
