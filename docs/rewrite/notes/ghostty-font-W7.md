# ghostty-font — W7 (Metrics, Atlas, Glyph, sprite font)

## Ported (Zig -> Rust, all under `crates/ghostty-font/src/`)
- `font/Metrics.zig` -> `metrics.rs` (`Metrics`, `FaceMetrics`, `Modifier`, `ModifierSet = HashMap<Key, Modifier>`, `Key`, `Modifier::parse/parse_cli/format_entry/apply_{u32,i32,f64}`).
- `font/Atlas.zig` -> `atlas.rs`; `font/Glyph.zig` -> `glyph.rs`.
- `font/sprite.zig` -> `sprite/mod.rs` (`Sprite` enum, re-exports).
- `font/sprite/Face.zig` -> `sprite/face.rs`; `sprite/canvas.zig` -> `sprite/canvas.rs`.
- `sprite/draw/{common,special,block,box,braille,branch,geometric_shapes,powerline,symbols_for_legacy_computing,symbols_for_legacy_computing_supplement}.zig` -> `sprite/draw/*.rs` (`box.zig` -> `box_.rs` since `box` is a keyword-ish name).
- `sprite/draw/octants.txt` is copied next to the supplement module (loaded with `include_str!`, parsed lazily).

## Design notes / deviations
- z2d replaced by `tiny-skia` (coverage rasterizer only). `Canvas` owns an alpha8 `Vec<u8>`; fills/strokes are rasterized into a scratch pixmap and composited `src_over` with the colour alpha, as z2d does for an opaque alpha8 pattern. `Canvas::inner_stroke_path` reproduces the Zig fill-mask x double-width-stroke trick. Paths are built with the local `Path` type (move_to/line_to/curve_to/close/arc); the padding translation is applied at raster time (like z2d's `staticPath` transform). z2d `Context` is not needed (only used for arc/fill/stroke) and was folded into `Path::arc` + `fill_path`/`stroke_path`.
- Stroke miter limit 10 (z2d default) and miter joins.
- Comptime `draw<MIN>_<MAX>` function discovery is replaced by explicit `pub const RANGES: &[Range]` tables in each draw module; `face::ranges()` sorts them and asserts non-overlap (equivalent of the Zig `@compileError`; covered by `ranges_sorted_and_non_overlapping`).
- Zig `assert` (debug only) -> `debug_assert!` in braille (Zig asserts only in safe builds). NB: some odd cell sizes (e.g. 14x23) trip that assert in debug builds exactly as in Zig. The workspace `release` profile has debug-assertions on, so they apply there too; `production` has them off.
- `Canvas::pixel` ignores out-of-bounds writes (z2d `putPixel` does as well). `Box<T>` in `sprite::canvas` shadows `std::boxed::Box` where imported.
- `Color` is `struct Color(pub u8)` (Zig non-exhaustive enum); `Shade` likewise.
- `Modifier::format_entry()` returns just the value text (e.g. `"24%"`); the `a = ...\n` config-entry formatting lives in ghostty-config.
- `ModifierSet` iteration order in `Metrics::apply` is the `Key` declaration order (Zig: hash-map order, unspecified).
- Cargo.toml: removed deps on `ghostty-config`, `ghostty-unicode`, `ghostty-terminal` (not used yet); added `tiny-skia`, `png`, `thiserror`, `log`.
- Atlas: `Atlas::new` is infallible (no allocator); `grow` is infallible; `modified`/`resized` are `AtomicUsize`.

## TODO(integration)
- `sprite::face::Presentation` and `sprite::face::RenderOptions` (only `cell_width`) are local stand-ins for `font.Presentation` / `font.face.RenderOptions`; replace when `face.rs` / `main.rs` of the font engine are ported.
- `ghostty-config` should use `Modifier::parse_cli` for the `adjust-*` options (and `Modifier::format_entry`).

## Tests
- Ported: Metrics 11/11 (the 2 formatConfig tests check the value text), Atlas 8/13, Sprite `start` 1/1, Face "render all sprites" 1/1.
- Skipped (Zig-only): Atlas `grow OOM`, `init error`, `reserve error`, `grow error` (allocator failure injection / tripwire), Atlas.Wasm `happy path` (wasm). `refAllDecls` tests.
- Added: canvas primitives, common helpers, braille layout, box/legacy smoke tests, `has_codepoint`, `ranges_sorted_and_non_overlapping`, `render_every_sprite_at_several_sizes` (every codepoint x 5 cell sizes x cell_width None/1/2 through `render_glyph`, checks trimmed bounds/ink), cursor-bar offsets.
- Reference comparison: `sprite_face_render_all_sprites` renders all 256-codepoint sheets at 18x36+4, 12x24+3, 11x21+2, 9x17+1 and compares with the upstream z2d reference PNGs (copied to `crates/ghostty-font/testdata/`) with a tolerance: <= 4% of ink pixels may differ by more than 96/255. Measured: braille and U+1CD00 sheets are pixel-identical; other sheets differ only by antialiasing (typical max delta <= 64; a handful of thin diagonal-hatch pixels in U+1FB98/99 and branch/curve glyphs at 9x17 differ by up to ~200).
- Review output (written by the tests): `target/sprite_sheets/` — `test-*.png` (our sheets), `diff-*.png` (red = reference, green = ours), `review-all-9x17.png`, `review-all-18x36.png` (every sprite on one sheet).

## Known gaps
- Pixel identity with z2d is not attempted (see above).
- Sprite `Face` is not yet wired into `Collection`/`SharedGrid` (those modules come later).
