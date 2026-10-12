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
| zig-js (`js.*` used by Atlas/web_canvas) | `ghostty-os/src/wasm/js.rs` + vendored zig-js host `ghostty-wasm/js/zig-js/` |
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

## JS protocol (zig-js, W1b)
The wasm module now speaks **exactly zig-js's ABI** (mitchellh/zig-js @
04db83c): import module `"zig-js"` with `valueGet/valueSet/valueObjectCreate/
valueStringCreate/valueStringLen/valueStringCopy/valueDeinit/valueNew/
funcApply`, 64-bit NaN-boxed refs (f64 for numbers; NaN head `0x7FF8_0000`,
bits 0-31 id, bits 32-34 type id; predefined ids nan/null/true/false/
undefined/global/runtime = 0..6) and the same get/set/apply/new/deinit
semantics. `ghostty-os/src/wasm/js.rs` is a Rust port of zig-js's Zig side:
`Ref`, `Type`, `Value` (low level), `Object` (`get::<T>`, `set`, `call::<T>`,
`new_instance`), `global()`, `runtime()`, `string()`, `Undefined`. `Object` has
`Drop` (= `deinit`); `Object::into_id` hands the ref id to the host (as
`atlas_debug_canvas` does). Result conversion mirrors `Object.convertValue`
(`Option<T>` for null/undefined, `()` = void, `Object/Value` retain,
others release). Deviation: a raw `Value` call argument is caller-owned.
The vendored host is zig-js's own `js/` (`crates/ghostty-wasm/js/zig-js/`,
unmodified TypeScript + MIT LICENSE; the lockfile, jest config are omitted).
The earlier invented `ghostty_js` ABI and `ghostty_host.js` are gone. `env.log`
(Zig's `os/wasm/log.zig`) is still a separate import.

`web_canvas` face and `atlas_debug_canvas` now call JS like the Zig code:
`document.createElement("canvas")`, `getContext`, `measureText`, `set("font")`,
`getImageData`, then `Uint8Array(runtime.memory.buffer, ptr).set(imageData.data)`
to copy pixels (no host-specific `copy_bytes`/`bytes`), and
`Uint8ClampedArray(memory.buffer, ptr, len)` (+ `.slice()` under
`wasm-shared`) + `ImageData` for the atlas. `face_debug_canvas` returns
`CanvasContainerNotFound` like Zig. `atlas_debug_canvas` returns the zig-js
ref id; free with `zigjs.deleteValue(id)`.
Remaining small deviation: `text_metrics` reads all five TextMetrics fields
instead of only the ones needed.

Host usage: `const zigjs = new ZigJS(); instantiate(bytes, {...zigjs.
importObject(), env: {log}}); zigjs.memory = instance.exports.memory;`
`globalThis` must provide `document`/`ImageData` (the host binds `globalThis`).
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

- Node test (`tests/node_test.js`) now loads zig-js's `ZigJS` straight from the
  vendored `.ts` (needs node >= 24 type stripping; skipped otherwise), uses a
  fake `document`/canvas on `globalThis`, asserts the import names are zig-js's,
  and checks the host value table has no leaked entries at the end.
- Native clippy of ghostty-os currently fails in `our_hostname` and wasm builds
  warn on `SEP` unused in `ghostty-os/src/lib.rs` (other agents' areas).
