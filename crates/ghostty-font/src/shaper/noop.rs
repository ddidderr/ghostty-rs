//! Shaper that doesn't do any shaping. Port of `font/shaper/noop.zig`.
//! Each individual codepoint is mapped directly to the detected text run
//! font's glyph index.

use crate::{
    face::Face,
    shape::{self, Cell, TextRun},
    shared_grid::SharedGrid,
};

/// Never produced (the no-op shaper can't fail to initialize); exists so the
/// API matches the other shapers (`shape::ShapeError`).
#[derive(Debug, thiserror::Error)]
pub enum ShapeError {}

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
    codepoint: u32,
    cluster: u32,
}

/// Shaper that doesn't do any shaping.
#[derive(Debug, Default)]
pub struct Shaper {
    /// The codepoints of the current run.
    codepoints: Vec<Codepoint>,

    /// The shared memory used for shaping results.
    cell_buf: Vec<Cell>,
}

impl Shaper {
    #[allow(clippy::unnecessary_wraps)]
    pub fn new(_opts: &shape::Options) -> Result<Shaper, ShapeError> {
        Ok(Shaper::default())
    }

    pub fn end_frame(&self) {}

    // ---- RunIterator hooks ----

    pub fn prepare(&mut self) {
        self.codepoints.clear();
    }

    pub fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        self.codepoints.push(Codepoint {
            codepoint: cp,
            cluster,
        });
    }

    pub fn finalize(&mut self) {}

    /// Shape the given text run (looks the face up in `grid`).
    pub fn shape_run(
        &mut self,
        grid: &SharedGrid,
        run: &TextRun,
    ) -> Result<&[Cell], ShapeRunError> {
        if run.font_index.special().is_none() {
            let mut guard = grid.write();
            let face = guard.resolver.collection.get_face(run.font_index)?;
            return Ok(self.shape(Some(face)));
        }
        Ok(self.shape(None))
    }

    /// Shape the current run. `face` is `None` for special fonts (sprite),
    /// whose codepoint == glyph index.
    ///
    /// Panics if a codepoint has no glyph in `face` (the run iterator
    /// guarantees this never happens).
    pub fn shape(&mut self, face: Option<&Face>) -> &[Cell] {
        self.cell_buf.clear();
        for entry in &self.codepoints {
            let glyph_index = match face {
                // Special fonts aren't shaped and their codepoint == glyph.
                None => entry.codepoint,
                Some(face) => face
                    .glyph_index(entry.codepoint)
                    .expect("run iterator guarantees a glyph for every codepoint"),
            };
            self.cell_buf.push(Cell {
                x: entry.cluster as u16,
                glyph_index,
                ..Cell::default()
            });
        }
        &self.cell_buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_special_passthrough() {
        let mut s = Shaper::new(&shape::Options::default()).unwrap();
        s.prepare();
        s.add_codepoint(0x41, 0);
        s.add_codepoint(0x42, 2);
        s.finalize();
        let cells = s.shape(None);
        assert_eq!(2, cells.len());
        assert_eq!(0x41, cells[0].glyph_index);
        assert_eq!(2, cells[1].x);
    }
}
