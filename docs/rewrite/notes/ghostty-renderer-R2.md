# ghostty-renderer — R2 (generic renderer, cells, links, images, overlay, state, thread)

Builds on R1 (`ghostty-renderer-R1.md`: OpenGL backend, shaders, shadertoy, size, cursor).
Deps re-added: `ghostty-terminal`, `ghostty-config`, `ghostty-input`, `ghostty-unicode`, `ghostty-vt`,
`ghostty-datastruct` (mailbox queue), `crossbeam-channel`, `onig` (link regexes), `png` + `jpeg-decoder`
(background image; replaces wuffs). No new `unsafe` (still only `opengl::gl`).

## Module map (Zig -> Rust, `crates/ghostty-renderer/src/`)
| Zig | Rust | notes |
|---|---|---|
| generic.zig | `generic.rs` | `Renderer`, `DerivedConfig`, `Host` trait, `Error`, `NullHost`, `update_frame`, `draw_frame`, rebuild_cells/row, cursor, preedit, decorations, custom-shader uniforms, bg image |
| renderer.zig | `lib.rs` | `pub type Renderer = generic::Renderer` (OpenGL), `DerivedConfig`, `Thread`, `Message`, `State` re-exports |
| cell.zig | `cell.rs` | `Contents`, `Key`, `is_covering`, `is_symbol`, `constraint_width`, `no_min_contrast` |
| row.zig | `row.rs` | `never_extend_bg` |
| link.zig | `link.rs` | `Set::from_config`, `Set::render_cell_map` (onig, uses `RenderState::string` + byte map) |
| State.zig | `state.rs` | `State`, `SharedState = Arc<Mutex<State>>`, `Mouse`, `Preedit`(+`range`) |
| Overlay.zig | `overlay.rs` | CPU RGBA canvas instead of z2d (only axis-aligned rects) |
| image.zig | `image.rs` | `State`, `Image` state machine, `Placement`, `Id`, `Pending`, kitty placement/virtual placement building |
| Thread.zig | `thread.rs` | `Thread`, `Handle`, `Running`, `Mailbox`, `DerivedConfig` |
| message.zig | `message.rs` | stubs from R1 replaced by real types |
| cursor.zig | `cursor.rs` | now takes `&RenderState`; the 4 Zig tests drive a real Terminal |
| Options.zig | `options.rs` | `Options<C>{config, font_grid, size, host}` |

## How the core Surface creates and drives the renderer

```rust
use ghostty_renderer::{generic::{Host, Renderer, DerivedConfig}, options::Options, opengl::OpenGL,
                       state::State, thread::Thread, message::Message};

// 1. In the GLArea `realize` handler (GL context current on the GTK thread):
let mut api = OpenGL::init(config.alpha_blending);
api.prepare_context(|name| gl_get_proc_address(name))?;       // Zig Renderer.surfaceInit
let (key, grid) = font_set.ref_(&font_derived_config, font_size)?;   // SharedGridSet
let renderer = Arc::new(Renderer::new(
    Options { config: DerivedConfig::new(&config)?, font_grid: grid, size, host: host.clone() },
    api)?);                                                   // creates buffers/textures/shaders: GL thread only

// 2. Shared state (replaces Zig's `renderer.State` + `renderer_state.mutex`):
let state: SharedState = State::new(terminal).into_shared();  // termio locks it to write the terminal
//    state.preedit / state.mouse / state.overlay_features are set by the Surface under the same lock.

// 3. Thread:
let thread = Thread::new(&config, renderer.clone(), state.clone(), host.clone());
let running = thread.spawn()?;                                // `running.handle` is Clone + Send
let handle = running.handle.clone();
```
* `Host` (implemented by core/apprt, `Send + Sync`) is the "surface mailbox": `scrollbar(Scrollbar) -> bool`
  (instant push; return false if full, the renderer retries next frame), `renderer_health(Health)`, and
  `redraw_surface() -> bool` (renderer thread -> apprt: "please call `draw_frame` on the main thread").
  Implementations must not block: `scrollbar` is called from `draw_frame` with the draw mutex held.
  `generic::NullHost` discards everything.
* Wake the thread after changing terminal state (termio does `handle.wakeup()` after parsing, Zig `renderer_thread.wakeup.notify()`).
* Messages: `handle.send(Message::..)` = push(forever) + wakeup; `handle.push(msg, Timeout)` without wakeup.
  `Focus(bool)`, `Visible(bool)`, `ResetCursorBlink`, `Resize(Size)`, `FontGrid(Box<FontGridChange{grid, set, new_key, old_key}>)`
  (thread calls `set.deref(&old_key)` after adopting), `ChangeConfig` (`Message::init_change_config(&Config)?` builds both derived configs),
  `SearchViewportMatches(Vec<Flattened>)`, `SearchSelectedMatch(Option<Flattened>)`, `Inspector(bool)`, `Crash` (panics on purpose).
  `Handle::draw_now()`, `Handle::stop()`.
* GTK drawing: in the GLArea `render` signal handler call `renderer.draw_frame(false)` (`true` = sync; the apprt/Surface
  uses `true` right after init and on resize like Zig `Surface.zig:869`). In the `redraw_surface` host callback call
  `glarea.queue_render()`. On GLArea `unrealize` call `renderer.display_unrealized()` (context current) and on the next
  `realize` `renderer.display_realized(loader)`; then draw with `sync = true`.
* Shutdown: `running.stop_and_join()` (thread drops its `Arc<Renderer>`), then drop the last `Arc<Renderer>` on the GL thread
  with the context current (it deletes GL objects); `font_set.deref(&key)`.
* Debug/screenshot: `renderer.last_frame_pixels()` (RGBA8, bottom-up) reads the last presented target.

## Threading model (documented in `generic.rs` and `thread.rs` module docs)
GTK GLArea's context is only current on the main thread, so (as in Ghostty, `must_draw_from_app_thread = true`):
* renderer thread: owns timers (cursor blink 600ms; 8ms draw timer only when custom shaders animate), drains the mailbox,
  and per wakeup runs `Renderer::update_frame` (lock `SharedState`, `RenderState::update`, OSC8/regex links, search highlights, overlay,
  shaping with the harfbuzz shaper + cache, glyph rendering into the font atlases via `SharedGrid`, rebuild CPU `cell::Contents`, uniforms)
  and then `Host::redraw_surface()`. It performs **no GL calls**.
* main thread: `Renderer::draw_frame` uploads cells/uniforms/atlases/images/bg image, runs the render passes (bg image or bg color,
  kitty below-bg, cell bg, kitty below-text, text, kitty above-text, overlay, custom shader post passes), `glFinish`s and presents.
* State split (no unsafe): `draw: Mutex<DrawState>` (Zig draw_mutex: what `draw_frame` reads) and `thread: Mutex<ThreadState>`
  (RenderState, shaper, cache, overlay, search; uncontended, only there for `Sync`). Lock order: `SharedState` (terminal) is never held
  while waiting for `thread`; `thread` -> `draw` -> font grid lock. `Thread::draw_on_app_thread = false` makes the thread call `draw_frame`
  itself (needs the context current on that thread; used for headless use).
* The swap chain keeps Zig's structure (`SWAP_CHAIN_COUNT = 1`) minus the semaphore: frames complete synchronously and the draw mutex serializes.

## Tests
* Ported: cell.zig 6/6 (`Contents`, `clear retains other content`, `clear last added content`, `zero-sized screen`, `Cell constraint widths`
  now with real Terminal+RenderState; +`reset` and helper tests), link.zig 3/3, cursor.zig 4/4 (real Terminal) + password test,
  row.zig (none upstream; 4 added), State.zig (none upstream; 4 preedit tests added), Overlay.zig (none upstream; 4 added),
  image.zig (none upstream; 7 added incl. a real `ImageStorage` -> `kitty_update` test), generic.zig (none upstream; 6 added: derived config,
  color resolution, bold policy, file types, PNG bg image round trip), thread.rs (3).
  Totals: 69 unit tests + 9 integration tests in `tests/egl_headless.rs` (3 from R1), all pass.
* New EGL integration tests (surfaceless Mesa GL 4.3): full path Terminal -> RenderState -> `Renderer` -> offscreen target ->
  `last_frame_pixels`: exact bg color at all four padding corners and the side padding, red glyph pixels in "hello", non-bg glyph pixels,
  exact palette color for cell backgrounds (SGR 42), block cursor drawn in the foreground color, blink-off hides the cursor, incremental
  row rebuild, underline sprite drawn, resize/mark_dirty, unrealize -> `Defunct` -> realize -> draw again, and a real `Thread`
  (initial wakeup renders and requests a draw, Focus message -> hollow cursor outline with empty center, stop/join).
* Skipped: none of the Zig tests (Metal-only/allocator-failure ones don't exist in these files).

## Deviations
* Run iterator wants `&[RunCell]` (owned `Vec<u32>` graphemes); each rebuilt row copies `RenderState` cells into a reusable scratch
  `Vec<RunCell>` (only cells with graphemes allocate). Shaped cells are copied from the shaper/cache into a scratch vec.
* `renderer.State`: one mutex-protected struct holding `terminal`, `preedit`, `mouse`, `overlay_features`
  (inspector deferred: `inspector: ?*Inspector` replaced by `overlay_features`). Zig `*Mutex` + pointers -> `SharedState`.
* Overlay: no z2d; direct RGBA rasterization (fill rect + 1px hairline border, src-over). Hairline placement is approximate.
* Background image decoding: `png` / `jpeg-decoder` instead of wuffs (png/jpeg only, same as Zig; other types warn).
* Zig `cursor_text.color` / `selection_*.color` custom-shader uniforms read the union payload unconditionally (invalid for `cell-*`);
  Rust only sets them for explicit colors.
* Zero-length regex matches advance by one character (Zig would loop forever).
* Atlas textures in `FrameState` start 2x2 (not 1x1: `Atlas::new` keeps a 1px border); replaced on first sync.
* `opengl::AlphaBlending` is now `ghostty_config::types_b::AlphaBlending` (R1 stub removed). `message.rs`: real derived-config types.
* `display_unrealized` also resets the image state and bg image (their textures die with the GL context); forces a kitty update.
* `Host::renderer_health` is called while holding the draw mutex (Zig pushes `.forever` too); keep it non-blocking.
* macOS-only paths (display link, glass blur, QoS) omitted.

## TODO(integration)
* **kitty graphics**: `Screen.kitty_images` is still the placeholder `KittyImages { dirty }` in ghostty-terminal. `image::State::kitty_update(&ImageStorage,
  &PageList, &TerminalInfo, CellSize)` is implemented and tested against the real storage, but `generic::update_kitty` cannot call it yet;
  once K1's step 1 lands (screen.rs holds `ImageStorage`), replace the body with the call and clear `storage.dirty`. Until then images
  never appear.
* Unify `RendererState` with `ghostty-termio::state` (INTEGRATION_TODO): the canonical type is now `ghostty_renderer::state::State`/`SharedState`.
* `ghostty_font::shared_grid::CellSize` still stands in for `renderer::CellSize` (converted by hand at the call site).
* apprt: `surface::Scrollbar` / `RendererHealth` stand-ins should be replaced by `ghostty_terminal::page_list::Scrollbar` and `ghostty_renderer::Health`
  (what `Host` uses).
* apprt-gtk: GLArea render handler must set the GL viewport (the renderer reads `GL_VIEWPORT` for the surface size, R1) and the
  present blit goes to the framebuffer bound as draw FBO when `draw_frame` is called (GLArea binds its own FBO before `render`).
* Inspector (Phase 7) fills `State::overlay_features`; `Message::Inspector` only records a flag.

## Shutdown mailbox lifetime

`Handle::stop` closes the renderer's bounded mailbox before signaling stop.
Core surface teardown closes it even earlier, before joining any worker, so
an IO resize or search callback cannot wait for a stopped renderer to drain.
The underlying `BlockingQueue::close` broadcasts to all producers and rejects
future sends; see `ghostty-core.md` and `DEVIANT_FIXES.md` row 43.
