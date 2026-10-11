//! VT-series parser for escape and control sequences
//! (ZIG/src/terminal/Parser.zig).
//!
//! NOTE(W1): this file is owned by work package W1. `MAX_PARAMS` and
//! `SepList` were pre-created by the orchestrator because `sgr` (W2)
//! depends on them; keep their API stable.

/// Maximum number of CSI parameters. This is a const in Ghostty so we
/// match it.
pub const MAX_PARAMS: usize = 24;

/// Bitset of length `MAX_PARAMS` marking which parameters were followed by
/// a colon (`:`) separator rather than a semicolon (`;`). Bit `i` set means
/// the separator *after* param `i` was a colon.
/// Equivalent to Zig's `std.StaticBitSet(MAX_PARAMS)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct SepList(u32);

impl SepList {
    /// An empty set (all separators are semicolons).
    pub const fn init_empty() -> Self {
        Self(0)
    }

    /// Whether bit `i` is set (separator after param `i` is a colon).
    pub fn is_set(&self, i: usize) -> bool {
        debug_assert!(i < MAX_PARAMS);
        self.0 & (1 << i) != 0
    }

    /// Set bit `i`.
    pub fn set(&mut self, i: usize) {
        debug_assert!(i < MAX_PARAMS);
        self.0 |= 1 << i;
    }

    /// Clear bit `i`.
    pub fn unset(&mut self, i: usize) {
        debug_assert!(i < MAX_PARAMS);
        self.0 &= !(1 << i);
    }

    /// Number of set bits.
    pub fn count(&self) -> usize {
        self.0.count_ones() as usize
    }

    /// Raw mask.
    pub fn mask(&self) -> u32 {
        self.0
    }
}
