//! ghostty-core: the terminal application core (`App.zig`, `Surface.zig`,
//! `surface_mouse.zig`, `global.zig`). See `docs/rewrite/notes/ghostty-core.md`.
//!
//! The core is runtime-agnostic: it talks to the app runtime (GTK) through
//! the traits in `ghostty_apprt::runtime_api` and implements
//! [`CoreApp`](ghostty_apprt::runtime_api::CoreApp) /
//! [`CoreSurface`](ghostty_apprt::runtime_api::CoreSurface).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::assert_is_empty))]
// Faithful port of long Zig functions with many Result-returning callbacks.
#![allow(
    clippy::nonminimal_bool,
    clippy::question_mark,
    clippy::unnecessary_wraps,
    clippy::struct_field_names,
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::collapsible_if,
    clippy::assigning_clones,
    clippy::items_after_statements,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::wildcard_imports,
    clippy::manual_let_else
)]

pub mod app;
pub mod global;
pub mod surface;
pub mod surface_mouse;

pub use app::App;
pub use surface::Surface;
