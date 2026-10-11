# ghostty-font — F2 (config integration, run iterator, SharedGridSet, terminal-dependent shaper tests)

## Done
* Deps `ghostty-config`, `ghostty-terminal` re-added (+ dev-dep `ghostty-vt` for SGR/Rgb in tests).
* Type unification (canonical definitions live in `ghostty-config`, font re-exports):
  * `face::{Variation, VariationId}` = `ghostty_config::font_types` (`VariationId(u32)`; `VariationId::init(b"wght")`, `.0`, `.str()`).
  * `discovery::Descriptor` = config's; `Descriptor::toFcPattern` is now the free fn `discovery::to_fc_pattern(&Descriptor)`.
  * `codepoint_map::{CodepointMap, Entry}` = config's `CodepointMap`/`CodepointMapEntry` (`range: [u32; 2]`).
  * `metrics::Modifier` = `ghostty_config::metric_modifier::MetricModifier` (type alias; `ModifierParseError` and `Modifier::format_entry/parse_cli` removed, use `parse`/`parse_cli_value` and the config `ConfigField` impl). `ModifierSet`/`Key` stay in font (+ new `Key::ALL`). Config's `apply_i32` out-of-range now yields `-i32::MAX` like Zig (only config edit besides doc comment in `font_types.rs`).
  * `config_types.rs` is now only re-exports (`FreetypeLoadFlags`, `FontSyntheticStyle`, `FontShapingBreak`, `FontStyle`, `GraphemeWidthMethod`); `face::FreetypeLoadFlags` re-exports config's.
  * INTEGRATION_TODO: removed the three font lines; config line narrowed to clipboard map / CursorStyle.
* `shaper/run.rs` (run.zig): `TextRun`, `RunCell`, `RunOptions`, `RunIteratorHook` trait (impl for harfbuzz + noop shapers), `RunIterator`. Full logic ported: invisible-cell skipping, right trim, selection/cursor splitting, spacer skipping, "fl/fi/st" ligature splits, style change splitting (bg ignored), font-style/presentation selection, grapheme font search (`index_for_cell`), U+FFFD / space fallback, Kitty placeholder, relative-cluster position-independent hash. `shape.rs` re-exports `TextRun/RunIterator/RunOptions/RunIteratorHook`.
* `shared_grid_set.rs` (SharedGridSet.zig): `SharedGridSet` (`new`, `ref_` (Zig `ref`), `deref(&Key)`, `count`), `DerivedConfig::new(&Config)`, `Key` (descriptors, style offsets, codepoint map, metric modifiers, font size, freetype flags; hashcode-equality like the Zig map), collection building incl. discovery, variation-retry, built-in JetBrains Mono / Nerd symbols / emoji fallbacks. Grids are `Arc<SharedGrid>`; the set drops its Arc at ref-count 0. `discovery::Fontconfig` is now `Clone` (Arc inside) so the resolver shares the set's discoverer.
* `harfbuzz::Shaper::shape_run(&SharedGrid, &TextRun)` = Zig `shape(run)` (takes the grid write lock to get the face).

## Tests
* SharedGridSet.zig: 5/5 (Key, Key different font points, Key different font DPI, SharedGridSet + extra `key_descriptors_and_modifiers`).
* All 23 deferred harfbuzz tests ported (in `shaper/harfbuzz.rs`), plus F1's terminal-free ones kept. Font crate: 157 pass, 3 ignored. Ignored (always `error.SkipZigTest` upstream, bodies ported from the commented-out Zig): `shape_tai_tham_vowels_position_differs_from_advance`, `shape_tai_tham_letters_run_offset_y_differs_from_zero`, `shape_javanese_ligatures`.
* Tests needing a discovered system font (Devanagari + Bengali = "Arial Unicode MS", Chakma = "Noto Sans Chakma", Tibetan = "Noto Serif Tibetan") return early (Zig `SkipZigTest`) when the font is missing; only Tibetan is installed on this machine (verified running); Devanagari/Chakma/Bengali are therefore not exercised here.
* Removed duplicate tests that moved to config: F1's `variation_id_*` in face.rs and `formatconfig_*` in metrics.rs (still covered by ghostty-config tests); adapted `modifier_parse_errors`.

## Deviations / TODO(integration)
* `Terminal`/`RenderState` do not exist (usable) in ghostty-terminal yet, so the tests use a small `Term` helper over `Screen` (`shaper/test_util.rs`): `test_write_string` for text, own grapheme-cluster attach via `ghostty_unicode::grapheme_break` for tests that enable `grapheme_cluster` mode, `Screen::set_attribute` for SGR, `cursor_right` for `\e[5C`. `RunCell::from_page_row(page, y)` replaces `RenderState` cells (`TODO(integration)`: switch to the terminal render-state cell type). Once `Terminal` is final the tests can be rewritten to drive it with the VT stream.
* `TextRun` has no `grid` field (keeps it `Copy`); the iterator's `RunOptions.grid` is `&SharedGrid`. `RunIterator::next(&mut shaper)` takes the shaper (hooks) per call instead of storing it, so the shaper can shape between calls.
* Run hash uses std `DefaultHasher` (Zig: Wyhash) — only used as a cache key.
* `KITTY_PLACEHOLDER` (0x10EEEE) is a local const (TODO(integration): terminal kitty graphics unicode module).
* macOS Apple Emoji branch in the collection builder is omitted (Linux only); discovery is always available.
* `SharedGridSet::ref_` holds the set lock while building the collection (as Zig).
* `shared_grid::CellSize` still stands in for `renderer::CellSize`.
* Quality gate: `cargo build/test` ok; clippy with `-D warnings` for ghostty-font passes with `--no-deps` (the dependency ghostty-terminal currently fails clippy in its in-progress `terminal.rs`, not my crate); fmt applied.
