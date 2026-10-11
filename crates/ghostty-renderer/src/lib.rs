//! ghostty-renderer: OpenGL renderer building blocks (see
//! docs/rewrite/ARCHITECTURE.md and docs/rewrite/notes/ghostty-renderer-R1.md).
//!
//! Terminal-independent parts of `renderer/`: the OpenGL graphics API backend
//! and its GPU object wrappers, shaders and their std140 data layouts,
//! shadertoy custom-shader conversion, size math and cursor style logic.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod backend;
pub mod cursor;
pub mod math;
pub mod message;
pub mod opengl;
pub mod options;
pub mod shadertoy;
pub mod size;

pub use backend::{Backend, Health};
pub use cursor::{Style as CursorStyle, style as cursor_style};
pub use opengl::OpenGL;
pub use size::{CellSize, Coordinate, GridSize, Padding, ScreenSize, Size};
