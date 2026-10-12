# macOS CoreText font backend (M1 / font)

Crate: `ghostty-font`. Compile-checked with
`cargo check|clippy --target aarch64-apple-darwin|x86_64-apple-darwin -p ghostty-font --all-targets`
(clean, `-D warnings`); never run on real macOS. Linux behaviour unchanged
(`cargo test -p ghostty-font`: 203 pass / 3 ignored, clippy `-D warnings`, nightly fmt).

## Backend selection (`font/backend.zig`)
`build.rs` emits cfgs from target + cargo features: `font_freetype`, `font_fontconfig`,
`font_harfbuzz`, `font_coretext` (Apple), `font_coretext_face`, `font_coretext_shaper`, `font_noshape`.
* Linux/other: fontconfig + FreeType + HarfBuzz (unchanged). wasm32: web_canvas (unchanged).
* Apple targets, default `Backend::Coretext`: CoreText discovery/face/shaper; freetype-sys,
  harfbuzz-sys, fontconfig are NOT linked (freetype/harfbuzz are optional deps, `bundled`, apple only).
* Cargo features (apple only): `coretext-freetype` (CT discovery, FreeType face, HarfBuzz shaper),
  `coretext-harfbuzz` (CT face + HarfBuzz via `hb_coretext_font_create`), `coretext-noshape`
  (no-op shaper). `backend::OPTIONS` follows the features. `face::DEFAULT_DPI` is 72 on macOS.

## Module map (Zig -> Rust, `crates/ghostty-font/src/`)
| Zig | Rust |
|---|---|
| face/coretext.zig | `face/coretext.rs` (CTFont face, CoreGraphics bitmap rasterization, `ColorState`) + `face/coretext_math.rs` (pure math, tested on Linux) |
| shaper/coretext.zig | `shaper/coretext.rs` (CTTypesetter/CTLine/CTRun, feature dicts, per-font attr-dict cache) + `shaper/coretext_cells.rs` (pure cluster/ligature logic, tested) + `shaper/cf_release.rs` |
| discovery.zig CoreText + `toCoreTextDescriptor` | `discovery_coretext.rs` (`CoreText`, `Score`-based sorting, CJK / `CTFontCreateForString` fallbacks) |
| DeferredFace.zig CoreText | `deferred_face_coretext.rs` (+ `coretext_freetype` file-path load) |
| library.zig (noop for CT) | `library.rs` (empty library when no FreeType) |
| SharedGridSet.zig Apple Emoji branch | `shared_grid_set.rs` (Apple Color Emoji via discovery on CT; embedded Noto emoji otherwise) |
| (CF helpers of the `macos` package) | `coretext_util.rs` (`CfBox` Send+Sync wrapper, tables, glyph lookup, variation axes) |

API unification (so collection/grid/resolver stop naming `face::freetype::*`): `face::{Face, FaceError,
RenderError, HbFont}`, `shape::{Shaper, ShapeError, ShapeRunError}`, `discovery::{Discover, NextError}`.
`discover_fallback` now takes `&mut Collection` (CT needs `get_face`); no-op shaper gained `shape_run`,
`ShapeError` and `new -> Result`.

## Tests
Ported & platform-independent (run on Linux): metrics table priority (OS/2 typo vs hhea vs win),
underline/strikethrough fallbacks, synthetic-bold width, glyph bearing/canvas layout (incl. sbix, thicken,
bold), UTF-16/surrogate table, CoreText cell building incl. ligature/out-of-order-cluster/non-LTR heuristics,
discovery `Score` ordering, style fuzzy/exact scoring, variation-axis bold/italic thresholds, backend
capability predicates. macOS-only tests written but not runnable here (`#[cfg(font_coretext...)]`):
face (Monaco ASCII, name, emoji, in-memory, variable, set variation, SVG table, glyphIndex colored vs text),
discovery (coretext, codepoint, sorting), deferred face. The Zig CoreText shaper tests (23, need
Terminal/RenderState + system fonts) are NOT ported; resolver/collection tests adjusted for CT
(`Coretext doesn't support Noto's format`, text-emoji index 1).

## Caveats / TODO(integration)
* Never executed on macOS: rasterization (CGBitmapContext flags, colour spaces), shaping and discovery are
  faithful ports but unverified at runtime.
* `coretext-freetype` / `coretext-harfbuzz` cannot be cross-checked here: bundled harfbuzz with CoreText
  needs the macOS SDK headers (`ApplicationServices`). `coretext-noshape` and the default are checked.
* `face_metrics` returns `Some(..)` for cap/ex/ascii height always (Zig returns plain values for CT).
* CF release thread is local to the font crate (`shaper/cf_release.rs`, std thread + channel);
  switch to `ghostty_os::cf_release_thread` once that exists.
* `Face::init` copies in-memory font data into a CFData (Zig used no-copy).
* Renderer crate must use `ghostty_font::shape::{ShapeError, ShapeRunError}` instead of
  `shaper::harfbuzz::{ShapeError, ShapeRunError}` (generic.rs lines ~144-146) to build on macOS.
* Cross-check env: `CC_<target>`/`AR_<target>` zig-cc wrappers (onig_sys C build); `GETTEXT_DIR` dummy.
