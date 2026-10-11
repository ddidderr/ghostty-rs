//! Shaping types shared by all shapers. Port of `font/shape.zig`.
//!
//! The shaper implementation for the Linux backend is HarfBuzz
//! ([`crate::shaper::harfbuzz::Shaper`]).

use crate::collection::Index;

pub use crate::shaper::cache::Cache;
pub use crate::shaper::feature::{DEFAULT_FEATURES, Feature, FeatureList};
pub use crate::shaper::{harfbuzz, noop};

/// Shaper implementation for our compile options (backend
/// `fontconfig_freetype` => HarfBuzz).
pub use crate::shaper::harfbuzz::Shaper;

/// A cell is a single single within a terminal that should be rendered
/// for a shaping call. Note all terminal cells may be present; only
/// cells that have a glyph that needs to be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    /// The X position of this shaper cell relative to the offset of the
    /// run. Because runs are always within a single row, it is expected
    /// that the caller can reconstruct the full position of the cell by
    /// using the known Y position of the cell and adding the X position
    /// to the run offset.
    pub x: u16,

    /// An additional offset to apply to the rendering.
    pub x_offset: i16,
    pub y_offset: i16,

    /// The glyph index for this cell. The font index to use alongside
    /// this cell is available in the text run. This glyph index is only
    /// valid for a given GroupCache and FontIndex that was used to create
    /// the runs.
    pub glyph_index: u32,
}

/// Options for shapers.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Font features to use when shaping.
    ///
    /// Note: eventually, this will move to font.Face probably as we may
    /// want to support per-face feature configuration. For now, we only
    /// support applying features globally.
    pub features: Vec<String>,
}

/// A single text run. Port of `shaper/run.zig` `TextRun`.
///
/// TODO(integration): `run.zig` (RunIterator over terminal rows) is deferred
/// to F2; the `grid: *SharedGrid` field is omitted here and should be added
/// when `SharedGrid` and the run iterator are wired in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRun {
    /// A unique hash for this run. This can be used to cache the shaping
    /// results.
    pub hash: u64,

    /// The offset in the row where this run started. This is added to the
    /// X position of the final shaped cells to get the absolute position
    /// in the row where they belong.
    pub offset: u16,

    /// The total number of cells produced by this run.
    pub cells: u16,

    /// The font index to use for the glyphs of this run.
    pub font_index: Index,
}
