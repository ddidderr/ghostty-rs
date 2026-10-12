//! Unit helpers (ZIG/src/inspector/units.zig).

pub fn to_kibi_bytes(bytes: usize) -> usize {
    bytes / 1024
}
