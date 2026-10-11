//! Text shapers. Port of `font/shaper/`.
//!
//! `run.zig` (RunIterator over terminal rows) is deferred (F2).

pub mod cache;
pub mod feature;
pub mod harfbuzz;
mod hb;
pub mod noop;
