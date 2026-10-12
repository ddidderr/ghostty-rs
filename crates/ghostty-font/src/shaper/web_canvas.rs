//! The web canvas shaper. Port of `font/shaper/web_canvas.zig`.
//!
//! Canvas cannot shape, so this shaper only performs grapheme clustering:
//! it can render things like skin tone emoji but not ligatures such as "=>".
//! `font_features` are not supported.
//!
//! The Zig shaper plugs into `RunIterator` through hooks; the Rust
//! `RunIterator` is tied to the FreeType `Collection`, which does not exist
//! on wasm32, so the hook methods ([`Shaper::prepare`], [`Shaper::add_codepoint`],
//! [`Shaper::finalize`]) are exposed directly. The face that reserves
//! grapheme glyph indexes is passed to [`Shaper::shape`] as a closure
//! (`face.graphemeGlyphIndex` in Zig, `run.group.faceFromIndex(run.font_index)`).

use ghostty_unicode::grapheme::{BreakState, grapheme_break};

use crate::shape::Cell;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid codepoint")]
    InvalidCodepoint,
    #[error("cell buffer too small")]
    CellBufferTooSmall,
    #[error(transparent)]
    Face(#[from] crate::face::web_canvas::Error),
}

#[derive(Debug, Clone, Copy)]
struct RunItem {
    /// The codepoint for this cell. This must be used in conjunction with
    /// cluster to find the total set of codepoints for a given cell.
    codepoint: u32,

    /// Cluster is set to the X value of the cell that this codepoint is part
    /// of. Note that a cell can have multiple codepoints with zero-width
    /// joiners (ZWJ) and such. Terminals do NOT handle full extended
    /// grapheme clustering well so it is possible a single grapheme extends
    /// multiple clusters.
    cluster: u32,
}

#[derive(Debug)]
pub struct Shaper {
    /// The shared memory used for shaping results.
    cell_buf: Vec<Cell>,

    /// The shared memory used for storing information about a run.
    run_buf: Vec<RunItem>,
}

impl Shaper {
    /// `cap` is the size of the buffer used for storing shaped results.
    /// This should be at least the number of columns in the terminal.
    pub fn new(cap: usize) -> Shaper {
        Shaper {
            cell_buf: vec![Cell::default(); cap],
            run_buf: Vec::new(),
        }
    }

    pub fn cell_buf_len(&self) -> usize {
        self.cell_buf.len()
    }

    pub fn end_frame(&self) {}

    /// Hook: reset the buffer for our current run.
    pub fn prepare(&mut self) {
        self.run_buf.clear();
    }

    /// Hook: add a codepoint of the current run.
    pub fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        self.run_buf.push(RunItem {
            codepoint: cp,
            cluster,
        });
    }

    /// Hook: finalize the run (nothing to do).
    pub fn finalize(&mut self) {}

    /// Shape the current text run. The return value is only valid until the
    /// next shape call. `grapheme_index` maps a UTF-8 grapheme cluster to a
    /// glyph index (see `Face::grapheme_glyph_index`).
    pub fn shape<E>(
        &mut self,
        mut grapheme_index: impl FnMut(&[u8]) -> Result<u32, E>,
    ) -> Result<&[Cell], Error>
    where
        Error: From<E>,
    {
        let n = self.run_buf.len();
        if n > self.cell_buf.len() {
            return Err(Error::CellBufferTooSmall);
        }

        match n {
            // Special cases: if we have no codepoints (is this possible?)
            // then our result is also an empty cell run.
            0 => return Ok(&self.cell_buf[..0]),

            // If we have only 1 codepoint, then we assume that it is a
            // single grapheme and just let it through. At this point, we
            // can't have any more information to do anything else.
            1 => {
                self.cell_buf[0] = Cell {
                    x: self.run_buf[0].cluster as u16,
                    glyph_index: self.run_buf[0].codepoint,
                    ..Cell::default()
                };
                return Ok(&self.cell_buf[..1]);
            }

            _ => {}
        }

        // We know we have at least two codepoints, so we now go through each
        // and perform grapheme clustering.
        let mut break_state = BreakState::Default;
        let mut cp1 = self.run_buf[0].codepoint;

        let mut start = 0usize;
        let mut cur = 0usize;
        let mut i = 1usize;
        while i <= n {
            // We loop to n so that we can handle the end case. In the end
            // case, we always assume it is a grapheme break. This isn't
            // strictly true but its how terminals work today.
            let is_break = i == n || {
                let cp2 = self.run_buf[i].codepoint;
                let r = grapheme_break(cp1, cp2, &mut break_state);
                cp1 = cp2;
                r
            };

            // If this is NOT a grapheme break, cp2 is part of a single
            // grapheme cluster and we expect there could be more.
            if !is_break {
                i += 1;
                continue;
            }

            // This IS a grapheme break, meaning that cp2 is NOT part of cp1.
            // So we need to render the prior grapheme.
            let len = i - start;
            debug_assert!(len > 0);
            let glyph_index = if len == 1 {
                // If we have only a single codepoint then just render it as-is.
                self.run_buf[start].codepoint
            } else {
                // We must have multiple codepoints. In this case we UTF-8
                // encode the codepoints and send them to the face to reserve
                // a private glyph index.
                let mut cluster = String::new();
                for item in &self.run_buf[start..i] {
                    cluster.push(char::from_u32(item.codepoint).ok_or(Error::InvalidCodepoint)?);
                }
                grapheme_index(cluster.as_bytes()).map_err(Error::from)?
            };

            self.cell_buf[cur] = Cell {
                x: self.run_buf[start].cluster as u16,
                glyph_index,
                ..Cell::default()
            };

            start = i;
            cur += 1;
            i += 1;
        }

        Ok(&self.cell_buf[..cur])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::web_canvas::{Error as FaceError, GraphemeTable};

    fn run(codepoints: &[(u32, u32)]) -> (Vec<Cell>, GraphemeTable) {
        let mut shaper = Shaper::new(16);
        let mut table = GraphemeTable::default();
        shaper.prepare();
        for (cp, cl) in codepoints {
            shaper.add_codepoint(*cp, *cl);
        }
        shaper.finalize();
        let cells = shaper
            .shape(|c| table.glyph_index(c).map_err(Error::from))
            .unwrap()
            .to_vec();
        (cells, table)
    }

    #[test]
    fn empty_run() {
        assert_eq!(run(&[]).0.len(), 0);
    }

    #[test]
    fn single_codepoint_passes_through() {
        let (cells, _) = run(&[(0x41, 3)]);
        assert_eq!(cells.len(), 1);
        assert_eq!((cells[0].x, cells[0].glyph_index), (3, 0x41));
    }

    #[test]
    fn plain_text_is_one_cell_per_codepoint() {
        let (cells, table) = run(&[(0x61, 0), (0x62, 1), (0x63, 2)]);
        assert_eq!(
            cells
                .iter()
                .map(|c| (c.x, c.glyph_index))
                .collect::<Vec<_>>(),
            [(0, 0x61), (1, 0x62), (2, 0x63)]
        );
        assert!(!table.contains(crate::face::web_canvas::GRAPHEME_START));
    }

    #[test]
    fn combining_mark_reserves_grapheme_glyph() {
        // e + COMBINING ACUTE ACCENT, then x
        let (cells, table) = run(&[(0x65, 0), (0x301, 0), (0x78, 1)]);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].x, 0);
        assert_eq!(
            cells[0].glyph_index,
            crate::face::web_canvas::GRAPHEME_START
        );
        assert_eq!(
            table.cluster(cells[0].glyph_index),
            Some("e\u{301}".as_bytes())
        );
        assert_eq!((cells[1].x, cells[1].glyph_index), (1, 0x78));
    }

    #[test]
    fn skin_tone_emoji_is_one_cluster() {
        let (cells, table) = run(&[(0x1F44D, 0), (0x1F3FD, 0)]);
        assert_eq!(cells.len(), 1);
        assert_eq!(table.cluster(cells[0].glyph_index), Some("👍🏽".as_bytes()));
    }

    #[test]
    fn buffer_too_small() {
        let mut shaper = Shaper::new(1);
        shaper.add_codepoint(0x61, 0);
        shaper.add_codepoint(0x62, 1);
        let r = shaper.shape(|_| Ok::<u32, FaceError>(0));
        assert!(matches!(r, Err(Error::CellBufferTooSmall)));
    }
}
