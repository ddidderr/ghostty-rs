//! `GhosttyResult` (ZIG/src/terminal/c/result.zig).

/// C: `GhosttyResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GhosttyResult {
    Success = 0,
    OutOfMemory = -1,
    InvalidValue = -2,
}
