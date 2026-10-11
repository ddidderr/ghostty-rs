# ghostty-renderer — R1 (OpenGL backend, shaders, shadertoy, size, cursor)

Crate lint: `unsafe_code = "deny"`; the only `#[allow(unsafe_code)]` item is `opengl::gl`
(thin safe wrapper over `glow`, every unsafe block has a `// SAFETY:` comment; the
module doc states the shared invariants). The rest of the crate is safe code.
Deps: `glow 0.18`, `naga 30` (features glsl-in, glsl-out, **wgsl-in** — see below),
`bytemuck` (derive), `ghostty-font`, `log`, `thiserror`; dev: `khronos-egl` (dynamic).
`ghostty-config` / `ghostty-terminal` deps removed.

## Module map (Zig -> Rust, `crates/ghostty-renderer/src/`)
| Zig | Rust |
|---|---|
| `pkg/opengl/*` (needed subset) | `opengl/gl.rs` (`Gl` = `Arc<glow::Context>` handle, GL enums, `Error`) |
| `opengl/buffer.zig` | `opengl/buffer.rs` (`Buffer<T: Pod>`, `Options`) |
| `opengl/Texture.zig`, `Sampler.zig`, `Target.zig` | `texture.rs`, `sampler.rs`, `target.rs` (RAII: `Drop` deletes the GL object) |
| `opengl/Pipeline.zig` | `pipeline.rs` (`Vertex` trait with `ATTRIBUTES` replaces comptime reflection) |
| `opengl/RenderPass.zig` | `render_pass.rs` (`RenderPass`, `Step`, `Draw`, `Attachment`) |
| `opengl/Frame.zig` | `frame.rs` (`Frame::complete(sync) -> Health`) |
| `opengl/shaders.zig` | `opengl/shaders.rs` (`Shaders`, `Uniforms`, `CellText`, `Image`, `BgImage`, `process_includes`) |
| `OpenGL.zig` | `opengl/mod.rs` (`OpenGL`) |
| `shaders/*.glsl` | `crates/ghostty-renderer/shaders/` (verbatim copies, `include_str!`) |
| `shadertoy.zig` | `shadertoy.rs` |
| `size.zig`, `cursor.zig`, `backend.zig` (+ `Health` from renderer.zig), `message.zig`, `Options.zig`, `math.zig` (ortho2d) | `size.rs`, `cursor.rs`, `backend.rs`, `message.rs`, `options.rs`, `math.rs` |

## API overview
```rust
// apprt (GTK GLArea realize, context current) supplies the loader:
let mut api = OpenGL::init(AlphaBlending::Native);               // or ::with_gl(blending, gl)
api.prepare_context(|name| gtk_get_proc_address(name))?;         // loads glow, checks >= 4.3,
                                                                  // enables DEBUG_OUTPUT + FRAMEBUFFER_SRGB
let shaders = api.init_shaders(&custom_glsl_strings)?;           // bg_color, cell_bg, cell_text, image, bg_image, post_pipelines
let target = Arc::new(api.init_target(w, h)?);                   // sRGB renderbuffer if blending.is_linear()
let frame = api.begin_frame(target.clone());
let mut pass = frame.render_pass(&[Attachment { target: AttachmentTarget::Target(&target), clear_color: Some([0.;4]) }]);
pass.step(&Step { pipeline: &shaders.cell_bg, uniforms: Some(ubo.raw()),
                  buffers: &[None, Some(bg_cells.raw())],        // [0] = vertex buffer, [1..] = SSBO at that index
                  textures: &[], samplers: &[], draw: Draw::new(Primitive::Triangle, 3) });
pass.complete();
let health = frame.complete(true);                               // glFinish, glGetError, present (blit to default FB)
```
* Uniform buffer is bound at binding 1, SSBOs at their slice index, textures/samplers at their slice index (same as Zig).
* GPU structs (`bytemuck::Pod`, layouts asserted in tests): `Uniforms` 144 B std140 (pack `bools`/`padding_extend`
  with `Bools::to_bits` / `PaddingExtend::to_bits`), `CellText` 32 B, `Image` 48 B, `BgImage` 8 B (`BgImageInfo::to_bits`),
  `shadertoy::Uniforms` 4496 B. Explicit `pad*` fields reproduce Zig `extern struct`/`align()` padding (set to 0; use `..Default::default()`).
* Atlas upload: `api.init_atlas_texture(&atlas)` creates the `GL_TEXTURE_RECTANGLE` texture (data `None`);
  upload with `Texture::replace_region(0,0,size,size,&atlas.data)` and track `atlas.modified/resized`.
* Textures: `texture_options()`, `image_texture_options(fmt, srgb)`, `sampler_options()`, `*_buffer_options()` mirror OpenGL.zig.
* Threading hooks: `surface_init(loader)`, `display_realized(loader)`, `finalize_surface_init`, `thread_enter/exit`, `draw_frame_start/end`
  (GTK semantics preserved: no-ops except context prepare). The GL context must be current on whichever thread calls GL.
* `present` keeps `Arc<Target>` as `last_target` (Zig copied the struct); call `clear_last_target()` when the target is replaced.
* Viewport is NOT set by the renderer (GLArea sets it; `surface_size()` reads GL_VIEWPORT) — a surfaceless EGL context has none.

## shadertoy (glslang + SPIR-V + spirv-cross replaced by naga)
`compile_shadertoy(user_src, errlog) -> Compiled` then `glsl_from_compiled(&Compiled) -> String` (GLSL 430 core), or
`load_from_file(s)` (4 MB limit, optional/required path list `PathItem`; `Target::Msl` -> `Error::Unsupported`).
`glsl_from_shader` still returns prefix + "\n\n" + src verbatim. Errors: `Error::GlslangFailed` (parse; `ShaderLog.info` has naga's
rendered diagnostics, warn-logged like Zig), `ValidationFailed`, `SpvcFailed`.
naga's GLSL frontend only takes a Vulkan-flavoured subset, so `adapt_for_naga` rewrites the **prefix** (the .glsl files are untouched):
`#version 430`->`460`; `uniform` dropped from block members; `layout(location=0) in vec4 gl_FragCoord;` removed (naga builtin);
`sampler2D iChannel0` -> `texture2D`+`sampler` pair with `#define iChannel0 sampler2D(tex, smp)` (so `texture(iChannel0, uv)` and
`texture2D` keep working); `layout(set=0,binding=1,std140)` on Globals; the `mainImage` prototype and `main()` move after the user code
(naga rejects forward declarations). Output keeps `layout(std140, binding = 1) uniform ...` and `layout(binding = 0) uniform sampler2D`,
so the Ghostty binding scheme works; verified by compiling+linking converted crt/focus shaders on Mesa. Limitations: user shaders that pass
`sampler2D` as a function parameter, or define helpers *after* `mainImage` that it calls (invalid GLSL anyway), will fail.
`naga` needs the `wgsl-in` feature only because naga 30.0.1's `front::interpolator` is cfg-gated on spv-in/wgsl-in (glsl-in alone does not compile).

## Tests
Unit 30 + integration 3 (`tests/egl_headless.rs`), all passing.
* size.zig 4/4 ported (`padding_balanced_on_zero`, `grid_size_update_exact`, `grid_size_update_rounding`, `coordinate_conversion`) + 3 added.
* cursor.zig 4/4 ported but drive a locally defined `RenderStateCursor` instead of Terminal/RenderState (TODO(integration)); + password/lock test.
* shadertoy.zig: `spirv`, `spirv invalid`, `shadertoy to glsl` ported (+ focus shader, file loading, helper-order tests);
  `shadertoy to msl` skipped (Metal out of scope; replaced by an Unsupported test).
* Layout tests: std140 offsets of `Uniforms`/`shadertoy::Uniforms`, `CellText`=32, `Image`=48, `BgImage`=8, bool bit values vs common.glsl, include processing.
* `tests/egl_headless.rs`: surfaceless Mesa EGL 4.3 core context (`khronos-egl` dynamic, `EGL_PLATFORM_SURFACELESS_MESA`);
  builds all 5 pipelines + 2 converted shadertoy post pipelines, renders bg_color into a `Target` and reads back the exact color,
  runs cell_bg/cell_text/image/bg_image/post passes with no GL error, buffer growth, shader compile-error reporting.
  If no EGL display/context is available the tests print `SKIP:` and return (they do run here).

## Deviations / notes
* `Buffer::init_fill`: Zig sets `len = data.len * sizeof(T)` (bug); we store the element count.
* `Pipeline` drop also deletes its FBO and VAO (Zig only destroyed the program).
* `Frame` no longer holds the renderer; `complete` returns `Health` and the caller calls `frameCompleted`.
* `shadertoy::Uniforms` field order follows Zig (`selection_background_color` before `selection_foreground_color`) while the GLSL
  block declares `iSelectionForegroundColor` before `iSelectionBackgroundColor` — upstream quirk preserved, flagged for review.
* Allocator-failure paths and comptime pipeline-collection type building are replaced by plain structs/`Result`.

## TODO(integration) / deferred
* Deferred per assignment: generic.zig, cell.zig, image.zig, link.zig, row.zig, State.zig, Thread.zig, Overlay.zig.
* `opengl::AlphaBlending` (config), `cursor::RenderStateCursor`/`TerminalCursorStyle` (terminal), `size::CellCountInt` (=u16),
  `shadertoy::PathItem` (config `RepeatablePath`), `message::{ThreadDerivedConfig, RendererDerivedConfig, Flattened, SharedGridKey}` stubs,
  `options::Options` lacks `surface_mailbox/rt_surface/thread`; `Message::init_change_config` takes pre-built derived configs.
* Metal/WebGL (`Backend::Metal/Webgl` variants exist only for parity with backend.zig).
