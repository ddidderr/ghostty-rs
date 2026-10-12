# macOS M1 — renderer (Metal backend + GraphicsApi abstraction)

Scope: `crates/ghostty-renderer` only. Zig: `renderer/Metal.zig`, `renderer/metal/*`, the macOS parts of
`generic.zig` / `Thread.zig` / `renderer.zig` / `shadertoy.zig`.

## GraphicsApi abstraction (`graphics_api.rs`)
Zig's `GenericRenderer(comptime GraphicsAPI)` duck-types its API; Rust has the `GraphicsApi` trait with
associated types (`Target`, `Texture`, `Sampler`, `Buffer<T>` (GAT), `BufferRef`, `Pipeline`, `Shaders`, `GpuUniforms`,
`Frame`, `RenderPass<'a>`), associated consts (`CUSTOM_SHADER_TARGET`, `CUSTOM_SHADER_Y_IS_DOWN`, `SWAP_CHAIN_COUNT`) and
operations (`init_*`, `begin_frame`, `surface_size`, `present_last_target`, `draw_frame_start/end`, `loop_enter/exit`, ...).
Helper traits: `GpuBuffer`, `GpuTexture`, `GpuTarget`, `GpuShaders`, `GpuFrame`, `GpuRenderPass`; shared value types
`Attachment`, `AttachmentTarget`, `Draw`, `Step`, `Primitive`, `ImageTextureFormat`, `TextureKind`, `ApiError`.
* `generic::Renderer<G: GraphicsApi = OpenGL>`, `image::{State,Image,Entry}<G>`, `thread::Thread<G = Api>`.
* `lib.rs`: `Api = metal::Metal` on `target_vendor = "apple"`, `opengl::OpenGL` otherwise; `Renderer = generic::Renderer<Api>`
  (`Backend::default_for_target` agrees, tested).
* The OpenGL backend implements the trait in `opengl/api.rs` by delegating to its (unchanged) inherent API, so apprt-gtk
  and `tests/egl_headless.rs` keep using the GL-typed API. `opengl::render_pass::{Attachment,Step,Draw}` are now aliases of the
  generic types; `gl::Primitive` re-exports `graphics_api::Primitive`.
* CPU-side GPU structs shared by every API moved to `shader_types.rs` (`Uniforms` keeps the std140 layout with bit-packed `bools`;
  Metal converts via `GraphicsApi::gpu_uniforms` to `MetalUniforms`, whose bools are four bytes like the MSL struct).
* Behaviour changes needed for asynchronous (Metal) frames, equal to Zig's `frame_sema`/`frameCompleted`:
  the swap chain hands frame states back from the frame completion callback (`SwapChainInner` + `Condvar`);
  `begin_frame(&self)` no longer borrows the API; `GpuFrame::complete(sync, completion)` reports health through a
  `FrameCompletion` (inline for GL / sync Metal, on Metal's completion thread otherwise); `display_unrealized` waits for
  in-flight frames; font grid changes bump an `atlas_epoch` so in-flight frames resync atlases.
* `Renderer::display_realized<F>(loader)` / `last_frame_pixels` exist only for `Renderer<OpenGL>`; the generic hook is
  `display_realized_with(|api| ..)`.
* GL: `OpenGL::last_target` is now `Arc<Mutex<Option<Arc<Target>>>>` (frames present without borrowing the API); `present`,
  `present_last_target`, `clear_last_target`, `begin_frame` take `&self`; `last_target()` returns a clone.

## Metal (`src/metal/`, `cfg(target_vendor = "apple")`, objc2 0.6 + objc2-metal 0.3)
| Zig | Rust |
|---|---|
| Metal.zig | `metal/mod.rs` (`Metal`, `ViewInfo`, `Metal::init(blending, &ViewInfo)`) |
| metal/api.zig | `metal_layout.rs` (raw values, plain Rust, unit tested) + `objc2_metal` types |
| metal/buffer.zig | `metal/buffer.rs` |
| metal/Frame.zig | `metal/frame.rs` (command buffer, completion block via `block2::RcBlock`) |
| metal/IOSurfaceLayer.zig | `metal/io_surface_layer.rs` (`define_class!` CALayer subclass `IOSurfaceLayer`: `display`, `actionForKey:`; main-thread `set_surface` via `dispatch2`) |
| metal/Pipeline.zig | `metal/pipeline.rs` (vertex attributes from `MetalVertex::ATTRIBUTES` instead of reflection) |
| metal/RenderPass.zig | `metal/render_pass.rs` (buffer slots: vertex 0, uniforms 1, rest from 2) |
| metal/Sampler.zig, Target.zig, Texture.zig | `metal/{sampler,target,texture}.rs` (Target = IOSurface BGRA + Display P3 colorspace + MTLTexture) |
| metal/shaders.zig | `metal/shaders.rs` (+ `shader_types`, `metal_layout::PIPELINE_DESCS`) |
| shaders/shaders.metal | `shaders/shaders.metal` (verbatim) |
* The metallib: Zig precompiles `shaders.metal` with `xcrun metal`; we embed the source and compile it with
  `newLibraryWithSource` at `Shaders::init` (same as post-process shaders already do). Entry points are looked up by name.
* View attachment is done with `msg_send!` on the `NSView`/`UIView` (no AppKit dependency): macOS `setLayer:` then
  `setWantsLayer:`, iOS `addSublayer:`. `setClipsToBounds:` is guarded with `respondsToSelector:` (NSView API since macOS 14).
* `Metal::init` expects to run on the main thread with the view the apprt (L1) provides; the apprt builds `Metal` and passes it
  to `Renderer::new(opts, api)`; `Thread::draw_on_app_thread` defaults to false on Apple targets.
* iOS: compiled via `target_vendor = "apple"` with the UIView branch; not checked here (only macOS targets are installed).

## macOS parts of generic / thread
* `display_link.rs`: `CVDisplayLink` wrapper (`objc2-core-video`, macOS only). On other platforms `DisplayLink` is uninhabited
  and the renderer holds `None` (Zig: `DisplayLink = void`). `Renderer` creates it when `window-vsync` is set (macOS),
  `loop_enter(draw_now)` installs the callback and starts it, `loop_exit` stops, `set_focus`/`set_visible` start/stop it,
  `set_macos_display_id`, `has_vsync` (= link running).
* `loop_enter` also sets the Metal layer's display callback to `draw_frame(true)` (holding a `Weak<Renderer>`).
* `thread.rs`: `pthread_setname_np("renderer")`, QoS class (`qos_for(visible, focused)`, tested on every platform),
  autorelease pool around `drain_mailbox`, `Message::MacosDisplayId`, `thread_enter/exit`, `loop_enter/exit` guards.
* `generic.rs`: with `background-blur = macos-glass-*` the background alpha is forced to 0 (macOS only).
  `Renderer::{thread_enter,thread_exit,loop_enter,loop_exit,set_macos_display_id}` are new.

## Shadertoy MSL
`shadertoy::msl_from_compiled` (naga `msl-out`, enabled on all platforms so it is tested on Linux): entry point renamed to
`ghostty_main` (spirv-cross' `main0` becomes `main0_` in naga, which suffixes names ending in a digit), `Globals` at
`[[buffer(1)]]`, `iChannel0` at `[[texture(0)]]` / `[[sampler(0)]]` (what the Metal render pass binds), MSL 2.1.
`Target::Msl` no longer returns `Unsupported`. Known limitation: the unused `iChannelTime` float array has std140 stride 16
in the CPU struct but naga emits tightly packed `float[4]` (spirv-cross emits padded types); no Ghostty shader uses it.

## Tests
* Linux (run): `metal_layout` (resource options / texture usage bits, storage mode, max texture size, surface size clamp,
  pixel formats, vertex attribute tables, buffer slots, blend state, `MetalUniforms` offsets == MSL struct, conversion from the
  std140 uniforms, shader function names exist in `shaders.metal`), shadertoy `shadertoy_to_msl` (port of "shadertoy to msl",
  +focus shader, +file loading), `thread::qos_class_policy`, `backend` tests, `tests/mock_api.rs` (generic renderer over a mock
  API with asynchronous triple-buffered completion, health reporting, unrealize waiting for in-flight frames).
  Totals: 97 unit + 9 EGL + 2 mock_api, all passing; the 9 EGL tests are unchanged apart from dropping an unneeded `mut`.
* macOS (compile-checked only): `metal/target.rs`, `render_pass.rs`, `frame.rs` unit tests (pixel format fourcc, primitive
  mapping, health from command buffer status) compile with `--all-targets`.
* Not ported: Zig has no tests in the Metal files other than `shadertoy to msl` (ported) and the `CellText` size test (covered
  by `shader_types` layout tests).

## Deviations / notes
* `Buffer::sync` (Metal) allocates exactly double the required bytes; Zig multiplies by `@sizeOf(T)` again.
* `Texture::replace_region` rejects too-short data (safe fn), Zig does not check.
* `Image` vertex stays 48 bytes (GL layout padded to 16); Metal's stride is `size_of` of that struct, offsets unchanged.
* `ghostty-core`'s `surface/mod.rs` imports `ghostty_renderer::Renderer` (the platform alias) but builds an `OpenGL` API (GTK);
  on macOS that alias is `Renderer<Metal>`. Until L1 provides the embedded surface, core should import
  `ghostty_renderer::generic::Renderer` (= `Renderer<OpenGL>`, identical type on Linux) so `cargo check --target *-apple-darwin
  -p ghostty-core` passes. `Thread` is generic over the API for this reason.
* Cross-check recipe (no macOS SDK): `GETTEXT_DIR=/nonexistent` (skip gettext-sys source build), `CC_<target>` = a wrapper
  around `zig cc -target aarch64-macos` for `onig_sys` (drop `-arch`/`--target`/`-mmacosx-version-min`), `CARGO_BUILD_RUSTC_WRAPPER=`
  (kache is picked up as cc wrapper), `ZIG_GLOBAL_CACHE_DIR` on a writable fs.
