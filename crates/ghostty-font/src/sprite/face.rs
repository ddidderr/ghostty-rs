//! This implements the built-in "sprite face". This font renders
//! the built-in glyphs for the terminal, such as box drawing fonts, as well
//! as specific sprites that are part of our rendering model such as
//! text decorations (underlines).
//!
//! This isn't really a "font face" so much as it is quacks like a font
//! face with regards to how it works with font.Group. We don't use any
//! dynamic dispatch so it isn't truly an interface but the functions
//! and behaviors are close enough to a system face that it makes it easy
//! to integrate with font.Group. This is desirable so that higher level
//! processes such as GroupCache, Shaper, etc. don't need to be aware of
//! special sprite handling and just treat it like a normal font face.
//!
//! Port of `font/sprite/Face.zig`.

use std::sync::OnceLock;

use super::Sprite;
use super::canvas::{Canvas, DrawError};
use super::draw::common::{DrawFn, Range};
use super::draw::{
    block, box_, braille, branch, geometric_shapes, powerline, special,
    symbols_for_legacy_computing, symbols_for_legacy_computing_supplement,
};
use crate::atlas::{self, Atlas};
use crate::glyph::Glyph;
use crate::metrics::Metrics;

pub use crate::Presentation;
pub use crate::face::RenderOptions;

/// Errors from [`Face::render_glyph`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Draw(#[from] DrawError),
    #[error(transparent)]
    Atlas(#[from] atlas::Error),
}

/// All codepoint ranges drawn by the draw modules, sorted ascending by `min`.
/// Panics (at first use) if any ranges overlap, mirroring Zig's
/// compile error.
pub fn ranges() -> &'static [Range] {
    static RANGES: OnceLock<Vec<Range>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let mut r: Vec<Range> = [
            block::RANGES,
            box_::RANGES,
            braille::RANGES,
            branch::RANGES,
            geometric_shapes::RANGES,
            powerline::RANGES,
            symbols_for_legacy_computing::RANGES,
            symbols_for_legacy_computing_supplement::RANGES,
        ]
        .concat();

        // Sort ranges in ascending order
        r.sort_by_key(|r| r.min);

        // Ensure there's no overlapping ranges
        let mut i = 0;
        for (k, n) in r.iter().enumerate() {
            assert!(
                n.min > i || k == 0,
                "Codepoint range {:X}..{:X} overlaps previous range ending at {:X}",
                n.min,
                n.max,
                i
            );
            i = n.max;
        }

        r
    })
}

fn get_draw_fn(cp: u32) -> Option<DrawFn> {
    // For special sprites (cursors, underlines, etc.) all sprites are drawn
    // by functions from `special` that share the name of the enum field.
    if cp >= Sprite::START {
        return Sprite::from_u32(cp).map(|s| match s {
            Sprite::Underline => special::underline as DrawFn,
            Sprite::UnderlineDouble => special::underline_double,
            Sprite::UnderlineDotted => special::underline_dotted,
            Sprite::UnderlineDashed => special::underline_dashed,
            Sprite::UnderlineCurly => special::underline_curly,
            Sprite::Strikethrough => special::strikethrough,
            Sprite::Overline => special::overline,
            Sprite::CursorRect => special::cursor_rect,
            Sprite::CursorHollowRect => special::cursor_hollow_rect,
            Sprite::CursorBar => special::cursor_bar,
            Sprite::CursorUnderline => special::cursor_underline,
        });
    }

    let r = ranges();
    // Ranges are sorted and non-overlapping: binary search.
    let idx = r.partition_point(|range| range.max < cp);
    r.get(idx)
        .filter(|range| cp >= range.min && cp <= range.max)
        .map(|range| range.draw)
}

/// The sprite face.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    /// Grid metrics for rendering sprites.
    pub metrics: Metrics,
}

impl Face {
    pub fn new(metrics: Metrics) -> Face {
        Face { metrics }
    }

    /// Returns true if the codepoint exists in our sprite font.
    pub fn has_codepoint(&self, cp: u32, _p: Option<Presentation>) -> bool {
        // We ignore presentation. No matter what presentation is
        // requested we always provide glyphs for our codepoints.
        get_draw_fn(cp).is_some()
    }

    /// Render the glyph.
    pub fn render_glyph(
        &self,
        atlas: &mut Atlas,
        cp: u32,
        opts: RenderOptions,
    ) -> Result<Glyph, RenderError> {
        if cfg!(debug_assertions) && !self.has_codepoint(cp, None) {
            log::error!("invalid codepoint cp={cp:x}");
            unreachable!("invalid sprite codepoint");
        }

        // It should be impossible for this to be null and we assert that
        // in runtime safety modes but in case it is its not worth memory
        // corruption so we return a valid, blank glyph.
        let Some(draw) = get_draw_fn(cp) else {
            return Ok(Glyph::default());
        };

        let metrics = &self.metrics;

        // We adjust our sprite width based on the cell width.
        let width = match opts.cell_width.unwrap_or(1) {
            0 | 1 => metrics.cell_width,
            w => metrics.cell_width * u32::from(w),
        };

        let height = metrics.cell_height;

        let padding_x = width / 4;
        let padding_y = height / 4;

        // Make a canvas of the desired size
        let mut canvas = Canvas::new(width, height, padding_x, padding_y);

        draw(cp, &mut canvas, width, height, metrics)?;

        // Write the drawing to the atlas
        let region = canvas.write_atlas(atlas)?;

        Ok(Glyph {
            width: region.width,
            height: region.height,
            offset_x: canvas.clip_left as i32 - padding_x as i32,
            offset_y: region.height.saturating_add(canvas.clip_bottom) as i32 - padding_y as i32,
            atlas_x: region.x,
            atlas_y: region.y,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::metrics::FaceMetrics;

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// Directory where review sheets / diffs are written (`<workspace>/target/sprite_sheets`).
    fn out_dir() -> PathBuf {
        let d = manifest_dir().join("../../target/sprite_sheets");
        std::fs::create_dir_all(&d).expect("create sprite_sheets dir");
        d
    }

    fn write_png(path: &Path, w: u32, h: u32, data: &[u8]) {
        let file = std::fs::File::create(path).expect("create png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(data).expect("png data");
    }

    fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
        let file = std::fs::File::open(path).expect("open reference png");
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .expect("png info");
        let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
        let info = reader.next_frame(&mut buf).expect("png frame");
        buf.truncate(info.buffer_size());
        // The reference files are 8-bit grayscale; if not, take the first channel.
        let ch = info.color_type.samples();
        let gray = if ch == 1 {
            buf
        } else {
            buf.chunks_exact(ch).map(|c| c[0]).collect()
        };
        (info.width, info.height, gray)
    }

    fn test_metrics(width: u32, ascent: u32, descent: u32, thickness: u32) -> Metrics {
        Metrics::calc(FaceMetrics {
            // Fudged number, not used in anything we care about here.
            px_per_em: 16.0,
            cell_width: f64::from(width),
            ascent: f64::from(ascent),
            descent: -f64::from(descent),
            line_gap: 0.0,
            underline_thickness: Some(f64::from(thickness)),
            strikethrough_thickness: Some(f64::from(thickness)),
            ..Default::default()
        })
    }

    /// Stats of the comparison of one atlas sheet against its reference.
    #[derive(Debug, Default)]
    struct Diff {
        /// number of pixels that differ at all
        differing: usize,
        /// number of pixels differing by more than `BIG`
        big: usize,
        /// number of pixels that are non-zero in either image
        ink: usize,
        max: u8,
    }

    /// Per-pixel difference above which a pixel counts as a "big" difference.
    /// z2d and tiny-skia antialias differently, so small differences are expected
    /// along diagonals and curves.
    const BIG: u8 = 96;

    fn diff_sheet(test: &[u8], reference: &[u8]) -> Diff {
        let mut d = Diff::default();
        for (t, r) in test.iter().zip(reference) {
            if *t != 0 || *r != 0 {
                d.ink += 1;
            }
            let delta = t.abs_diff(*r);
            if delta != 0 {
                d.differing += 1;
                d.max = d.max.max(delta);
            }
            if delta > BIG {
                d.big += 1;
            }
        }
        d
    }

    /// Draws all ranges in to a set of 16x16 glyph sheets and compares them
    /// with the reference files (with tolerance). Returns the failure
    /// messages (empty if all ok). Mirrors `testDrawRanges` in Face.zig.
    fn test_draw_ranges(width: u32, ascent: u32, descent: u32, thickness: u32) -> Vec<String> {
        let metrics = test_metrics(width, ascent, descent, thickness);
        let height = ascent + descent;
        let padding_x = width / 4;
        let padding_y = height / 4;

        let mut canvas = Canvas::new(width, height, padding_x, padding_y);

        // We render glyphs in batches of 256, which we copy (including padding) to
        // a 16 by 16 surface to be compared with the reference file for that range.
        let stride_x = width + 2 * padding_x;
        let stride_y = height + 2 * padding_y;
        let (aw, ah) = (stride_x * 16, stride_y * 16);
        let mut atlas = vec![0u8; (aw * ah) as usize];

        let rs = ranges();
        let mut i = rs[0].min & !0xff;
        let mut failures = Vec::new();

        let flush = |i: u32, atlas: &[u8], failures: &mut Vec<String>| {
            let name = format!(
                "U+{i:X}...U+{:X}-{width}x{height}+{thickness}.png",
                i + 0xff
            );
            write_png(&out_dir().join(format!("test-{name}")), aw, ah, atlas);

            let ref_path = manifest_dir().join("testdata").join(&name);
            if !ref_path.exists() {
                failures.push(format!("missing reference file {name}"));
                return;
            }
            let (rw, rh, reference) = read_png(&ref_path);
            if (rw, rh) != (aw, ah) {
                failures.push(format!("{name}: size {aw}x{ah} != reference {rw}x{rh}"));
                return;
            }
            let d = diff_sheet(atlas, &reference);

            // Tolerances: antialiasing differences between z2d and tiny-skia
            // are fine, but the geometry must match: very few pixels may be
            // off by a lot, relative to the amount of ink on the sheet.
            let big_ratio = d.big as f64 / d.ink.max(1) as f64;
            let ok = big_ratio <= 0.04;
            if !ok || d.differing > 0 {
                // Write a visual diff (red = reference, green = ours).
                let mut rgb = Vec::with_capacity(atlas.len() * 3);
                for (t, r) in atlas.iter().zip(&reference) {
                    if t == r {
                        rgb.extend_from_slice(&[t / 3, t / 3, t / 3]);
                    } else {
                        rgb.extend_from_slice(&[*r, *t, 0]);
                    }
                }
                let path = out_dir().join(format!("diff-{name}"));
                let file = std::fs::File::create(&path).expect("create diff png");
                let mut enc = png::Encoder::new(std::io::BufWriter::new(file), aw, ah);
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()
                    .and_then(|mut w| w.write_image_data(&rgb))
                    .expect("write diff");
            }
            if !ok {
                failures.push(format!(
                    "{name}: {} big diffs out of {} ink pixels ({:.2}%), {} differing, max delta {}",
                    d.big,
                    d.ink,
                    big_ratio * 100.0,
                    d.differing,
                    d.max
                ));
            }
        };

        for range in rs {
            for cp in range.min..=range.max {
                // If we've moved to a new batch of 256, check the
                // current one and clear the surface for the next one.
                if cp - i >= 0x100 {
                    flush(i, &atlas, &mut failures);
                    i = cp & !0xff;
                    atlas.fill(0);
                }

                (range.draw)(cp, &mut canvas, width, height, &metrics).expect("draw");
                canvas.clear_clipping_regions();

                // Composite (src) the canvas into its slot of the sheet.
                let ox = stride_x * ((cp - i) % 16);
                let oy = stride_y * ((cp - i) / 16);
                for y in 0..canvas.sfc_height {
                    let src =
                        &canvas.sfc[(y * canvas.sfc_width) as usize..][..canvas.sfc_width as usize];
                    let dst = ((oy + y) * aw + ox) as usize;
                    atlas[dst..dst + src.len()].copy_from_slice(src);
                }
                canvas.reset();
            }
        }
        flush(i, &atlas, &mut failures);

        failures
    }

    #[test]
    fn sprite_face_render_all_sprites() {
        // Renders all sprites to sheets and compares them to the reference
        // images from upstream (rendered with z2d), with a tolerance for
        // antialiasing differences.
        //
        // testDrawRanges(width, ascent, descent, thickness):
        //
        // We compare 4 different sets of metrics;
        // - even cell size / even thickness
        // - even cell size / odd thickness
        // - odd cell size / even thickness
        // - odd cell size / odd thickness
        // (Also a decreasing range of sizes.)
        let mut failures = Vec::new();
        failures.extend(test_draw_ranges(18, 30, 6, 4));
        failures.extend(test_draw_ranges(12, 20, 4, 3));
        failures.extend(test_draw_ranges(11, 19, 2, 2));
        failures.extend(test_draw_ranges(9, 15, 2, 1));
        assert!(
            failures.is_empty(),
            "sprite sheets differ from reference (see {}):\n{}",
            out_dir().display(),
            failures.join("\n")
        );
    }

    #[test]
    fn ranges_sorted_and_non_overlapping() {
        let rs = ranges();
        assert!(!rs.is_empty());
        for w in rs.windows(2) {
            assert!(w[0].min <= w[0].max);
            assert!(
                w[0].max < w[1].min,
                "{:X?} overlaps {:X?}",
                w[0].max,
                w[1].min
            );
        }
    }

    #[test]
    fn has_codepoint() {
        let face = Face::new(test_metrics(9, 15, 2, 1));
        assert!(face.has_codepoint(0x2500, None));
        assert!(face.has_codepoint(0x2588, Some(Presentation::Emoji)));
        assert!(face.has_codepoint(0x28ff, None));
        assert!(face.has_codepoint(0xe0b0, None));
        assert!(face.has_codepoint(0x1fb00, None));
        assert!(face.has_codepoint(Sprite::Underline as u32, None));
        assert!(face.has_codepoint(Sprite::CursorUnderline as u32, None));
        assert!(!face.has_codepoint(u32::from('a'), None));
        assert!(!face.has_codepoint(Sprite::START + 11, None));
    }

    /// Render every supported codepoint (and special sprite) through the
    /// full `render_glyph` path at several cell sizes / cell widths and
    /// check basic invariants. Also writes a review sheet to
    /// `target/sprite_sheets/review-<w>x<h>.png`.
    #[test]
    fn render_every_sprite_at_several_sizes() {
        for (w, asc, desc, th) in [
            (9, 15, 2, 1),
            (12, 20, 4, 3),
            (18, 30, 6, 4),
            (7, 11, 2, 1),
            (25, 40, 8, 5),
        ] {
            let metrics = test_metrics(w, asc, desc, th);
            let face = Face::new(metrics);
            let mut atlas = Atlas::new(4096, crate::atlas::Format::Grayscale);

            let mut cps: Vec<u32> = ranges().iter().flat_map(|r| r.min..=r.max).collect();
            cps.extend(Sprite::ALL.iter().map(|s| *s as u32));

            let mut inked = 0usize;
            let mut count = 0usize;
            for cp in cps {
                for cell_width in [None, Some(1), Some(2)] {
                    let g = face
                        .render_glyph(
                            &mut atlas,
                            cp,
                            RenderOptions {
                                cell_width,
                                ..RenderOptions::new(metrics)
                            },
                        )
                        .unwrap_or_else(|e| panic!("cp {cp:X} {w}x{}: {e}", asc + desc));
                    // The glyph must stay within the padded canvas.
                    let cw = metrics.cell_width * u32::from(cell_width.unwrap_or(1).max(1));
                    assert!(g.width <= cw + 2 * (cw / 4), "cp {cp:X} too wide");
                    assert!(
                        g.height <= metrics.cell_height + 2 * (metrics.cell_height / 4),
                        "cp {cp:X} too tall"
                    );
                    if g.width > 0 && g.height > 0 {
                        // Trimmed regions must contain ink in every edge row/col
                        // (that is the definition of trim).
                        let row = |y: u32| {
                            (0..g.width).any(|x| {
                                atlas.data[((g.atlas_y + y) * atlas.size + g.atlas_x + x) as usize]
                                    != 0
                            })
                        };
                        assert!(row(0) && row(g.height - 1), "cp {cp:X} not trimmed");
                        inked += 1;
                    }
                    count += 1;
                }
            }
            // Most sprites have ink; only blank braille/space-like ones don't.
            assert!(inked * 10 > count * 9, "{inked} of {count} have ink");
        }

        // A review sheet of the most common blocks.
        for (w, asc, desc, th) in [(9, 15, 2, 1), (18, 30, 6, 4)] {
            let metrics = test_metrics(w, asc, desc, th);
            let height = asc + desc;
            let (px, py) = (w / 4, height / 4);
            let (sx, sy) = (w + 2 * px, height + 2 * py);
            let cps: Vec<u32> = ranges().iter().flat_map(|r| r.min..=r.max).collect();
            let cols = 64u32;
            let rows = (cps.len() as u32).div_ceil(cols);
            let (aw, ah) = (sx * cols, sy * rows);
            let mut sheet = vec![0u8; (aw * ah) as usize];
            let mut canvas = Canvas::new(w, height, px, py);
            for (n, cp) in cps.iter().enumerate() {
                let draw = get_draw_fn(*cp).expect("draw fn");
                draw(*cp, &mut canvas, w, height, &metrics).expect("draw");
                let (ox, oy) = (sx * (n as u32 % cols), sy * (n as u32 / cols));
                for y in 0..canvas.sfc_height {
                    let src =
                        &canvas.sfc[(y * canvas.sfc_width) as usize..][..canvas.sfc_width as usize];
                    let dst = ((oy + y) * aw + ox) as usize;
                    sheet[dst..dst + src.len()].copy_from_slice(src);
                }
                canvas.reset();
            }
            write_png(
                &out_dir().join(format!("review-all-{w}x{height}.png")),
                aw,
                ah,
                &sheet,
            );
        }
    }

    #[test]
    fn render_glyph_offsets_for_cursor_bar() {
        let face = Face::new(test_metrics(9, 15, 2, 1));
        let mut atlas = Atlas::new(256, crate::atlas::Format::Grayscale);
        let g = face
            .render_glyph(
                &mut atlas,
                Sprite::CursorBar as u32,
                RenderOptions::new(face.metrics),
            )
            .unwrap();
        // 1px bar placed at x = -1 (half its thickness rounded up).
        assert_eq!(1, g.width);
        assert_eq!(17, g.height);
        assert_eq!(-1, g.offset_x);
        // offset_y = region.height + clip_bottom - padding_y = 17 + 4 - 4
        assert_eq!(17, g.offset_y);
    }
}
