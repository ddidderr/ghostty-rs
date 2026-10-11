//! Glyph drawing functions for the sprite font. Port of `font/sprite/draw/*.zig`.
//!
//! Each submodule exposes `RANGES`, a table of the codepoint ranges (inclusive)
//! it can draw. In Zig these are discovered at comptime from function names of
//! the form `draw<CP>` / `draw<MIN>_<MAX>`.

pub mod block;
pub mod box_;
pub mod braille;
pub mod branch;
pub mod common;
pub mod geometric_shapes;
pub mod powerline;
pub mod special;
pub mod symbols_for_legacy_computing;
pub mod symbols_for_legacy_computing_supplement;
