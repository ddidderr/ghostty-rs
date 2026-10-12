//! C ABI of libghostty-vt (ZIG/src/lib_vt.zig, ZIG/src/terminal/c/*.zig).
//!
//! The exported symbols and struct/enum layouts match
//! `include/ghostty/vt.h` (copied unchanged from upstream) exactly. Each
//! Zig file under `terminal/c/` maps to a module of the same name; the
//! Zig `@export` list in `lib_vt.zig` is expressed here as
//! `#[unsafe(no_mangle)]` on each `extern "C"` function.
//!
//! Wasm: on `wasm32` the allocation helpers (`ghostty_wasm_alloc_*`,
//! `ghostty_wasm_free_*`, `ghostty_wasm_{alloc,free}_sgr_attribute`) are
//! exported as well; on other targets they exist as ordinary Rust functions
//! but are not exported (like upstream).

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod alloc;
pub mod color;
pub mod key_encode;
pub mod key_event;
pub mod osc;
pub mod paste;
pub mod result;
pub mod sgr;
pub mod wasm;

pub use result::GhosttyResult;
