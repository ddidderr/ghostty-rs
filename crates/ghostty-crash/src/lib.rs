//! Local crash handling (port of `ghostty/src/crash`).
//!
//! This covers setting up the process to catch crashes, writing crash
//! reports (Sentry envelopes) to disk, and introspecting them. Nothing is
//! ever sent over the network.

// Fatal-signal handling does not exist on Windows (Zig skips crash capture
// there), so parts of the shared report writer are unused.
#![cfg_attr(windows, allow(dead_code))]

pub mod context;
pub mod dir;
pub mod envelope;
#[cfg(target_os = "linux")]
mod minidump;
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
