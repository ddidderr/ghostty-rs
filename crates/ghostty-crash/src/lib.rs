//! Local crash handling (port of `ghostty/src/crash`).
//!
//! This covers setting up the process to catch crashes, writing crash
//! reports (Sentry envelopes) to disk, and introspecting them. Nothing is
//! ever sent over the network.

// The Windows exception filter does not use the signal-safe report writer,
// so parts of it are unused there.
#![cfg_attr(windows, allow(dead_code))]

pub mod context;
#[cfg(target_vendor = "apple")]
mod darwin;
pub mod dir;
pub mod envelope;
mod exception;
#[cfg(target_os = "macos")]
mod mach_handler;
#[cfg(target_os = "linux")]
mod minidump;
// Pure (platform independent) event-context formatting; only used by the
// Darwin signal path but unit-tested everywhere.
#[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
mod native;
pub mod sentry;
#[cfg(unix)]
mod signal;
#[cfg(windows)]
#[path = "signal_windows.rs"]
mod signal;

pub use context::{DimsSnapshot, SurfaceDims, ThreadGuard, ThreadType, current_dims};
pub use dir::{Dir, Report, ReportIterator, default_dir};
pub use envelope::Envelope;
pub use sentry::{Options, deinit, init, install};
