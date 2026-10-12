# Z20: RV17 P3 fixes (uncommitted)

1. FreeType-only on Apple: `ghostty-font/build.rs` honours `font-backend-freetype` on every non-wasm
   target (`coretext` cfgs off, so the embedded-emoji fallback applies, as in Zig `Discover == void`);
   `backend.rs` OPTIONS and the `harfbuzz-sys/coretext` feature (now only for the coretext features,
   because the SDK headers are needed otherwise). New test `selected_backend_matches_build_cfgs`.
   `ghostty_surface_quicklook_font` now answers only for plain `.coretext` as in Zig
   (`ghostty_font::face::quicklook_font_raw`). `scripts/macos-crosscheck.sh` takes
   `MACOS_CHECK_FEATURES`; clippy passes on both Apple targets with and without the feature.
2. `cli/list_fonts.rs` test follows `ghostty_font::options.backend` / `HAS_DISCOVERY`.
3. Inspector: 32 px (`FONT_SIZE`), FreeType through `dear-imgui-rs/freetype` on non-Windows, non-Apple,
   non-wasm targets (registered deviation elsewhere: stb_truetype). `setup` lost its scale parameter
   (Zig has none). Visual check: new scenario `scripts/visual-parity.sh inspector` (ctrl+shift+i via
   xdotool, production build vs /usr/bin/ghostty 1.3.1). The window position differs between runs, so the
   tabs + table region was compared at the best-aligned offset (95,134): 0.031 % of pixels differ
   (max channel delta 14); the list rows differ only by the hover highlight (pointer position).
4. DCS/APC descriptions follow Zig 0.15.2 `printValue(any)`; prefix is `terminal.Parser.Action`
   (was `parser.Parser.Action`), checked by running Zig 0.15.2 (`{ 36, 32 }`, `{  }` for empty slices).
5. Demo window gate: `ghostty_debug_build` cfg from `PROFILE == "debug"` (`ghostty-inspector/build.rs`).
6. Metal buffer: `metal_layout::grown_buffer_bytes` + test; DEVIANT_FIXES row 34 (commit bab49ee).
