//! ghostty-renderer: the OpenGL renderer (see docs/rewrite/ARCHITECTURE.md and
//! docs/rewrite/notes/ghostty-renderer-R1.md / -R2.md).
//!
//! * R1: the OpenGL graphics API backend and its GPU object wrappers, shaders
//!   and their std140 data layouts, shadertoy custom-shader conversion, size
//!   math and cursor style logic.
//! * R2: the generic renderer that builds GPU cells from a terminal
//!   `RenderState` ([`generic`]), cell contents ([`cell`]), row heuristics,
//!   link matching, kitty/overlay/background images ([`image`]), the debug
//!   [`overlay`], the shared render [`state`] and the renderer [`thread`].

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod backend;
pub mod cell;
pub mod cursor;
pub mod display_link;
pub mod generic;
pub mod graphics_api;
pub mod headless;
pub mod image;
pub mod link;
pub mod math;
pub mod message;
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code, clippy::struct_field_names)]
pub mod metal;
pub mod metal_layout;
pub mod opengl;
pub mod options;
pub mod overlay;
pub mod row;
pub mod shader_types;
pub mod shadertoy;
mod shadertoy_sampler;
pub mod size;
pub mod state;
pub mod thread;

pub use backend::{Backend, Health};
pub use cursor::{Style as CursorStyle, style as cursor_style};
pub use message::Message;
pub use opengl::OpenGL;
pub use size::{CellSize, Coordinate, GridSize, Padding, ScreenSize, Size};
pub use state::State;

/// The graphics API the renderer is built with (`renderer.zig`: `Renderer =
/// switch (build_config.renderer) { .metal => GenericRenderer(Metal), .opengl
/// => GenericRenderer(OpenGL) }`): Metal on Apple platforms, OpenGL elsewhere
/// (see [`Backend::default_for_target`]).
#[cfg(target_vendor = "apple")]
pub type Api = metal::Metal;
/// The graphics API the renderer is built with: OpenGL.
#[cfg(not(target_vendor = "apple"))]
pub type Api = opengl::OpenGL;

/// The renderer: the generic renderer instantiated with the graphics API
/// selected for the target ([`Api`]).
pub type Renderer = generic::Renderer<Api>;

/// The renderer's derived configuration (`Renderer.DerivedConfig`).
pub type DerivedConfig = generic::DerivedConfig;

/// The renderer thread (`renderer.Thread`).
pub use thread::Thread;
