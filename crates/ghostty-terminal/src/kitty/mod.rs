//! Kitty graphics protocol support (`ZIG/src/terminal/kitty.zig` and
//! `ZIG/src/terminal/kitty/graphics.zig`).
//!
//! Documentation:
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol>
//!
//! Unimplemented features that are still todo (same as Ghostty):
//! - animation
//!
//! The command parser lives in `ghostty_vt::kitty::graphics_command`; the
//! types most callers need are re-exported here.

pub mod graphics_exec;
pub mod graphics_image;
pub mod graphics_render;
pub mod graphics_storage;
pub mod graphics_unicode;

pub use ghostty_vt::kitty::{color, graphics_command, key};
pub use graphics_command::{Command, Parser as CommandParser, Response};
pub use graphics_exec::{ExecTerminal, execute};
pub use graphics_image::Image;
pub use graphics_render::Placement as RenderPlacement;
pub use graphics_storage::ImageStorage;
pub use graphics_unicode as unicode;
