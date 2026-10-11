//! HarfBuzz shaper. Port of `font/shaper/harfbuzz.zig`.
//!
//! The Zig shaper is driven by `RunIterator` (shaper/run.zig, deferred to F2)
//! through the hooks `prepare` / `addCodepoint` / `finalize`; those hooks are
//! exposed here as [`Shaper::prepare`], [`Shaper::add_codepoint`] and
//! [`Shaper::finalize`]. [`Shaper::shape`] takes the face of the run's font
//! (or `None` for the special sprite font, whose codepoint == glyph index)
//! instead of a `TextRun` + `SharedGrid`; the caller is responsible for
//! holding whatever lock protects the face (FreeType faces are not thread
//! safe). [`Shaper::shape_text`] is a convenience that does the whole
//! prepare/add/finalize/shape sequence.

use super::hb;
use crate::face::freetype::Face;
use crate::shape::{self, Cell, DEFAULT_FEATURES, FeatureList};

#[derive(Debug, thiserror::Error)]
pub enum ShapeError {
    #[error("failed to create harfbuzz buffer")]
    BufferCreate,
    #[error("harfbuzz failed")]
    HarfbuzzFailed,
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
    use crate::face::{DesiredSize, FontData, Options};
    use crate::{Library, embedded};

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
    fn shape_inconsolata_ligs() {
        let f = face(embedded::INCONSOLATA);
        // Some of our tests rely on dlig being enabled by default.
        let mut s = Shaper::new(&shape::Options {
            features: vec!["dlig".to_string()],
        })
        .unwrap();
        let cells = s.shape_text(Some(&f), &text(">=")).unwrap();
        assert_eq!(1, cells.len());
        let cells = s.shape_text(Some(&f), &text("===")).unwrap();
        assert_eq!(1, cells.len());
    }

    #[test]
    fn shape_monaspace_ligs() {
        let f = face(embedded::MONASPACE_NEON);
        // Some of our tests rely on dlig being enabled by default.
        let mut s = Shaper::new(&shape::Options {
            features: vec!["dlig".to_string()],
        })
        .unwrap();
        let cells = s.shape_text(Some(&f), &text("===")).unwrap();
        assert_eq!(1, cells.len());
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
    fn shape_arabic_forced_ltr() {
        let f = face(embedded::ARABIC);
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        let arabic = include_str!("../../testdata/arabic.txt");
        let line = arabic.lines().next().unwrap();
        let t = text(line);
        let n = t.len();
        let cells = s.shape_text(Some(&f), &t).unwrap();
        assert_eq!(n, cells.len());
        let mut x = cells[0].x;
        for cell in &cells[1..] {
            assert_eq!(x + 1, cell.x);
            x = cell.x;
        }
    }

    #[test]
    fn shape_combining_characters() {
        let f = face(embedded::INCONSOLATA);
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        // n + two combining diaeresis in cell 0, then 'a' in cell 1.
        let t = [('n' as u32, 0), (0x0308, 0), (0x0308, 0), ('a' as u32, 1)];
        let cells = s.shape_text(Some(&f), &t).unwrap();
        assert_eq!(4, cells.len());
        assert_eq!(0, cells[0].x);
        assert_eq!(0, cells[1].x);
        assert_eq!(0, cells[2].x);
        assert_eq!(1, cells[3].x);
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
}
