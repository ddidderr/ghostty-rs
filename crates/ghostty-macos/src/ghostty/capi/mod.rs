//! The libghostty C API as seen by the app.
//!
//! The Swift app imports `ghostty.h` (module `GhosttyKit`). The Rust app links
//! `ghostty-lib` and calls the very same entry points as Rust functions: the
//! exported `extern "C"` functions keep their C names and signatures, and
//! `types` holds the `#[repr(C)]` structs/unions of the header. [`consts`] holds
//! the enumerators (`GHOSTTY_ACTION_QUIT`, ...), generated from the header by
//! `scripts/gen-macos-capi-consts.py`.
//!
//! The functions are `unsafe` (raw handles); the safe wrappers are the types
//! in `ghostty::{app, config, surface}`.

pub mod consts;

pub use libghostty::{config_capi::*, embedded::*, main_c::*, types::*};

/// Length of `s` in bytes as the `usize` the `*_len` parameters of the C API
/// take (`UInt(s.lengthOfBytes(using: .utf8))`).
#[must_use]
pub fn utf8_len(s: &str) -> usize {
    s.len()
}
