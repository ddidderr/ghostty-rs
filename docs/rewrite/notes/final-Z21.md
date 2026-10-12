# Z21 final fix pass

- RV18 #1: removed the reachable `debug_assert!(!b)` in `Terminal::print_grapheme` (Zig ReleaseFast ignores it). The panic hunt (`difftest_fuzz panics`, new Rust-only mode, debug build, ~60k cases) then found a second input-reachable defect: Devanagari conjunct at the right margin on the bottom row (margin wrap scrolls) returned `InvalidCoordinate` and left a grapheme flag without data, so the formatter panicked later. Fixed by moving the grapheme from the cell that holds it. 0 panics afterwards; Zig differential 3000 cases, 0 divergences.
- RV18 #2-#4: errno accessor per target, FreeBSD resources conventions, `ghostty_debug_build` cfg (new `crates/ghostty-os/build.rs`).
- FreeBSD check: `cargo check --target x86_64-unknown-freebsd -p ghostty-os` passes (with FREETYPE_NO_PKG_CONFIG=1); `-p ghostty-termio` is blocked by dear-imgui-sys (no FreeBSD bindings) via ghostty-inspector; the `errno_now` cfg block was compiled alone against libc for FreeBSD and Linux.
- Docs: DEVIANT_FIXES.md entry 10 rewritten, entries 35 (PTY hangup drain, 6629fb0) and 36 (canvas BGRA, d89691b), dev resources fallback row (967e3a1); new KNOWN_DIFFERENCES.md.
- Side effect: the in-workspace resources fallback is now dev-profile only.
