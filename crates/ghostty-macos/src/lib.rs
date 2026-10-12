//! Ghostty's macOS app: a Rust/AppKit port of the Swift app in `macos/Sources`.
//!
//! The Swift app is a client of `libghostty` (`include/ghostty.h`). This port
//! drives the *same* API surface in-process by calling the Rust functions of
//! `ghostty-lib` (see [`ghostty::capi`]) with the same runtime configuration
//! (wakeup, action, clipboard, close-surface callbacks), so behaviour matches
//! the Swift app.
//!
//! Layout mirrors `Sources/`:
//!
//! | Swift                         | Rust                      |
//! |-------------------------------|---------------------------|
//! | `Sources/App/macOS`           | [`app`]                   |
//! | `Sources/Ghostty`             | [`ghostty`]               |
//! | `Sources/Helpers`             | [`helpers`]               |
//! | `Sources/Features/<Area>`     | [`features`]`::<area>`    |
//!
//! Everything that touches AppKit is `cfg(target_vendor = "apple")`; on other
//! targets the crate only contains the platform independent logic (config
//! value parsing, shell quoting, menu tables, bundle metadata, ...) so it is
//! tested on Linux and the workspace stays green. The AppKit code can only be
//! compile-checked off macOS (`scripts/macos-crosscheck.sh`); see
//! `docs/rewrite/notes/macos-app-design.md`.

pub mod app;
pub mod features;
pub mod ghostty;
pub mod helpers;
