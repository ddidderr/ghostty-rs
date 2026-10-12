# WASM port (W1)

Port of Ghostty's wasm-specific Zig. Everything is behind
`cfg(target_arch = "wasm32")` (or target-gated Cargo deps); native behaviour is
unchanged.

## File map
| Zig | Rust |
|---|---|
| `main_wasm.zig` | `crates/ghostty-wasm` (new, `cdylib`; empty on native) |
| `config/Wasm.zig` | `ghostty-wasm/src/config.rs` |
| `os/wasm.zig`, `os/wasm/target.zig`, `os/wasm/log.zig` | `ghostty-os/src/wasm/{mod,target,log}.rs` |
| zig-js (`js.*` used by Atlas/web_canvas) | `ghostty-os/src/wasm/js.rs` + host `ghostty-wasm/js/ghostty_host.js` |
| `font/face/web_canvas.zig` | `ghostty-font/src/face/web_canvas.rs` (+ exports in `ghostty-wasm/src/face.rs`) |
| `font/shaper/web_canvas.zig` | `ghostty-font/src/shaper/web_canvas.rs` (+ `ghostty-wasm/src/shaper.rs`) |
| `font/DeferredFace.zig` (`wc` variant, `Wasm`) | `ghostty-font/src/deferred_face_web_canvas.rs` |
| `font/Atlas.zig` `Wasm` | `ghostty-wasm/src/atlas.rs` |
| `font/backend.zig` | `ghostty-font/src/backend.rs` (`OPTIONS`/`default_for_target` = `WebCanvas` on wasm32) |
| `renderer/WebGL.zig` (empty stub), `App.zig` `Wasm` (commented out upstream) | nothing to export; `ghostty-wasm/src/app.rs` documents it |
| `os/passwd.zig` (`@compileError` on wasm) | `ghostty-os/src/unsupported.rs` shims |

## Build gating
- `ghostty-os`: nix/rustix/zbus/gettext target-gated; POSIX modules (`env`,
  `file`, `flatpak`, ...) are not compiled on wasm32. `unsupported.rs` gives
  API-compatible "unavailable" stand-ins (`xdg`, `passwd`, `expand_home`,
  `resources_dir`, `is_flatpak`) so `ghostty-config` compiles unchanged
  (Zig never analyzes them on wasm). `unsafe` is `deny` (not `forbid`) for the
  wasm import blocks only.
- `ghostty-config`: only `std::os::unix` bits changed (file-type names,
  `OsStrExt`).
- `ghostty-terminal`: `onig`, `ghostty-onig`, `rustix` target-gated. Like Zig
  with `oniguruma = false`, `StringMap::search_iterator`/`SearchIterator` do
  not exist on wasm (match types stay). Kitty shared-memory transmission
  returns `InvalidData` on wasm; `OsStrExt` shim for the file medium.
- `ghostty-font`: freetype/harfbuzz/fontconfig `-sys` target-gated; on wasm the
  `collection`, `codepoint_resolver`, `shared_grid(_set)`, `library`,
  `discovery`, `fontconfig`, `embedded`, `face::freetype`, harfbuzz shaper
  and `shaper::{run,cache,noop}` modules are compiled out.
- Builds: `cargo build --target wasm32-unknown-unknown -p ghostty-font -p ghostty-config -p ghostty-terminal -p ghostty-vt -p ghostty-wasm` passes (debug + release); clippy `-D warnings` clean on both targets.

## JS protocol (deviation)
`zig-js` (mitchellh/zig-js) is **not vendored** in the Ghostty tree, so its
import names could not be mirrored. `js.rs` documents a small handle-based
ABI in import module `ghostty_js` (`global/get/set/call/new/string/number/
boolean/to_f64/string_len/string_copy/is_nullish/bytes/copy_bytes/release`)
plus `env.log(ptr,len)` (same as Zig's log). The Rust-side API mirrors
`js.Object`. `ghostty-wasm/js/ghostty_host.js` is a reference host.
Allocation helpers `wasm_alloc/wasm_free` (host-owned buffers) replace the
stale `toHostOwned` (does not exist in current `os/wasm.zig`).

## Exports
Same names as Zig: `atlas_{new,free,reserve,set,grow,clear,debug_canvas}`,
`face_{new,free,render_glyph,debug_canvas}`, `deferred_face_{new,free,load}`,
`shaper_{new,free,test}`, `config_{new,free,load_string,finalize}`.
Differences/extras: `region_free`/`glyph_free` (Zig: "global free"),
`atlas_data/atlas_size`, `face_metrics`, `face_glyph_index`,
`shaper_{prepare,add_codepoint,shape}`, `config_format`,
`config_diagnostics_count`, `ghostty_wasm_init`, `wasm_alloc/free`.
`shaper_test` takes a `Face` (Zig takes a stale `GroupCache`) and prints into a
`ghostty-terminal` Terminal.

## Deviations from the (stale) Zig web_canvas code
- Zig refers to `Atlas.Format.rgba` and a 7-field `Metrics` that no longer
  exist; here Bgra atlases get the canvas RGBA bitmap with R/B swapped,
  grayscale gets alpha, and `Metrics::init()` + face size = cell size.
- The `RunIterator` hook API is tied to the FreeType `Collection` (not built on
  wasm), so `Shaper` exposes `prepare/add_codepoint/finalize/shape` directly and
  takes the grapheme-index callback (`Face::grapheme_glyph_index`).
- `Collection`/`SharedGrid` are not generic over the face type in this port, so
  they are unavailable on wasm (Zig's wasm exports never use them).
- `render_glyph` drops the unused `RenderOptions` parameter.

## Tests
- Native: pure parts of web_canvas (grapheme table, metrics math, presentation
  detection, glyph sizing, bitmap/atlas conversion; 9 tests) and the shaper
  clustering (6 tests) run natively; `backend::web_canvas_capabilities`.
- `crates/ghostty-wasm/tests/node.rs` builds the module (separate target dir
  `target/wasm-test`) and runs `tests/node_test.js` under node with a fake
  canvas: config parse/finalize/format, atlas, face metrics/glyph_index/
  render, emoji presentation, shaper clustering, deferred face, debug canvas.
  Skipped if node or the wasm32 target is missing.
- Not ported: Zig tests in Atlas.Wasm (`toHostOwned`, stale).

## Known gaps
- `ghostty-os/tests/common/flatpak_mock.rs` (another agent's, untracked) fails
  `clippy --all-targets -D warnings` (needless_pass_by_value); unrelated.
