//! HarfBuzz shaper. Port of `font/shaper/harfbuzz.zig`.
//!
//! The Zig shaper is driven by `RunIterator` (shaper/run.rs)
//! through the hooks `prepare` / `addCodepoint` / `finalize`; those hooks are
//! exposed here as [`Shaper::prepare`], [`Shaper::add_codepoint`] and
//! [`Shaper::finalize`]. [`Shaper::shape`] takes the face of the run's font
//! (or `None` for the special sprite font, whose codepoint == glyph index)
//! instead of a `TextRun` + `SharedGrid`; the caller is responsible for
//! holding whatever lock protects the face (FreeType faces are not thread
//! safe). [`Shaper::shape_text`] is a convenience that does the whole
//! prepare/add/finalize/shape sequence.

use super::hb;
use crate::{
    face::freetype::Face,
    shape::{self, Cell, DEFAULT_FEATURES, FeatureList, TextRun},
    shared_grid::SharedGrid,
};

#[derive(Debug, thiserror::Error)]
pub enum ShapeError {
    #[error("failed to create harfbuzz buffer")]
    BufferCreate,
    #[error("harfbuzz failed")]
    HarfbuzzFailed,
}

/// Errors from [`Shaper::shape_run`].
#[derive(Debug, thiserror::Error)]
pub enum ShapeRunError {
    #[error(transparent)]
    GetFace(#[from] crate::collection::GetFaceError),
    #[error(transparent)]
    Shape(#[from] ShapeError),
}

#[derive(Debug, Clone, Copy)]
struct Codepoint {
    cluster: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct RunOffset {
    cluster: u32,
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, Default)]
struct CellOffset {
    cluster: u32,
    x: i32,
}

/// Shaper that uses Harfbuzz.
pub struct Shaper {
    /// The buffer used for text shaping. We reuse it across multiple shaping
    /// calls to prevent allocations.
    hb_buf: hb::Buffer,

    /// The shared memory used for shaping results.
    cell_buf: Vec<Cell>,

    /// The features to use for shaping.
    hb_feats: Vec<hb::Feature>,

    /// The codepoints added to the buffer before shaping. We need to keep
    /// these separately because after shaping, HarfBuzz replaces codepoints
    /// with glyph indices in the buffer.
    codepoints: Vec<Codepoint>,
}

impl Shaper {
    pub fn new(opts: &shape::Options) -> Result<Shaper, ShapeError> {
        // Parse all the features we want to use.
        let mut feature_list = FeatureList::default();
        feature_list.features.extend_from_slice(&DEFAULT_FEATURES);
        for feature_str in &opts.features {
            feature_list.append_from_string(feature_str);
        }

        let hb_feats = feature_list
            .features
            .iter()
            .map(|f| hb::global_feature(f.tag, f.value))
            .collect();

        Ok(Shaper {
            hb_buf: hb::Buffer::new().ok_or(ShapeError::BufferCreate)?,
            cell_buf: Vec::new(),
            hb_feats,
            codepoints: Vec::new(),
        })
    }

    pub fn end_frame(&self) {}

    /// Number of codepoints currently in the HarfBuzz buffer (before
    /// shaping) / glyphs (after shaping).
    pub fn hb_buf_len(&self) -> usize {
        self.hb_buf.len()
    }

    // ---- RunIterator hooks ----

    /// Reset the buffer for the current run.
    pub fn prepare(&mut self) {
        self.hb_buf.reset();
        self.hb_buf.set_content_type_unicode();

        // We set the cluster level to `characters` to give us the most
        // granularity, matching the CoreText shaper, and allowing us
        // to use our same ligature detection heuristics.
        self.hb_buf.set_cluster_level_characters();

        self.codepoints.clear();

        // We don't support RTL text because RTL in terminals is messy.
        // Its something we want to improve. For now, we force LTR because
        // our renderers assume a strictly increasing X value.
        self.hb_buf.set_direction_ltr();
    }

    pub fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        // We pass the index into codepoints as the cluster value to HarfBuzz.
        // After shaping, we use info.cluster to get back the index, which
        // lets us look up the original cluster value from codepoints.
        let index = self.codepoints.len() as u32;
        self.hb_buf.add(cp, index);
        self.codepoints.push(Codepoint { cluster });
    }

    pub fn finalize(&mut self) {
        self.hb_buf.guess_segment_properties();
    }

    /// Convenience: shape `text` as a single run where every item is a
    /// `(codepoint, cluster)` pair (cluster = cell x position).
    pub fn shape_text(
        &mut self,
        face: Option<&Face>,
        text: &[(u32, u32)],
    ) -> Result<&[Cell], ShapeError> {
        self.prepare();
        for &(cp, cluster) in text {
            self.add_codepoint(cp, cluster);
        }
        self.finalize();
        self.shape(face)
    }

    /// Shape the given text run (`shape(run)` in Zig). The run must be the
    /// immediately previous run that was iterated since it shares state with
    /// the shaper. The face is looked up in `grid` under its exclusive lock
    /// (FreeType faces are not thread safe).
    ///
    /// The return value is only valid until the next shape call is called.
    pub fn shape_run(
        &mut self,
        grid: &SharedGrid,
        run: &TextRun,
    ) -> Result<&[Cell], ShapeRunError> {
        // We only do shaping if the font is not a special-case. For
        // special-case fonts, the codepoint == glyph_index so we don't need
        // to run any shaping.
        if run.font_index.special().is_none() {
            let mut guard = grid.write();
            let face = guard.resolver.collection.get_face(run.font_index)?;
            return Ok(self.shape(Some(face))?);
        }
        Ok(self.shape(None)?)
    }

    /// Shape the current run (as built through the hooks). `face` is the face
    /// for the run's font index, or `None` if the font is a special-case
    /// font (sprite), for which codepoint == glyph_index.
    ///
    /// The return value is only valid until the next shape call is called.
    pub fn shape(&mut self, face: Option<&Face>) -> Result<&[Cell], ShapeError> {
        // We only do shaping if the font is not a special-case. For special-case
        // fonts, the codepoint == glyph_index so we don't need to run any shaping.
        if let Some(face) = face {
            let i = if face.quirks_disable_default_font_features {
                // If we are disabling default font features we just offset
                // our features by the hardcoded items because always
                // add those at the beginning.
                DEFAULT_FEATURES.len()
            } else {
                0
            };

            hb::shape(face.hb_font(), &mut self.hb_buf, &self.hb_feats[i..]);
        }

        self.cell_buf.clear();

        // If our buffer is empty, we short-circuit the rest of the work
        // return nothing.
        if self.hb_buf.is_empty() {
            return Ok(&self.cell_buf);
        }
        let info = self.hb_buf.glyph_infos();
        let pos = self
            .hb_buf
            .glyph_positions()
            .ok_or(ShapeError::HarfbuzzFailed)?;

        // This is perhaps not true somewhere, but we currently assume it is true.
        // If it isn't true, I'd like to catch it and learn more.
        debug_assert_eq!(info.len(), pos.len());

        // This keeps track of the current x and y offsets (sum of advances)
        // and the furthest cluster we've seen so far (max).
        let mut run_offset = RunOffset::default();

        // This keeps track of the cell starting x and cluster.
        let mut cell_offset = CellOffset::default();

        // Convert all our info/pos to cells and set it.
        for (info_v, pos_v) in info.iter().zip(pos.iter()) {
            // info_v.cluster is the index into our codepoints array. We use it
            // to get the original cluster.
            let index = info_v.cluster as usize;
            // Our cluster is also our cell X position. If the cluster changes
            // then we need to reset our current cell offsets.
            let cluster = self.codepoints[index].cluster;
            if cell_offset.cluster != cluster {
                let is_after_glyph_from_current_or_next_clusters = cluster <= run_offset.cluster;

                let is_first_codepoint_in_cluster = if index == 0 {
                    true
                } else {
                    self.codepoints[index - 1].cluster != cluster
                };

                // We need to reset the `cell_offset` at the start of a new
                // cluster, but we do that conditionally if the codepoint
                // `is_first_codepoint_in_cluster` and the cluster is not
                // `is_after_glyph_from_current_or_next_clusters`, which is
                // a heuristic to detect ligatures and avoid positioning
                // glyphs that mark ligatures incorrectly. The idea is that
                // if the first codepoint in a cluster doesn't appear in
                // the stream, it's very likely that it combined with
                // codepoints from a previous cluster into a ligature.
                // Then, the subsequent codepoints are very likely marking
                // glyphs that are placed relative to that ligature, so if
                // we were to reset the `cell_offset` to align it with the
                // grid, the positions would be off. The
                // `!is_after_glyph_from_current_or_next_clusters` check is
                // needed in case these marking glyphs come from a later
                // cluster but are rendered first (see the Chakma and
                // Bengali tests). In that case when we get to the
                // codepoint that `is_first_codepoint_in_cluster`, but in a
                // cluster that
                // `is_after_glyph_from_current_or_next_clusters`, we don't
                // want to reset to the grid and cause the positions to be
                // off. (Note that we could go back and align the cells to
                // the grid starting from the one from the cluster that
                // rendered out of order, but that is more complicated so
                // we don't do that for now. Also, it's TBD if there are
                // exceptions to this heuristic for detecting ligatures,
                // but using the logging below seems to show it works
                // well.)
                if is_first_codepoint_in_cluster && !is_after_glyph_from_current_or_next_clusters {
                    cell_offset = CellOffset {
                        cluster,
                        x: run_offset.x,
                    };
                }
            }

            // Under both FreeType and CoreText the harfbuzz scale is
            // in 26.6 fixed point units, so we round to the nearest
            // whole value here.
            let x_offset = run_offset.x - cell_offset.x + ((pos_v.x_offset + 0b100_000) >> 6);
            let y_offset = run_offset.y + ((pos_v.y_offset + 0b100_000) >> 6);

            self.cell_buf.push(Cell {
                x: cell_offset.cluster as u16,
                x_offset: x_offset as i16,
                y_offset: y_offset as i16,
                glyph_index: info_v.codepoint,
            });

            // Add our advances to keep track of our run offsets.
            // Advances apply to the NEXT cell.
            // Under both FreeType and CoreText the harfbuzz scale is
            // in 26.6 fixed point units, so we round to the nearest
            // whole value here.
            run_offset.x += (pos_v.x_advance + 0b100_000) >> 6;
            run_offset.y += (pos_v.y_advance + 0b100_000) >> 6;
            run_offset.cluster = run_offset.cluster.max(cluster);
        }

        Ok(&self.cell_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Library,
        embedded,
        face::{DesiredSize, FontData, Options},
        shaper::{
            run::{RunIterator, RunOptions},
            test_util::{
                Term,
                TestFont,
                shape_all,
                test_shaper,
                test_shaper_with_discovered_font,
                test_shaper_with_font,
            },
        },
    };

    fn text(s: &str) -> Vec<(u32, u32)> {
        s.chars()
            .enumerate()
            .map(|(i, c)| (c as u32, i as u32))
            .collect()
    }

    fn face(data: &'static [u8]) -> Face {
        let lib = Library::init().unwrap();
        Face::init(
            &lib,
            FontData::Static(data),
            Options::new(DesiredSize::new(12.0)),
        )
        .unwrap()
    }

    #[test]
    fn shape_emoji_run_buffer_length() {
        // "shape": thumbs up, thumbs up, skin tone modifier => 3 codepoints.
        let f = face(embedded::INCONSOLATA);
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        s.prepare();
        s.add_codepoint(0x1F44D, 0);
        s.add_codepoint(0x1F44D, 1);
        s.add_codepoint(0x1F3FD, 1);
        s.finalize();
        assert_eq!(3, s.hb_buf_len());
        s.shape(Some(&f)).unwrap();
    }

    #[test]
    fn shape_ligatures_disabled_by_feature() {
        let f = face(embedded::INCONSOLATA);
        let mut s = Shaper::new(&shape::Options {
            features: vec!["-liga".to_string()],
        })
        .unwrap();
        let cells = s.shape_text(Some(&f), &text("===")).unwrap();
        assert_eq!(3, cells.len());
    }

    #[test]
    fn shape_special_font_passthrough() {
        // Special fonts (sprite) are not shaped: glyph index == codepoint.
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        let cells = s.shape_text(None, &[(0x2500, 0), (0x2502, 1)]).unwrap();
        assert_eq!(2, cells.len());
        assert_eq!(0x2500, cells[0].glyph_index);
        assert_eq!(1, cells[1].x);
        assert_eq!(0, cells[1].x_offset);
    }

    #[test]
    fn shape_empty_run() {
        let f = face(embedded::INCONSOLATA);
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        assert_eq!(0, s.shape_text(Some(&f), &[]).unwrap().len());
    }

    #[test]
    fn shape_with_empty_cells_in_between_clusters() {
        let f = face(embedded::INCONSOLATA);
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        // 'A' at cell 0, 'B' at cell 3.
        let cells = s
            .shape_text(Some(&f), &[('A' as u32, 0), ('B' as u32, 3)])
            .unwrap();
        assert_eq!(2, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(3, cells[1].x);
    }

    /// Run all runs over `row` of `t` with the default options and return
    /// the shaped runs.
    fn run_row(
        ts: &mut crate::shaper::test_util::TestShaper,
        t: &Term,
        y: u32,
    ) -> Vec<crate::shaper::test_util::Shaped> {
        let cells = t.row(y);
        shape_all(&mut ts.shaper, RunOptions::new(&ts.grid, &cells))
    }

    fn count_runs(
        ts: &mut crate::shaper::test_util::TestShaper,
        cells: &[crate::shaper::run::RunCell],
        selection: Option<[u16; 2]>,
        cursor_x: Option<usize>,
    ) -> usize {
        let mut opts = RunOptions::new(&ts.grid, cells);
        opts.selection = selection;
        opts.cursor_x = cursor_x;
        shape_all(&mut ts.shaper, opts).len()
    }

    #[test]
    fn run_iterator() {
        let mut ts = test_shaper();

        {
            let mut t = Term::new(5, 3);
            t.print("ABCD");
            let cells = t.row(0);
            let mut it = RunIterator::new(RunOptions::new(&ts.grid, &cells));
            let mut count = 0;
            while it.next(&mut ts.shaper).unwrap().is_some() {
                count += 1;
            }
            assert_eq!(1, count);
        }

        // Spaces should be part of a run
        {
            let mut t = Term::new(10, 3);
            t.print("ABCD   EFG");
            let cells = t.row(0);
            let mut it = RunIterator::new(RunOptions::new(&ts.grid, &cells));
            let mut count = 0;
            while it.next(&mut ts.shaper).unwrap().is_some() {
                count += 1;
            }
            assert_eq!(1, count);
        }

        {
            let mut t = Term::new(5, 3);
            t.print("A\u{1F603}D");
            let cells = t.row(0);
            let mut it = RunIterator::new(RunOptions::new(&ts.grid, &cells));
            let mut count = 0;
            while it.next(&mut ts.shaper).unwrap().is_some() {
                count += 1;

                // All runs should be exactly length 1
                assert_eq!(1, ts.shaper.hb_buf_len());
            }
            assert_eq!(3, count);
        }
    }

    #[test]
    fn run_iterator_empty_cells_with_background_set() {
        use ghostty_terminal::page::{Cell, ContentTag};
        use ghostty_vt::color::Rgb;

        let mut ts = test_shaper();
        let mut t = Term::new(5, 3);
        // Set red background and write A
        t.bg_rgb(255, 0, 0);
        t.print("A");

        // Get our first row
        for x in [1, 2] {
            let mut cell = Cell::default();
            cell.set_content_tag(ContentTag::BgColorRgb);
            cell.set_color_rgb(Rgb {
                r: 0xFF,
                g: 0,
                b: 0,
            });
            *t.cell_mut(x, 0) = cell;
        }

        let cells = t.row(0);
        let mut it = RunIterator::new(RunOptions::new(&ts.grid, &cells));
        {
            let run = it.next(&mut ts.shaper).unwrap().unwrap();
            assert_eq!(3, ts.shaper.hb_buf_len());
            let shaped = ts.shaper.shape_run(&ts.grid, &run).unwrap();
            assert_eq!(3, shaped.len());
        }
        assert!(it.next(&mut ts.shaper).unwrap().is_none());
    }

    #[test]
    fn shape() {
        let mut ts = test_shaper();
        // Thumbs up plain, thumbs up plain, medium skin tone
        let mut t = Term::new(10, 3);
        t.print("\u{1F44D}\u{1F44D}\u{1F3FD}");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(3, runs[0].hb_len);
    }

    #[test]
    fn shape_inconsolata_ligs() {
        let mut ts = test_shaper();

        {
            let mut t = Term::new(5, 3);
            t.print(">=");
            let runs = run_row(&mut ts, &t, 0);
            assert_eq!(1, runs.len());
            assert_eq!(2, runs[0].run.cells);
            assert_eq!(1, runs[0].cells.len());
        }

        {
            let mut t = Term::new(5, 3);
            t.print("===");
            let runs = run_row(&mut ts, &t, 0);
            assert_eq!(1, runs.len());
            assert_eq!(3, runs[0].run.cells);
            assert_eq!(1, runs[0].cells.len());
        }
    }

    #[test]
    fn shape_monaspace_ligs() {
        let mut ts = test_shaper_with_font(TestFont::MonaspaceNeon);

        let mut t = Term::new(5, 3);
        t.print("===");
        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(3, runs[0].run.cells);
        assert_eq!(1, runs[0].cells.len());
    }

    // Ghostty doesn't currently support RTL and our renderers assume
    // that cells are in strict LTR order. This means that we need to
    // force RTL text to be LTR for rendering. This test ensures that
    // we are correctly forcing RTL text to be LTR.
    #[test]
    fn shape_arabic_forced_ltr() {
        let mut ts = test_shaper_with_font(TestFont::Arabic);

        let mut t = Term::new(120, 30);
        t.print(include_str!("../../testdata/arabic.txt"));

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(25, runs[0].run.cells);
        let cells = &runs[0].cells;
        assert_eq!(25, cells.len());
        let mut x = cells[0].x;
        for cell in &cells[1..] {
            assert_eq!(x + 1, cell.x);
            x = cell.x;
        }
    }

    #[test]
    fn shape_emoji_width() {
        let mut ts = test_shaper();

        let mut t = Term::new(5, 3);
        t.print("\u{1F44D}");
        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(2, runs[0].run.cells);
        assert_eq!(1, runs[0].cells.len());
    }

    #[test]
    fn shape_emoji_width_long() {
        use ghostty_terminal::page::{Cell, ContentTag};

        let mut ts = test_shaper();

        // Make a screen and add a long emoji sequence to it.
        let mut t = Term::new(30, 3);
        {
            let pin = t
                .screen
                .pages
                .pin(ghostty_terminal::point::Point::Active(
                    ghostty_terminal::point::Coordinate { x: 0, y: 1 },
                ))
                .unwrap();
            let page = t.screen.page_mut(pin.node);
            let y = usize::from(pin.y);
            let mut cell = Cell::default();
            cell.set_content_tag(ContentTag::Codepoint);
            cell.set_codepoint(0x1F9D4); // Person with beard
            *page.get_cells_mut(y).unwrap().first_mut().unwrap() = cell;
            let graphemes = [
                0x1F3FB, // Light skin tone (Fitz 1-2)
                0x200D,  // ZWJ
                0x2642,  // Male sign
                0xFE0F,  // Emoji presentation selector
            ];
            page.set_graphemes(0, y, &graphemes).unwrap();
        }

        let runs = run_row(&mut ts, &t, 1);
        assert_eq!(1, runs.len());
        assert_eq!(4, runs[0].hb_len);
        assert_eq!(1, runs[0].cells.len());
    }

    #[test]
    fn shape_variation_selector_vs15() {
        let mut ts = test_shaper();

        // Victory sign (default text) + VS15 to force text
        let mut t = Term::new(10, 3);
        t.print("\u{270C}\u{FE0E}");
        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(1, runs[0].hb_len);
        assert_eq!(1, runs[0].cells.len());
    }

    #[test]
    fn shape_variation_selector_vs16() {
        let mut ts = test_shaper();

        // Victory sign (default text) + VS16 to force color
        let mut t = Term::new(10, 3);
        t.print("\u{270C}\u{FE0F}");
        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(1, runs[0].hb_len);
        assert_eq!(1, runs[0].cells.len());
    }

    #[test]
    fn shape_with_empty_cells_in_between() {
        let mut ts = test_shaper();

        let mut t = Term::new(30, 3);
        t.print("A");
        t.cursor_right(5);
        t.print("B");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(7, runs[0].cells.len());
    }

    #[test]
    fn shape_combining_characters() {
        let mut ts = test_shaper();

        let mut t = Term::new(30, 3);
        t.print("n\u{0308}\u{0308}a");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(4, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);
        assert_eq!(0, cells[2].x);
        assert_eq!(1, cells[3].x);
    }

    // This test exists because the string it uses causes HarfBuzz to output a
    // non-monotonic run with our cluster level set to `characters`, which we need
    // to handle by tracking the max cluster for the run.
    #[test]
    fn shape_devanagari_string() {
        // We need a font that supports devanagari for this to work, if we can't
        // find Arial Unicode MS, which is a system font on macOS, we just skip
        // the test.
        let Some(mut ts) = test_shaper_with_discovered_font("Arial Unicode MS") else {
            return;
        };

        // Disable grapheme clustering
        let mut t = Term::new(30, 3);
        t.print("अपार्टमेंट");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(8, cells.len());
        let xs: Vec<u16> = cells.iter().map(|c| c.x).collect();
        assert_eq!(vec![0, 1, 2, 4, 4, 5, 5, 6], xs);
    }

    // This test fails on Linux if you have the "Noto Sans Tai Tham" font installed
    // locally. Disabled upstream (`return error.SkipZigTest`).
    #[test]
    #[ignore = "skipped upstream: fails on Linux with Noto Sans Tai Tham installed"]
    fn shape_tai_tham_vowels_position_differs_from_advance() {
        // Note that while this test was necessary for CoreText, the old logic was
        // working for HarfBuzz. Still we keep it to ensure it has the correct
        // behavior.
        let Some(mut ts) = test_shaper_with_discovered_font("Noto Sans Tai Tham") else {
            return;
        };

        let mut t = Term::new(30, 3);
        t.print_clustered("\u{1a2F}\u{1a70}"); // ᨯ ᩰ

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(2, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);

        // The first glyph renders in the next cell. We expect the x_offset
        // to equal the cell width. However, with FreeType the cell_width is
        // computed from ASCII glyphs, and Noto Sans Tai Tham only has the
        // space character in ASCII (with a 3px advance), so the cell_width
        // metric doesn't match the actual Tai Tham glyph positioning.
        assert_eq!(7, cells[0].x_offset);
        assert_eq!(0, cells[1].x_offset);
    }

    #[test]
    fn shape_tibetan_characters() {
        // We need a font that has multiple glyphs for this codepoint to reproduce
        // the old broken behavior, and Noto Serif Tibetan is one of them. If we
        // can't find it we just skip the test.
        let Some(mut ts) = test_shaper_with_discovered_font("Noto Serif Tibetan") else {
            return;
        };

        // Enable grapheme clustering
        let mut t = Term::new(30, 3);
        t.print_clustered("\u{0f00}"); // ༀ

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(2, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);

        // The second glyph renders at the correct location
        assert!(cells[1].x_offset < 2);
    }

    // Disabled upstream (`return error.SkipZigTest`).
    #[test]
    #[ignore = "skipped upstream: fails on Linux with Noto Sans Tai Tham installed"]
    fn shape_tai_tham_letters_run_offset_y_differs_from_zero() {
        let Some(mut ts) = test_shaper_with_discovered_font("Noto Sans Tai Tham") else {
            return;
        };

        let mut t = Term::new(30, 3);
        // First grapheme cluster: HA, SAKOT. Second grapheme cluster,
        // combining with the first in a ligature: YA, U.
        t.print_clustered("\u{1a49}\u{1a60}\u{1a3f}\u{1a69}");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(3, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);
        assert_eq!(0, cells[2].x); // U from second grapheme

        // The U glyph renders at a y below zero
        assert_eq!(-3, cells[2].y_offset);
    }

    // Disabled upstream (`return error.SkipZigTest`).
    #[test]
    #[ignore = "skipped upstream (error.SkipZigTest)"]
    fn shape_javanese_ligatures() {
        let Some(mut ts) = test_shaper_with_discovered_font("Noto Sans Javanese") else {
            return;
        };

        let mut t = Term::new(30, 3);
        // NA, PANGKON; then HA, vowel sign SUKU combining in a ligature.
        t.print_clustered("\u{a9a4}\u{a9c0}\u{a9b2}\u{a9b8}");

        let cell_width = ts.grid.metrics().cell_width;
        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(3, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);
        assert_eq!(0, cells[2].x);

        // The vowel sign SUKU renders with correct x_offset
        assert!(i64::from(cells[2].x_offset) > 3 * i64::from(cell_width));
    }

    #[test]
    fn shape_chakma_vowel_sign_with_ligature_vowel_sign_renders_first() {
        // We need a font that supports Chakma for this to work, if we can't find
        // Noto Sans Chakma Regular we just skip the test.
        let Some(mut ts) = test_shaper_with_discovered_font("Noto Sans Chakma") else {
            return;
        };

        let mut t = Term::new(30, 3);
        // BAA; TAA + Virama; YYAA + Vowel Sign U (ligature with the second).
        t.print_clustered("\u{1111d}\u{11116}\u{11133}\u{11120}\u{1112c}");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(4, cells.len());
        assert_eq!(0, cells[0].x);
        // See the giant "We need to reset the `cell_offset`" comment, but here
        // we should technically have the rest of these be `x` of 1, but that
        // would require going back in the stream to adjust past cells, and
        // we don't take on that complexity.
        assert_eq!(0, cells[1].x);
        assert_eq!(0, cells[2].x);
        assert_eq!(0, cells[3].x);

        // The vowel sign U renders before the TAA:
        assert!(cells[1].x_offset < cells[2].x_offset);
    }

    #[test]
    fn shape_bengali_ligatures_with_out_of_order_vowels() {
        // Whereas this test in CoreText had everything shaping into one giant
        // ligature, HarfBuzz splits it into a few clusters. It still looks okay
        // (see #10332).
        let Some(mut ts) = test_shaper_with_discovered_font("Arial Unicode MS") else {
            return;
        };

        let mut t = Term::new(30, 3);
        // RA + AA; SSA + Virama; TTA + Virama; RA + Vowel sign E.
        t.print_clustered("\u{09b0}\u{09be}\u{09b7}\u{09cd}\u{099f}\u{09cd}\u{09b0}\u{09c7}");

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        let cells = &runs[0].cells;
        assert_eq!(8, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);
        // Whereas CoreText puts everything all into the first cell (see the
        // corresponding test), HarfBuzz splits into two clusters.
        for c in &cells[2..] {
            assert_eq!(2, c.x);
        }

        // The vowel sign E renders before the SSA:
        assert!(cells[2].x_offset < cells[3].x_offset);
    }

    #[test]
    fn shape_box_glyphs() {
        let mut ts = test_shaper();

        let mut t = Term::new(10, 3);
        t.print("\u{2500}\u{2501}"); // horiz line

        let runs = run_row(&mut ts, &t, 0);
        assert_eq!(1, runs.len());
        assert_eq!(2, runs[0].hb_len);
        let cells = &runs[0].cells;
        assert_eq!(2, cells.len());
        assert_eq!(0x2500, cells[0].glyph_index);
        assert_eq!(0, cells[0].x);
        assert_eq!(0x2501, cells[1].glyph_index);
        assert_eq!(1, cells[1].x);
    }

    #[test]
    fn shape_selection_boundary() {
        let mut ts = test_shaper();

        let mut t = Term::new(10, 3);
        t.print("a1b2c3d4e5");
        let cells = t.row(0);

        // Full line selection
        assert_eq!(1, count_runs(&mut ts, &cells, Some([0, 9]), None));
        // Offset x, goes to end of line selection
        assert_eq!(2, count_runs(&mut ts, &cells, Some([2, 9]), None));
        // Offset x, starts at beginning of line
        assert_eq!(2, count_runs(&mut ts, &cells, Some([0, 3]), None));
        // Selection only subset of line
        assert_eq!(3, count_runs(&mut ts, &cells, Some([1, 3]), None));
        // Selection only one character
        assert_eq!(3, count_runs(&mut ts, &cells, Some([1, 1]), None));
    }

    #[test]
    fn shape_cursor_boundary() {
        let mut ts = test_shaper();

        let mut t = Term::new(10, 3);
        t.print("a1b2c3d4e5");
        let cells = t.row(0);

        // No cursor is full line
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));

        // Cursor at index 0 is two runs
        assert_eq!(2, count_runs(&mut ts, &cells, None, Some(0)));
        // And without cursor splitting remains one
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));

        // Cursor at index 1 is three runs
        assert_eq!(3, count_runs(&mut ts, &cells, None, Some(1)));
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));

        // Cursor at last col is two runs
        assert_eq!(2, count_runs(&mut ts, &cells, None, Some(9)));
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));

        // The font break config disables cursor splitting.
        let mut opts = RunOptions::new(&ts.grid, &cells);
        opts.cursor_x = Some(1);
        opts.apply_break_config(ghostty_config::types_a::FontShapingBreak { cursor: false });
        assert_eq!(1, shape_all(&mut ts.shaper, opts).len());
    }

    #[test]
    fn shape_cursor_boundary_and_colored_emoji() {
        let mut ts = test_shaper();

        let mut t = Term::new(3, 10);
        t.print("\u{1F44D}\u{1F3FC}");
        let cells = t.row(0);

        // No cursor is full line
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));

        // Cursor on emoji does not split it
        assert_eq!(1, count_runs(&mut ts, &cells, None, Some(0)));
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));
        assert_eq!(1, count_runs(&mut ts, &cells, None, Some(1)));
        assert_eq!(1, count_runs(&mut ts, &cells, None, None));
    }

    #[test]
    fn shape_cell_attribute_change() {
        use ghostty_vt::sgr::Attribute;

        let mut ts = test_shaper();

        // Plain >= should shape into 1 run
        {
            let mut t = Term::new(10, 3);
            t.print(">=");
            assert_eq!(1, count_runs(&mut ts, &t.row(0), None, None));
        }

        // Bold vs regular should split
        {
            let mut t = Term::new(3, 10);
            t.print(">");
            t.sgr(Attribute::Bold);
            t.print("=");
            assert_eq!(2, count_runs(&mut ts, &t.row(0), None, None));
        }

        // Changing fg color should split
        {
            let mut t = Term::new(3, 10);
            t.fg_rgb(1, 2, 3);
            t.print(">");
            t.fg_rgb(3, 2, 1);
            t.print("=");
            assert_eq!(2, count_runs(&mut ts, &t.row(0), None, None));
        }

        // Changing bg color should not split
        {
            let mut t = Term::new(3, 10);
            t.bg_rgb(1, 2, 3);
            t.print(">");
            t.bg_rgb(3, 2, 1);
            t.print("=");
            assert_eq!(1, count_runs(&mut ts, &t.row(0), None, None));
        }

        // Same bg color should not split
        {
            let mut t = Term::new(3, 10);
            t.bg_rgb(1, 2, 3);
            t.print(">");
            t.print("=");
            assert_eq!(1, count_runs(&mut ts, &t.row(0), None, None));
        }
    }
}
