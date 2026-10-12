# Z19: font-backend=freetype feature, DEVIANT_FIXES additions (RV16)

## Font backend `freetype` (Zig `-Dfont-backend=freetype`)
- Cargo feature `font-backend-freetype` on `ghostty-font`, forwarded by `ghostty-cli` and the root crate (`cargo build --features font-backend-freetype`). Default (fontconfig_freetype / coretext / web_canvas) unchanged. Non-Apple, non-wasm targets only (build.rs warns and ignores it elsewhere).
- `build.rs` emits `font_discovery` (fontconfig or CoreText exists; Zig `Discover != void`); `font_fontconfig` is off in this mode. `Backend::name()` added; `OPTIONS` = `Backend::Freetype`; `ghostty_font::HAS_DISCOVERY`.
- Zig behaviour matched: `Discover = void` (Rust: uninhabited stub `Discover`, never constructed); `DeferredFace` is empty (family/name "", load/has_codepoint unreachable); SharedGridSet skips the font-family search (primary fonts come from the embedded fallback only) and still adds the embedded emoji fonts; CodepointResolver has no discovery fallback and codepoint-map overrides return None; `+list-fonts` (after arg parsing) and `+show-face` (before parsing, written to stdout like Zig's writers) print the "built without a font discovery mechanism" message (no trailing newline) and exit 1. `+version` shows `.freetype`; crash options use `Backend::name()`.
- Fontconfig sys crate is still linked (target dependency), just unused.
- Tests: `cargo test -p ghostty-font --features font-backend-freetype` (198 pass), `cargo test -p ghostty-cli --features font-backend-freetype list_fonts`. shaper `test_util` now compiles without fontconfig (discovered-font tests skip). Note `--all-features` (just recipes) selects this backend workspace-wide.

## DEVIANT_FIXES.md
- Added row 33 (maximum-length NOTIFY_SOCKET: Zig `systemd.zig:116` writes `path[108]` after the `108 < len` check; Rust uses rustix `SocketAddrUnix::new`). Introduced in f8c5ce0. New regression test `notify_maximum_length_socket_path_is_rejected_safely` in `crates/ghostty-os/src/systemd.rs` (own hunk, for the orchestrator's fix-deviant commit).
- Added "Benchmark / synthetic tooling extensions" row (`--time`, `--handler=full`, `+sgr`; commit 0c4f0ee) under Other deliberate deviations.
