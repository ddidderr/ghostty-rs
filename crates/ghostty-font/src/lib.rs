//! Font engine. Currently contains the metrics, atlas, glyph and sprite-font
//! modules; the rest of the font engine (faces, shaping, collections, ...) is
//! ported later.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod atlas;
pub mod glyph;
pub mod metrics;
pub mod sprite;

pub use atlas::Atlas;
pub use glyph::Glyph;
pub use metrics::Metrics;
