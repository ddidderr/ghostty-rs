//! Helpers for wasm compilation. Port of `ZIG/src/os/wasm.zig` and
//! `ZIG/src/os/wasm/*.zig`.
//!
//! Only compiled for `wasm32` (Zig: `@compileError` otherwise). The module is
//! organised like the Zig directory:
//!
//! - [`log`]: the freestanding log sink (`os/wasm/log.zig`).
//! - [`target`]: the wasm target platform (`os/wasm/target.zig`).
//! - [`js`]: the JS interop layer that stands in for the `zig-js` package
//!   used by the Zig font code (see that module for the import ABI).

pub mod js;
pub mod log;
pub mod target;

/// True if we're in shared memory mode. If true, then the memory buffer
/// in JS will be backed by a SharedArrayBuffer and some behaviors change.
/// (Zig: `options.wasm_shared`, here the `wasm-shared` cargo feature.)
pub const SHARED_MEM: bool = cfg!(feature = "wasm-shared");

// The Zig `alloc` (`std.heap.wasm_allocator`) is the Rust global allocator on
// wasm32 (`std::alloc::System` is the wasm allocator); nothing to port.
