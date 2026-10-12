//! `Sources/Helpers`: the generic helpers.
//!
//! SwiftUI-only helpers have no Rust/AppKit counterpart and are documented as
//! unportable in `docs/rewrite/notes/macos-app-design.md` (`Backport`,
//! `HostingWindow`, `NonDraggableHostingView`, `MetalView`, the `View` /
//! `Transferable` extensions). `Weak<T>` is `std::rc::Weak` / `objc2::rc::Weak`.
//!
//! Pure logic (string/color/frame math, counters, decisions) lives in
//! platform independent modules so it is unit-tested on every target; the
//! AppKit parts are thin `cfg(target_vendor = "apple")` wrappers around it.

#![cfg_attr(test, allow(clippy::unwrap_used))]

// Platform independent (tested on every target).
pub mod any_sort_key;
pub mod app_info;
pub mod color;
pub mod cursor_counter;
pub mod extensions;
pub mod fullscreen_logic;
pub mod geometry;
pub mod keyboard_shortcut;
pub mod last_window_position_logic;
pub mod notification_center;
pub mod published;

// AppKit.
#[cfg(target_vendor = "apple")]
pub mod codable_bridge;
#[cfg(target_vendor = "apple")]
pub mod cross_kit;
#[cfg(target_vendor = "apple")]
pub mod cursor;
#[cfg(target_vendor = "apple")]
pub mod dispatch;
#[cfg(target_vendor = "apple")]
pub mod expiring_undo_manager;
#[cfg(target_vendor = "apple")]
pub mod fullscreen;
#[cfg(target_vendor = "apple")]
pub mod keyboard_layout;
#[cfg(target_vendor = "apple")]
pub mod last_window_position;
#[cfg(target_vendor = "apple")]
pub mod objc_exception_catcher;
#[cfg(target_vendor = "apple")]
pub mod private;
#[cfg(target_vendor = "apple")]
pub mod vibrant_layer;

/// The main thread marker. Every AppKit object is created and used on the main
/// thread (the Swift app uses `@MainActor` / `DispatchQueue.main`); panics
/// elsewhere, which is a programming error.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn main_thread() -> objc2::MainThreadMarker {
    objc2::MainThreadMarker::new().expect("must be called on the main thread")
}
