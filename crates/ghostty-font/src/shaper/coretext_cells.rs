//! Platform-independent part of the CoreText shaper
//! (`font/shaper/coretext.zig`): building the UTF-16 string / codepoint
//! table for a run, and converting CoreText glyph runs to shaped cells.
//!
//! The CoreText calls themselves live in `shaper/coretext.rs` (Apple
//! targets); this module takes plain data so the (subtle) cluster /
//! ligature heuristics can be unit tested on every platform.

use crate::shape::Cell;

/// One entry of the codepoint table. Surrogate pairs occupy two entries so
/// that indices match the UTF-16 string handed to CoreText; the second one
/// has `codepoint == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codepoint {
    pub codepoint: u32,
    pub cluster: u32,
}

/// The state of the current run (`RunState`).
#[derive(Debug, Default)]
pub struct RunState {
    pub codepoints: Vec<Codepoint>,
    pub unichars: Vec<u16>,
}

impl RunState {
    pub fn reset(&mut self) {
        self.codepoints.clear();
        self.unichars.clear();
    }

    /// `RunIteratorHook.addCodepoint`.
    pub fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        // Build our UTF-16 string for CoreText
        let (units, len) = crate::face::coretext_math::utf16_units(cp);
        self.unichars.extend_from_slice(&units[..len]);

        // Build our reverse lookup table for codepoints to clusters
        self.codepoints.push(Codepoint {
            codepoint: cp,
            cluster,
        });

        // If the UTF-16 codepoint is a pair then we need to insert
        // a dummy entry so that the CTRunGetStringIndices() function
        // maps correctly.
        if len == 2 {
            self.codepoints.push(Codepoint {
                codepoint: 0,
                cluster,
            });
        }
    }
}

/// Special fonts aren't shaped and their codepoint == glyph so we can just
/// return the codepoints as-is.
pub fn special_cells(codepoints: &[Codepoint], out: &mut Vec<Cell>) {
    out.clear();
    for entry in codepoints {
        // We use null codepoints to pad out our list so indices match
        // the UTF-16 string we constructed for CoreText. We don't want
        // to emit these if this is a special font, since they're not
        // part of the original run.
        if entry.codepoint == 0 {
            continue;
        }

        out.push(Cell {
            x: entry.cluster as u16,
            glyph_index: entry.codepoint,
            ..Cell::default()
        });
    }
}

/// One glyph of a CoreText run (the zipped glyphs / advances / positions /
/// string indices arrays).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunGlyph {
    pub glyph: u16,
    pub advance_width: f64,
    pub x: f64,
    pub y: f64,
    /// Index into the UTF-16 string (and therefore the codepoint table).
    pub index: usize,
}

/// One CoreText glyph run of the line.
#[derive(Debug, Clone, Default)]
pub struct CtRun {
    /// True if the run status has `non_monotonic` or `right_to_left`.
    pub non_ltr: bool,
    pub glyphs: Vec<RunGlyph>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Offset {
    cluster: u32,
    x: f64,
}

/// Convert CoreText runs to cells (the loop at the end of `Shaper.shape`).
pub fn cells_from_runs(codepoints: &[Codepoint], runs: &[CtRun], out: &mut Vec<Cell>) {
    // This keeps track of the current x offset (sum of advance.width) and
    // the furthest cluster we've seen so far (max).
    let mut run_offset = Offset::default();

    // This keeps track of the cell starting x and cluster.
    let mut cell_offset = Offset::default();

    out.clear();

    // CoreText, despite our insistence with an enforced embedding level,
    // may sometimes output runs that are non-monotonic. In order to fix
    // this, we check the run status for each run and if any aren't ltr
    // we set this to true, which indicates that we must sort our buffer.
    let mut non_ltr = false;

    // CoreText may generate multiple runs even though our input to
    // CoreText is already split into runs by our own run iterator.
    // The runs as far as I can tell are always sequential to each
    // other so we can iterate over them and just append to our
    // cell buffer.
    for run in runs {
        if run.non_ltr {
            non_ltr = true;
        }

        for g in &run.glyphs {
            let index = g.index;

            // Our cluster is also our cell X position. If the cluster changes
            // then we need to reset our current cell offsets.
            let cluster = codepoints[index].cluster;
            if cell_offset.cluster != cluster {
                // We previously asserted that the new cluster is greater
                // than cell_offset.cluster, but this isn't always true.
                // See e.g. the "shape Chakma vowel sign with ligature
                // (vowel sign renders first)" test.

                let is_after_glyph_from_current_or_next_clusters = cluster <= run_offset.cluster;

                let is_first_codepoint_in_cluster = {
                    let mut i = index;
                    let mut result = true;
                    while i > 0 {
                        i -= 1;
                        let codepoint = codepoints[i];

                        // Skip surrogate pair padding
                        if codepoint.codepoint == 0 {
                            continue;
                        }
                        result = codepoint.cluster != cluster;
                        break;
                    }
                    result
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
                    cell_offset = Offset {
                        cluster,
                        x: run_offset.x,
                    };
                }
            }

            let x_offset = g.x - cell_offset.x;

            out.push(Cell {
                x: cell_offset.cluster as u16,
                x_offset: x_offset.round() as i16,
                y_offset: g.y.round() as i16,
                glyph_index: u32::from(g.glyph),
            });

            // Add our advances to keep track of our run offsets.
            // Advances apply to the NEXT cell.
            run_offset.x += g.advance_width;
            run_offset.cluster = run_offset.cluster.max(cluster);
        }
    }

    // If our buffer contains some non-ltr sections we need to sort it :/
    if non_ltr {
        // This is EXCEPTIONALLY rare. Only happens for languages with
        // complex shaping which we don't even really support properly
        // right now, so are very unlikely to be used heavily by users
        // of Ghostty.
        //
        // Zig's `std.mem.sort` is not stable but Rust's `sort_by_key` is;
        // equal keys keep their relative order which is a valid result.
        out.sort_by_key(|c| c.x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(cps: &[(u32, u32)]) -> RunState {
        let mut s = RunState::default();
        for &(cp, cl) in cps {
            s.add_codepoint(cp, cl);
        }
        s
    }

    fn glyph(glyph: u16, adv: f64, x: f64, y: f64, index: usize) -> RunGlyph {
        RunGlyph {
            glyph,
            advance_width: adv,
            x,
            y,
            index,
        }
    }

    #[test]
    fn add_codepoint_bmp() {
        let s = state(&[(0x41, 0), (0x42, 1)]);
        assert_eq!(s.unichars, vec![0x41, 0x42]);
        assert_eq!(s.codepoints.len(), 2);
    }

    #[test]
    fn add_codepoint_surrogate_pair_pads_table() {
        let s = state(&[(0x1F978, 0), (0x41, 2)]);
        assert_eq!(s.unichars, vec![0xD83E, 0xDD78, 0x41]);
        assert_eq!(
            s.codepoints,
            vec![
                Codepoint {
                    codepoint: 0x1F978,
                    cluster: 0
                },
                Codepoint {
                    codepoint: 0,
                    cluster: 0
                },
                Codepoint {
                    codepoint: 0x41,
                    cluster: 2
                },
            ]
        );
    }

    #[test]
    fn special_skips_padding() {
        let s = state(&[(0x1F978, 3), (0x41, 5)]);
        let mut out = Vec::new();
        special_cells(&s.codepoints, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].x, 3);
        assert_eq!(out[0].glyph_index, 0x1F978);
        assert_eq!(out[1].x, 5);
        assert_eq!(out[1].glyph_index, 0x41);
    }

    #[test]
    fn simple_run_one_glyph_per_cluster() {
        let s = state(&[(0x41, 0), (0x42, 1), (0x43, 2)]);
        let run = CtRun {
            non_ltr: false,
            glyphs: vec![
                glyph(10, 8.0, 0.0, 0.0, 0),
                glyph(11, 8.0, 8.0, 0.0, 1),
                glyph(12, 8.0, 16.0, 0.0, 2),
            ],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert_eq!(out.len(), 3);
        for (i, c) in out.iter().enumerate() {
            assert_eq!(c.x as usize, i);
            assert_eq!(c.x_offset, 0);
            assert_eq!(c.y_offset, 0);
            assert_eq!(c.glyph_index, 10 + i as u32);
        }
    }

    #[test]
    fn ligature_glyph_spans_clusters() {
        // "fi" shaped to one glyph (index 0) -> a single cell at cluster 0;
        // the following char starts a new cell at cluster 2.
        let s = state(&[(0x66, 0), (0x69, 1), (0x41, 2)]);
        let run = CtRun {
            non_ltr: false,
            glyphs: vec![glyph(50, 16.0, 0.0, 0.0, 0), glyph(11, 8.0, 16.0, 0.0, 2)],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].x, 0);
        assert_eq!(out[1].x, 2);
        assert_eq!(out[1].x_offset, 0);
    }

    #[test]
    fn mark_after_ligature_keeps_ligature_cell() {
        // Clusters 0,1 ligate; a mark in cluster 2 positioned relative to
        // the ligature: first codepoint of cluster 2 isn't "first" in the
        // stream (it's index 2, preceded by cluster 1) so it IS first in
        // cluster; cluster 2 > run_offset.cluster(0) so offset resets.
        let s = state(&[(0x66, 0), (0x69, 1), (0x301, 2)]);
        let run = CtRun {
            non_ltr: false,
            glyphs: vec![glyph(50, 16.0, 0.0, 0.0, 0), glyph(99, 0.0, 14.0, 3.0, 2)],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].x, 2);
        // cell offset reset to run_offset.x = 16 => x_offset = 14 - 16
        assert_eq!(out[1].x_offset, -2);
        assert_eq!(out[1].y_offset, 3);
    }

    #[test]
    fn glyph_from_earlier_cluster_does_not_reset_offset() {
        // Glyph for cluster 1 comes after glyph for cluster 2 (rendered out
        // of order): cluster 1 <= run_offset.cluster (2) => no reset, the
        // cell stays at the cluster 2 offset.
        let s = state(&[(0x41, 0), (0x42, 1), (0x43, 2)]);
        let run = CtRun {
            non_ltr: false,
            glyphs: vec![
                glyph(1, 8.0, 0.0, 0.0, 0),
                glyph(3, 8.0, 8.0, 0.0, 2),
                glyph(2, 8.0, 16.0, 0.0, 1),
            ],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert_eq!(out.len(), 3);
        // cluster 2's glyph: reset (2 > 0): offset x = 8
        assert_eq!(out[1].x, 2);
        // cluster 1 glyph after cluster 2: 1 <= 2 so no reset, stays at 2
        assert_eq!(out[2].x, 2);
        assert_eq!(out[2].x_offset, 8);
    }

    #[test]
    fn non_ltr_sorts_by_x() {
        let s = state(&[(0x41, 0), (0x42, 1)]);
        let run = CtRun {
            non_ltr: true,
            glyphs: vec![glyph(2, 8.0, 0.0, 0.0, 1), glyph(1, 8.0, 8.0, 0.0, 0)],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert!(out.windows(2).all(|w| w[0].x <= w[1].x));
    }

    #[test]
    fn surrogate_padding_skipped_when_looking_back() {
        // Emoji (pair) at cluster 0 followed by 'A' at cluster 1. The 'A'
        // glyph looks back past the padding entry to the real codepoint.
        let s = state(&[(0x1F978, 0), (0x41, 1)]);
        let run = CtRun {
            non_ltr: false,
            glyphs: vec![glyph(7, 16.0, 0.0, 0.0, 0), glyph(8, 8.0, 16.0, 0.0, 2)],
        };
        let mut out = Vec::new();
        cells_from_runs(&s.codepoints, &[run], &mut out);
        assert_eq!(out[1].x, 1);
        assert_eq!(out[1].x_offset, 0);
    }
}
