# ghostty-bench: CodepointWidth / IsSymbol (compat-GC)

- `codepoint-width` modes (CLI as Zig: noop|wcwidth|simd|table):
  - `wcwidth`: real libc `wcwidth` via a local `extern "C"` (no libc crate), in
    a module with `#[allow(unsafe_code)]` + SAFETY comment. Zig does not call
    `setlocale`, so neither do we (C locale). Non-unix: warning no-op like Zig on Windows.
  - `simd`: new `src/simd_width.rs`, scalar port of `simd/codepoint_width.cpp`
    (same range tables and precedence; vector lanes replaced by a sequential scan).
  - `table`: generated two-stage table with the `<= 0xFF => 1` shortcut.
- `is-symbol` modes: `uucode` (default) = `is_symbol_computed`, computed from
  private-use + block ranges (`computeIsSymbol`); `table` = generated symbols table.
  Test verifies equality with the table over all codepoints.
- Cargo.toml: `[lints]` no longer `workspace = true` (workspace `forbid` cannot be
  locally allowed); `unsafe_code = "deny"` and the clippy set is copied. Keep in sync.
- Deviation: Zig's `simd` falls back to uucode when built without SIMD; there is no
  separate uucode width mode in the CLI (none in Zig), so none added.
- Tests: all modes run on a sample file; computed vs table symbols; simd vs table
  samples; wcwidth ASCII. Zig `test CodepointWidth/IsSymbol` (run once, no data) covered by the run tests.
