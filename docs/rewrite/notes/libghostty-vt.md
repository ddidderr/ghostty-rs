# libghostty-vt C API (Phase 8, agent L2)

Crate `crates/ghostty-vt-capi` (crate types `cdylib` + `staticlib` + `rlib`).
Depends only on `ghostty-vt` and `ghostty-input` (no `ghostty-terminal`), so it
builds for `wasm32-unknown-unknown`.

## Mapping

| Zig | Rust |
| --- | --- |
| `lib_vt.zig` `@export` list | `#[unsafe(no_mangle)] extern "C"` fns (38 native symbols) |
| `terminal/c/color.zig` | `color.rs` (`ghostty_color_rgb_get`, `GhosttyColorRgb`) |
| `terminal/c/osc.zig` | `osc.rs` |
| `terminal/c/sgr.zig` | `sgr.rs` (+ wasm attribute helpers in `wasm.rs`) |
| `terminal/c/key_event.zig` | `key_event.rs` |
| `terminal/c/key_encode.zig` | `key_encode.rs` |
| `terminal/c/paste.zig` | `paste.rs` |
| `terminal/c/result.zig` | `result.rs` |
| `lib/allocator.zig` (C allocator) | `alloc.rs` |
| `lib/allocator/convenience.zig` | `wasm.rs` (exported only on wasm32) |

Headers are copied unchanged to `include/ghostty/` (`diff -r` against
`ZIG/include/ghostty` is empty). The C examples (`examples/c/*.c`) and wasm
examples (`examples/wasm/*.html`) are unchanged copies of
`ZIG/example/{c-vt,c-vt-key-encode,c-vt-paste,c-vt-sgr,wasm-key-encode,wasm-sgr}`.

## Library file names

Cargo forbids hyphens in `[lib] name`, so the artifacts are
`libghostty_vt_capi.{so,a}` / `ghostty_vt_capi.wasm`. `build.rs` sets the
soname to `libghostty-vt.so`; `just vt-lib [profile]` (and the integration
tests) install copies as `libghostty-vt.{so,a}` and `ghostty-vt.wasm`
(upstream's names).

## Tests

* unit tests: ports of the Zig `test` blocks in `terminal/c/*.zig`
  (22 tests; the `lib_alloc.test_allocator` leak-check variants map to plain
  alloc/free round trips).
* `tests/abi.rs`: generates a C probe from the headers (enum typedefs and
  `#define`s are parsed from the header text, every declared function's address
  is taken), compiles it with `cc`, links it against `libghostty-vt.so`, and
  compares sizeof/alignof/offsetof of every struct/union, sizeof of every enum
  and typedef, the value of every enumerator and `#define` against the Rust
  definitions (`size_of`/`align_of`/`offset_of!`, `Key::ALL`, discriminants).
  A second test checks `nm -D` exports == declared functions.
* `tests/c_examples.rs`: compiles the four C examples with `cc` and checks
  stdout (upstream has no expected-output files; the strings were derived by
  reading the example sources), plus a static-link run against `libghostty-vt.a`.
* `tests/wasm_examples.rs` + `tests/wasm/run.mjs`: builds
  `--target wasm32-unknown-unknown`, checks every header function (including
  the `__wasm__`-only `ghostty_wasm_*`) is exported and that the module has no
  imports, then runs the two wasm example pages' unmodified `<script>` under node
  with a DOM stub; key-encode output is compared against the native Rust
  encoder, SGR output against literals. Skipped (with a note) if `node` or the
  wasm target is missing.

`cargo test -p ghostty-vt-capi` runs all of it (`just vt-test`). The tests run a
nested `cargo build` of the crate (and the wasm target) into the same target dir.

## Deviations / notes

* `unsafe_code = "deny"` for this crate (lints table copied from the workspace
  because `workspace = true` cannot be partially overridden); FFI modules carry
  `#![allow(unsafe_code)]` and SAFETY comments.
* Enum parameters from C (`action`, `key`, `option`, `data`) are taken as
  `c_int` and validated (Zig only validates in safe builds); invalid values are
  ignored/rejected like Zig's `runtime_safety` path. Null handles are no-ops /
  return defaults instead of Zig's `.?` panic.
* Wrapper objects and the SGR parameter copy use the caller's
  `GhosttyAllocator` (copied by value at `*_new`); interior Rust collections
  (OSC buffer) always use the Rust global allocator.
* `ghostty_osc_end` returns a pointer to a command slot owned by the parser
  (Zig returns a pointer to the parser's command); the window title is
  re-materialized NUL-terminated there. Valid until the next `end`/`free`.
* `GhosttyOscCommandType` can also yield 23/24 (`kitty_clipboard_protocol`,
  `context_signal`) which exist in the Rust/Zig parser but not in `osc.h`.
* Key event `utf8` is a Rust `&str` in `ghostty-input`; bytes that are not
  valid UTF-8 are truncated to the valid prefix when encoding. `get_utf8`
  returns the caller's original pointer/len. Unknown mod bits (padding) are
  dropped on `set_mods`.
* `ghostty_sgr_set_params` with `separators != NULL` and `len > 24` returns
  `INVALID_VALUE` after replacing the params (same order as Zig). Non-NULL
  required `params` with `len > 0` and NULL pointer returns `INVALID_VALUE`
  (Zig: UB).
* Upstream wasm builds import `env.log` for `std.log`; the Rust build has no
  host imports (the `log` crate has no logger installed).
* Upstream finding: `example/wasm-sgr/index.html` has a stale `SGR_ATTR_TAGS`
  table (header changed in upstream commit `a384af5e2`, example not updated), so
  the stock page mislabels e.g. direct colors. The test runs the stock page
  (checks only the still-correct underline line) and a copy whose table is
  regenerated from `sgr.h` (checks full output).
* The Zig reference could not be built here (needs Zig 0.15; 0.16 installed),
  so outputs were validated against the Zig sources and the protocol rather
  than a Zig-built library.
