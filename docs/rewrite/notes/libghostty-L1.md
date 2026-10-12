# libghostty: the embedded C API (Phase 8, agent L1)

Crate `crates/ghostty-lib` (crate types `cdylib` + `staticlib` + `rlib`, `[lib] name = "ghostty"` so Cargo
produces `libghostty.{so,a,dylib}` directly). `include/ghostty.h` and `include/module.modulemap` are
byte-identical copies of `ZIG/include` (a test compares them when the Zig checkout is present).
`unsafe_code = "deny"` crate wide; every FFI module has `#![allow(unsafe_code)]` with `// SAFETY:` comments.

## Mapping

| Zig | Rust (`crates/ghostty-lib/src`) |
|---|---|
| `main_c.zig` (`ghostty_init`, `_cli_try_action`, `_info`, `_translate`, `_string_free`, `String`) | `main_c.rs` |
| `config/CApi.zig`, `config/c_get.zig`, `Config.zig` `cval`s | `config_capi.rs` + `ghostty-config/src/c_value.rs` (`CValue`, `ConfigField::c_value`, `Config::c_value`) |
| `apprt/action.zig` `C`/`cval` (target + action unions) | `action_c.rs`, C layouts in `types.rs` |
| `apprt/embedded.zig` `App`, `Surface`, `Platform`, `CAPI` | `embedded.rs` (`AppRt: RtApp`, `SurfaceRt: RtSurface`, `AppHandle`, `Surface`, all `ghostty_app_*`/`ghostty_surface_*`) |
| `apprt/embedded.zig` `Inspector` + `ghostty_inspector_*` | `inspector.rs` (+ `inspector_metal.rs`, Apple only) |
| `benchmark/CApi.zig` | `benchmark.rs` (`ghostty_benchmark_cli`) |
| `include/ghostty.h` structs/unions/enums | `types.rs` (`#[repr(C)]`, enums are `c_int`) |

`ghostty_set_window_background_blur` is exported on every platform (no-op off macOS), like Zig. The Darwin-only
exports (`ghostty_surface_set_display_id`, `_quicklook_font`, `_quicklook_word`, `ghostty_inspector_metal_*`) are
`cfg(target_vendor = "apple")`, matching the header's `#ifdef __APPLE__`.

## Runtime design

* The embedded runtime implements `ghostty_apprt::runtime_api::{RtApp, RtSurface}`; the real `ghostty-core` drives it.
  `AppRt` holds a copy of `ghostty_runtime_config_s`, the app config (`Arc<Config>`, updated on the app-targeted
  `config_change` action exactly like Zig's `performPreAction`), the surface registry (for titles) and the `Keymap`.
  `SurfaceRt` holds userdata, the view pointer, content scale / size / cursor position and the title.
* `ghostty_app_t` = `Box<AppHandle>`, `ghostty_surface_t` = `Box<Surface>`. The address of the `Surface` is its
  `SurfaceId` in core and the `ghostty_surface_t` in `ghostty_target_s`, so actions fired during
  `Surface::new` (cell size, size limit, title) already name the handle the host will get back.
* `core::App::create_surface` (new) returns the concrete `Rc<core::Surface>` so the embedded API can reach everything
  Zig's `core_surface` exposes (`text_callback`, `refresh_callback`, `draw`, `mouse_captured`, `dump_text_locked`,
  `lock_terminal`, ...), not only the `CoreSurface` trait.
* Everything is main-thread only (core uses `Rc`/`RefCell`); the single cross-thread entry is `wakeup_cb`
  (`WakerImpl`, a `Waker`). No `RefCell` borrow is held across a call into the host (the host may re-enter).
* Clipboard: `read_clipboard_cb` gets a `Box<ClipboardRequest>` pointer that is consumed by
  `ghostty_surface_complete_clipboard_request` (kept alive when `confirm_read_clipboard_cb` is invoked, like Zig).
* `ghostty_config_t` is a `Box<CConfig>` (config + the memory `ghostty_config_get` hands out pointers into: strings,
  color lists, command lists, diagnostics; kept until `ghostty_config_free`, identical values return identical
  pointers). Enum tag names are interned for the process lifetime. The `config_change` action passes a temporary
  `CConfig` handle that is freed after the callback returns.

## Config C getters (`c_get.zig`)

`ConfigField::c_value() -> CValue` (default `Unsupported`) is implemented by: `bool`, `u8`/`u32` (`c_uint`), `i16`
(`c_short`), `f32`/`f64`, `Option<String>` (`?[:0]const u8`: unset -> true + NULL), every `config_enum!` (tag name),
every `config_packed!` (bits as `c_uint`, <= 32 fields), `Option<T>` (None -> false), `Color`, `ColorList`, `Palette`,
`RepeatableCommand`, `QuickTerminalSize`, `BackgroundBlur` (0/20/radius/-1/-2), `Duration` (`usize` ms), `Path`,
`WindowDecoration`, `CursorStyle`, `OptionAsAlt`. Everything else (`String`, `u16`, `u64`, `usize`, unions without
`cval`, repeatable types, ...) is unsupported (get returns false) exactly like Zig. `ghostty-config` has a test
(`c_value_supported_keys_match_zig`, data in `testdata/c_get_supported.txt`) that compares the supported key set
(143 keys) with the one derived from the Zig field types. `RepeatableStringMap::put` was added (needed for
surface `env_vars`).

## Platform gating (renderer selection)

* `ghostty-core` no longer hard-codes OpenGL: `surface/mod.rs` uses `ghostty_renderer::{Renderer, Api}`
  (Metal on `target_vendor = "apple"`, OpenGL elsewhere; Zig's `renderer.zig`). `init_graphics_api` is
  `OpenGL::init` + `prepare_context(rt.gl_proc_address)` off Apple and `Metal::init_with_view(blending, view, scale)`
  on Apple. `RtSurface::platform_view()` (new, default null) carries the `NSView`/`UIView`; `Metal::init_with_view`
  (new, in the renderer crate's unsafe-allowed `metal` module) turns the raw pointer into the `ViewInfo`.
  GL-only helpers (`frame_pixels`, screenshots, `display_realized(loader)`) are `cfg(not(apple))`.
* Surfaces: Zig's `Platform.init` only accepts `macos`/`ios` on Darwin targets and fails with `UnsupportedPlatform`
  everywhere else, so `ghostty_surface_new` returns NULL on Linux (tested). No host-supplied OpenGL context exists in
  Zig, so there is none here either.
* New renderer/font/core helpers for the Darwin exports: `Renderer::font_grid()`, `coretext::Face::quicklook_font`,
  `core::Surface::{set_macos_display_id, quicklook_font}` (core gained an Apple-only `objc2-core-foundation` dep).
* Test seam: cargo feature `test-platform` (off by default, enabled by `--all-features`) lets `ghostty_surface_new`
  accept `platform_tag = 1000` and takes the OpenGL loader from a thread-local the test installs (headless EGL).
  Production behaviour is unchanged.

## Deviations from the Zig

* `ghostty_init` stores the given argv only to detect the CLI action; config CLI parsing and `+action` execution use
  the process arguments (`ghostty_os::args`, `NSProcessInfo` on macOS), which are the same in a real process.
  `ghostty_init` returns 1 for multiple/invalid actions (Zig: error from `detectArgs`). The `+action` runs through
  `ghostty_cli::run_from_args` (so `+version` etc. print exactly like the binary).
* Lib-mode logging default: stderr off (`GlobalState.Logging.stderr = app_runtime != .none`), via the new
  `ghostty_core::global::init_lib`. `BuildInfo` for `+version` reports `.none` runtime / CoreText + Metal on Apple.
* Invalid enum values coming from C (mouse state/button, split directions, color scheme, key action, pressure stage)
  are ignored / return false (Zig: UB / safety panic). Null handles are tolerated.
* `ghostty_surface_key_is_binding` writes only the low byte (Zig writes the `u8` `Flags.C` through the enum pointer).
* `ghostty_surface_inherited_config` working-directory strings are kept in the surface (valid until it is freed) instead
  of leaking an allocation per call.
* `initial_input` is escaped like `std.zig.stringEscape`, except non-ASCII code points use `\u{..}` (the config `Raw`
  value is a Rust `String`, which cannot hold the `\xNN` bytes of a split UTF-8 sequence).
* `ghostty_benchmark_cli` splits the args string itself (quotes group, `\` escapes inside quotes) and runs
  `ghostty_bench::benches::main_action`.
* Header typo kept: `chostty_ipc_target_s`; `ghostty_inspector_metal_shutdown` is declared `bool` in the header but
  Zig exports it as `void` (we follow Zig; the Swift app ignores the result).

## Inspector

C API complete: `ghostty_surface_inspector` (one per surface, returns the existing one), `ghostty_inspector_free`
(takes the *surface*), size / content scale / focus / mouse / scroll / key / text feed a per-inspector Dear ImGui
`Context` (`ghostty_inspector::setup`, style scaled by the factor change, scroll x0.1 for precision devices, only
left/middle/right buttons). Activation calls `core::Surface::activate_inspector` and `RtSurface::redraw_inspector`
raises the `render_inspector` action, as in Zig.

`inspector_metal.rs` (Apple only, compile-checked only) replaces Dear ImGui's `imgui_impl_metal.mm`: the same MSL
shaders compiled with `newLibraryWithSource`, a pipeline keyed by the host render pass' formats, per-draw-list buffers,
`dear-imgui-rs` managed textures (RGBA32/Alpha8 -> RGBA8), `RENDERER_HAS_VTX_OFFSET | RENDERER_HAS_TEXTURES`. Two
frames are built per render (the first only `end_frame`), like Zig. Raw draw callbacks are ignored. It has never run
on a Mac.

## Tests

* Unit: 20 in `src` (string helpers, `ghostty_config_get` ports of the Zig tests: bool/enum/optional null/unknown
  key/optional string null/float/struct cval, the `c_get:` tests u8/enum/color/optional/background-blur/
  split-preserve-zoom, action/target conversions, benchmark arg splitting). `ghostty-config`: +4 `c_value` tests and the
  two `RepeatableCommand cval` tests (previously skipped).
* `tests/abi.rs` (generated C probe compiled with `cc`, linked against `libghostty.so`): sizeof/alignof/offsetof of every
  struct and union of the header (field names parsed from the header, so a missing field fails), sizeof + value of
  every enumerator of every enum (755 values, compared with the Rust definitions: `Key::ALL`, `apprt::action::Key`,
  `MouseShape`, `Mods` bits, ...), the address of every declared function (links), `nm -D` exports == declared
  functions, header files identical to upstream. The probe asserts every header enum/struct is covered.
* `tests/c_host.rs` + `tests/c/host.c`: a C host program on the unchanged header: `ghostty_init`, `+version` /
  `+list-keybinds` through `ghostty_cli_try_action`, config new/load_file/finalize/get (bool, enum, string, colors,
  palette, float, packed, `short` blur, duration, quick-terminal-size, command list, unsupported/unknown/unset keys),
  triggers, diagnostics, clone, open_path, app new/tick/key/key_is_binding/focus/color scheme/open_config/
  update_config (the `config_change` action carries a readable config), global keybinds, `ghostty_surface_new`
  returning NULL off Darwin, `ghostty_benchmark_cli`.
* `tests/surface.rs` (feature `test-platform`, headless EGL, real shell on a real pty): 13 tests driving the exported C
  surface API: creation/actions/size/scale/free, text + key input, key_is_binding flags, selection + copy/paste
  (including the unsafe-paste confirmation round trip), OSC 52 / title / pwd actions, inherited config, env vars /
  working directory / initial input / font size options, mouse capture and reports, focus/occlusion/preedit/splits/
  close callback, child exit, app + surface config updates, inspector API, rejected platforms.
* Cross-compile checks (recipe from `macos-M1.md`, wrappers around `zig cc`; the CoreGraphics/Metal/CoreText code is
  only type-checked): `cargo check|clippy --all-targets -p ghostty-lib --target aarch64-apple-darwin` and
  `x86_64-apple-darwin` pass with `-D warnings`, as does `-p ghostty-core`.

## Known gaps

* Nothing in the Apple-only paths (Metal surface setup, display link, quicklook, window blur, Metal inspector) has run
  on a Mac; the Swift app has not been built against the library.
* `ghostty_translate` returns interned translations; `ghostty_init` ignores `argv` beyond action detection (see above).
* Only the debug-profile static library was inspected (~500 MB with debug info); release/production sizes untested.
