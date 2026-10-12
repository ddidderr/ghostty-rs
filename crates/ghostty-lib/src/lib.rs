//! libghostty: the embedded C API of Ghostty (`include/ghostty.h`), ported
//! from `ZIG/src/main_c.zig`, `ZIG/src/apprt/embedded.zig`,
//! `ZIG/src/config/CApi.zig` + `c_get.zig`, the C parts of
//! `ZIG/src/apprt/action.zig` and `ZIG/src/benchmark/CApi.zig`.
//!
//! The header is shipped unchanged in `include/`. The exported symbols and
//! the struct / enum layouts of [`types`] match it exactly (verified by
//! `tests/abi.rs`).
//!
//! Platform gating follows Zig: on Apple targets surfaces render with the
//! Metal renderer into the host's `NSView` / `UIView`; elsewhere the
//! platform independent API (config, app, CLI, key translation, ...) works,
//! but `ghostty_surface_new` fails with `UnsupportedPlatform` (Zig's
//! `Platform.init`).
//!
//! `unsafe` is denied crate wide and allowed (with `SAFETY:` comments) only
//! in the FFI modules.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::undocumented_unsafe_blocks))]

pub mod action_c;
pub mod benchmark;
pub mod config_capi;
pub mod embedded;
pub mod inspector;
#[cfg(target_vendor = "apple")]
pub mod inspector_metal;
pub mod main_c;
pub mod types;
