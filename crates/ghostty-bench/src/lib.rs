//! Port of `ZIG/src/benchmark` (the `ghostty-bench` CLI) and
//! `ZIG/src/synthetic` (the `ghostty-synthetic` data generators).
//!
//! Like the Zig binary, `ghostty-bench +<action> [--opt=value ...]` runs the
//! selected benchmark exactly once and prints nothing; time the process with
//! `hyperfine`/`perf stat`. As a Rust-side convenience `--time` prints the
//! measured step duration and throughput to stderr.
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod args;
pub mod benches;
pub mod benchmark;
#[cfg(target_vendor = "apple")]
pub mod signpost;
mod simd_width;
pub mod synthetic;
