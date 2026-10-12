//! Branch Drawing Characters | U+F5D0...U+F60D
//!
//! Branch drawing character set, used for drawing git-like
//! graphs in the terminal. Originally implemented in Kitty.
//! Ref:
//! - <https://github.com/kovidgoyal/kitty/pull/7681>
//! - <https://github.com/kovidgoyal/kitty/pull/7805>
//!
//! NOTE: Kitty is GPL licensed, and its code was not referenced
//! for these characters, only the loose specification of
//! the character set in the pull request descriptions.
//!
//! Port of `sprite/draw/branch.zig`.

use std::f64::consts::PI;

use super::{
    box_,
    common::{Corner, DrawFn, Edge, Range, Shade, Thickness, hline_middle, vline_middle},
};
use crate::{
    metrics::Metrics,
    sprite::canvas::{Canvas, CapMode, Color, DrawError, Path, StrokeOptions},
};

pub const RANGES: &[Range] = &[Range {
    min: 0xf5d0,
    max: 0xf60d,
    draw: draw_f5d0_f60d as DrawFn,
}];

/// Specification of a branch drawing node, which consists of a
/// circle which is either empty or filled, and lines connecting
/// optionally between the circle and each of the 4 edges.
#[derive(Debug, Clone, Copy, Default)]
struct BranchNode {
    up: bool,
    right: bool,
    down: bool,
    left: bool,
    filled: bool,
}

pub fn draw_f5d0_f60d(
    cp: u32,
    canvas: &mut Canvas,
    _width: u32,
    _height: u32,
    metrics: &Metrics,
) -> Result<(), DrawError> {
    match cp {
        0x0f5d0 => {
            hline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5d1 => {
            vline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5d2 => {
            fading_line(metrics, canvas, Edge::Right, Thickness::Light);
        }
        0x0f5d3 => {
            fading_line(metrics, canvas, Edge::Left, Thickness::Light);
        }
        0x0f5d4 => {
            fading_line(metrics, canvas, Edge::Bottom, Thickness::Light);
        }
        0x0f5d5 => {
            fading_line(metrics, canvas, Edge::Top, Thickness::Light);
        }
        0x0f5d6 => {
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5d7 => {
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5d8 => {
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
        }
        0x0f5d9 => {
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
        }
        0x0f5da => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
        }
        0x0f5db => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5dc => {
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5dd => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
        }
        0x0f5de => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5df => {
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5e0 => {
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
            hline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5e1 => {
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
            hline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5e2 => {
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5e3 => {
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
            hline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5e4 => {
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            hline_middle(metrics, canvas, Thickness::Light);
        }
        0x0f5e5 => {
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
        }
        0x0f5e6 => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
        }
        0x0f5e7 => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5e8 => {
            hline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
        }
        0x0f5e9 => {
            hline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5ea => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5eb => {
            vline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5ec => {
            hline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tl, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Br, Thickness::Light)?;
        }
        0x0f5ed => {
            hline_middle(metrics, canvas, Thickness::Light);
            box_::arc(metrics, canvas, Corner::Tr, Thickness::Light)?;
            box_::arc(metrics, canvas, Corner::Bl, Thickness::Light)?;
        }
        0x0f5ee => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: false,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5ef => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: false,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5f0 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: false,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5f1 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: false,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5f2 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: false,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5f3 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: false,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5f4 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: false,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5f5 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: false,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5f6 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: true,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5f7 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: true,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5f8 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: false,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5f9 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: false,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5fa => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: true,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5fb => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: true,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5fc => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: true,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5fd => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: true,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f5fe => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: true,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f5ff => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: false,
                down: true,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f600 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: false,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f601 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: false,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f602 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: false,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f603 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: false,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f604 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: true,
                left: false,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f605 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: true,
                left: false,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f606 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: true,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f607 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: false,
                down: true,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f608 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: true,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f609 => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: false,
                right: true,
                down: true,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f60a => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: false,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f60b => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: false,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),
        0x0f60c => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: true,
                left: true,
                filled: true,
            },
            Thickness::Light,
        ),
        0x0f60d => branch_node(
            metrics,
            canvas,
            BranchNode {
                up: true,
                right: true,
                down: true,
                left: true,
                filled: false,
            },
            Thickness::Light,
        ),

        _ => unreachable!(),
    }
    Ok(())
}

fn branch_node(metrics: &Metrics, canvas: &mut Canvas, node: BranchNode, thickness: Thickness) {
    let thick_px = thickness.height(metrics.box_thickness);
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);
    let float_thick = f64::from(thick_px);

    // Top of horizontal strokes
    let h_top = metrics.cell_height.saturating_sub(thick_px) / 2;
    // Bottom of horizontal strokes
    let h_bottom = h_top.saturating_add(thick_px);
    // Left of vertical strokes
    let v_left = metrics.cell_width.saturating_sub(thick_px) / 2;
    // Right of vertical strokes
    let v_right = v_left.saturating_add(thick_px);

    // We calculate the center of the circle this way
    // to ensure it aligns with box drawing characters
    // since the lines are sometimes off center to
    // make sure they aren't split between pixels.
    let cx = f64::from(v_left) + float_thick / 2.0;
    let cy = f64::from(h_top) + float_thick / 2.0;
    // The radius needs to be the smallest distance from the center to an edge.
    let r = cx.min(cy).min((float_width - cx).min(float_height - cy));

    // These float->int casts shouldn't ever fail since r can never
    // be greater than cx or cy, so when subtracting it from them the
    // result can never be negative.
    if node.up {
        canvas.box_(
            v_left as i32,
            0,
            v_right as i32,
            (cy - r + float_thick / 2.0).ceil() as i32,
            Color::ON,
        );
    }
    if node.right {
        canvas.box_(
            (cx + r - float_thick / 2.0).floor() as i32,
            h_top as i32,
            metrics.cell_width as i32,
            h_bottom as i32,
            Color::ON,
        );
    }
    if node.down {
        canvas.box_(
            v_left as i32,
            (cy + r - float_thick / 2.0).floor() as i32,
            v_right as i32,
            metrics.cell_height as i32,
            Color::ON,
        );
    }
    if node.left {
        canvas.box_(
            0,
            h_top as i32,
            (cx - r + float_thick / 2.0).ceil() as i32,
            h_bottom as i32,
            Color::ON,
        );
    }

    let mut path = Path::new();
    if node.filled {
        path.arc(cx, cy, r, 0.0, PI * 2.0);
        path.close();
        let _ = canvas.fill_path(&path, Shade::ON.color());
    } else {
        path.arc(cx, cy, r - float_thick / 2.0, 0.0, PI * 2.0);
        path.close();
        let _ = canvas.stroke_path(
            &path,
            StrokeOptions {
                line_width: float_thick,
                line_cap_mode: CapMode::Butt,
            },
            Shade::ON.color(),
        );
    }
}

fn fading_line(metrics: &Metrics, canvas: &mut Canvas, to: Edge, thickness: Thickness) {
    let thick_px = thickness.height(metrics.box_thickness);
    let float_width = f64::from(metrics.cell_width);
    let float_height = f64::from(metrics.cell_height);

    // Top of horizontal strokes
    let h_top = metrics.cell_height.saturating_sub(thick_px) / 2;
    // Bottom of horizontal strokes
    let h_bottom = h_top.saturating_add(thick_px);
    // Left of vertical strokes
    let v_left = metrics.cell_width.saturating_sub(thick_px) / 2;
    // Right of vertical strokes
    let v_right = v_left.saturating_add(thick_px);

    // If we're fading to the top or left, we start with 0.0
    // and increment up as we progress, otherwise we start
    // at 255.0 and increment down (negative).
    let mut color: f64 = match to {
        Edge::Top | Edge::Left => 0.0,
        Edge::Bottom | Edge::Right => 255.0,
    };
    let inc: f64 = 255.0
        / match to {
            Edge::Top => float_height,
            Edge::Bottom => -float_height,
            Edge::Left => float_width,
            Edge::Right => -float_width,
        };

    match to {
        Edge::Top | Edge::Bottom => {
            for y in 0..metrics.cell_height {
                for x in v_left..v_right {
                    canvas.pixel(x as i32, y as i32, Color(color.round() as u8));
                }
                color += inc;
            }
        }
        Edge::Left | Edge::Right => {
            for x in 0..metrics.cell_width {
                for y in h_top..h_bottom {
                    canvas.pixel(x as i32, y as i32, Color(color.round() as u8));
                }
                color += inc;
            }
        }
    }
}
