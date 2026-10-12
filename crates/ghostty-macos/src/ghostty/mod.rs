//! `Sources/Ghostty`: the Swift side's wrappers around the libghostty C API
//! (`Ghostty.App`, `Ghostty.Config`, `Ghostty.Surface`, actions, notifications).
//!
//! These drive `ghostty-lib`'s `ghostty_*` functions directly (see [`capi`]),
//! i.e. the identical API surface that the Swift app drives through
//! `ghostty.h`.

// Platform independent (tested on every target).
pub mod action;
pub mod capi;
pub mod command;
pub mod config;
pub mod config_types;
pub mod duration_format;
pub mod error;
pub mod input;
pub mod notification;
pub mod package;
pub mod shell;
pub mod surface_configuration;

// AppKit.
#[cfg(target_vendor = "apple")]
pub mod app;
#[cfg(target_vendor = "apple")]
pub mod event;
#[cfg(target_vendor = "apple")]
pub mod fullscreen_mode;
#[cfg(target_vendor = "apple")]
pub mod ns_event;
#[cfg(target_vendor = "apple")]
pub mod surface;
#[cfg(target_vendor = "apple")]
pub mod surface_view;
