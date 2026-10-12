//! The terminal inspector (ZIG/src/inspector/*).
//!
//! The inspector is a development tool to debug the terminal, similar to a
//! browser's developer tools. The UI is built with Dear ImGui
//! ([`dear_imgui_rs`]); the apprt owns the ImGui context, feeds input events
//! and renders the draw data. This crate only draws the widgets and records
//! the events (keyboard, VT stream) that the widgets display.

pub use dear_imgui_rs as imgui;

mod dock;
mod inspector;
pub mod keys;
pub mod units;
pub mod widgets;

pub use inspector::{Inspector, Shared, setup};
pub use widgets::key::Event as KeyEvent;
