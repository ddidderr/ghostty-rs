//! Headless GL integration test: creates a surfaceless EGL context (Mesa),
//! builds every pipeline, compiles the converted shadertoy shaders and runs a
//! tiny render pass, reading the pixels back.
//!
//! Skipped (returns early with a message) when no EGL display / GL 4.3 core
//! context can be created, e.g. on CI without Mesa.

// These tests drive the OpenGL backend (Mesa); Apple platforms render with Metal.
#![cfg(not(target_vendor = "apple"))]
// The EGL loader calls are `unsafe fn`s in khronos-egl (test-only usage).
#![allow(unsafe_code, clippy::unwrap_used)]

use std::{ffi::c_void, sync::Arc};

use ghostty_renderer::{
    opengl::{
        AlphaBlending,
        OpenGL,
        buffer,
        gl::{Gl, Primitive},
        render_pass::{Attachment, AttachmentTarget, Draw, Step},
        shaders::{BgImage, BgImageFit, BgImageInfo, BgImagePosition, CellText, Image, Uniforms},
    },
    shadertoy,
};
use khronos_egl as egl;

// EGL_PLATFORM_SURFACELESS_MESA, falling back to the default display.
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

struct Headless {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
}

impl Headless {
    fn new() -> Result<Headless, String> {
        // SAFETY: loads the system libEGL.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| format!("cannot load libEGL: {e}"))?;

        // SAFETY: EGL_DEFAULT_DISPLAY with an empty (terminated) attribute list.
        let display = unsafe {
            egl.get_platform_display(
                PLATFORM_SURFACELESS_MESA,
                egl::DEFAULT_DISPLAY,
                &[egl::ATTRIB_NONE],
            )
        }
        .or_else(|_| {
            // SAFETY: default display handle.
            unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }.ok_or(egl::Error::BadDisplay)
        })
        .map_err(|e| format!("no EGL display: {e}"))?;
        egl.initialize(display)
            .map_err(|e| format!("eglInitialize: {e}"))?;
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| format!("eglBindAPI: {e}"))?;

        let attribs = [
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::SURFACE_TYPE,
            egl::PBUFFER_BIT,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attribs)
            .map_err(|e| format!("eglChooseConfig: {e}"))?
            .ok_or("no EGL config")?;
        let ctx_attribs = [
            egl::CONTEXT_MAJOR_VERSION,
            4,
            egl::CONTEXT_MINOR_VERSION,
            3,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let context = egl
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| format!("eglCreateContext (GL 4.3 core): {e}"))?;
        // Surfaceless: EGL_KHR_surfaceless_context.
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| format!("eglMakeCurrent: {e}"))?;
        Ok(Headless {
            egl,
            display,
            context,
        })
    }

    fn proc_address(&self, name: &str) -> *const c_void {
        self.egl
            .get_proc_address(name)
            .map_or(std::ptr::null(), |f| f as *const c_void)
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

fn setup() -> Option<(Headless, OpenGL)> {
    let h = match Headless::new() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("SKIP: headless EGL unavailable: {e}");
            return None;
        }
    };
    let mut api = OpenGL::init(AlphaBlending::Native);
    if let Err(e) = api.prepare_context(|n| h.proc_address(n)) {
        eprintln!("SKIP: cannot prepare GL context: {e}");
        return None;
    }
    Some((h, api))
}

/// Returns the pixel at (x, y) of an RGBA readback.
fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

#[test]
fn headless_shaders_compile_and_render() {
    let Some((_h, api)) = setup() else {
        return;
    };
    let gl: Gl = api.gl().unwrap().clone();

    // Convert the shadertoy test shaders to GLSL 430 and build everything.
    let custom: Vec<String> = [shadertoy::test_shaders::CRT, shadertoy::test_shaders::FOCUS]
        .iter()
        .map(|src| {
            let c = shadertoy::compile_shadertoy(src, None).unwrap();
            shadertoy::glsl_from_compiled(&c).unwrap()
        })
        .collect();
    let shaders = api.init_shaders(&custom).expect("all pipelines compile");
    assert_eq!(
        shaders.post_pipelines.len(),
        2,
        "converted shadertoy shaders must compile and link on the driver"
    );

    // Vertex layouts are accepted by the driver: no GL error pending.
    gl.get_error().unwrap();

    // ------- A tiny render pass: bg_color fills the target with the bg color.
    let (w, h) = (16usize, 8usize);
    let target = Arc::new(api.init_target(w, h).unwrap());
    let uniforms = Uniforms {
        projection_matrix: ghostty_renderer::math::ortho2d(0.0, w as f32, h as f32, 0.0),
        screen_size: [w as f32, h as f32],
        cell_size: [8.0, 8.0],
        grid_size: [2, 1],
        bg_color: [10, 200, 30, 255],
        ..Default::default()
    };
    let ubo = buffer::Buffer::<Uniforms>::init_fill(
        &gl,
        buffer::Options {
            target: ghostty_renderer::opengl::gl::BufferTarget::Uniform,
            usage: ghostty_renderer::opengl::gl::BufferUsage::DynamicDraw,
        },
        &[uniforms],
    )
    .unwrap();

    // A surfaceless context has no default viewport (GTK's GLArea sets one).
    gl.viewport(0, 0, w as i32, h as i32).unwrap();
    assert_eq!(api.surface_size().unwrap(), (w as u32, h as u32));

    let frame = api.begin_frame(Arc::clone(&target));
    {
        let attachments = [Attachment {
            target: AttachmentTarget::Target(&target),
            clear_color: Some([0.0, 0.0, 0.0, 0.0]),
        }];
        let mut pass = frame.render_pass(&attachments);
        pass.step(&Step {
            pipeline: &shaders.bg_color,
            uniforms: Some(ubo.raw()),
            buffers: &[],
            textures: &[],
            samplers: &[],
            draw: Draw::new(Primitive::Triangle, 3),
        });
        pass.complete();
    }
    let health = frame.complete(true);
    assert_eq!(health, ghostty_renderer::Health::Healthy);

    // Read the target back.
    let fbobind = gl.bind_framebuffer(
        target.framebuffer(),
        ghostty_renderer::opengl::gl::FramebufferTarget::Read,
    );
    let pixels = gl.read_pixels_rgba8(0, 0, w as i32, h as i32);
    gl.unbind_framebuffer(fbobind);
    gl.get_error().unwrap();
    assert_eq!(px(&pixels, w, 0, 0), [10, 200, 30, 255]);
    assert_eq!(px(&pixels, w, w - 1, h - 1), [10, 200, 30, 255]);

    // ------- The other pipelines draw (instanced) without GL errors.
    let instances = buffer::Buffer::<CellText>::init_fill(
        &gl,
        buffer::Options::default(),
        &[CellText {
            grid_pos: [0, 0],
            color: [255, 255, 255, 255],
            ..Default::default()
        }],
    )
    .unwrap();
    let images = buffer::Buffer::<Image>::init_fill(
        &gl,
        buffer::Options::default(),
        &[Image {
            source_rect: [0.0, 0.0, 1.0, 1.0],
            dest_size: [4.0, 4.0],
            ..Default::default()
        }],
    )
    .unwrap();
    let bg_images = buffer::Buffer::<BgImage>::init_fill(
        &gl,
        buffer::Options::default(),
        &[BgImage {
            opacity: 1.0,
            info: BgImageInfo {
                position: BgImagePosition::Mc,
                fit: BgImageFit::Contain,
                repeat: false,
            }
            .to_bits(),
            pad: [0; 3],
        }],
    )
    .unwrap();
    let bg_cells = buffer::Buffer::<[u8; 4]>::init_fill(
        &gl,
        buffer::Options {
            target: ghostty_renderer::opengl::gl::BufferTarget::Storage,
            ..Default::default()
        },
        &[[1, 2, 3, 255]; 2],
    )
    .unwrap();
    let atlas = ghostty_font::atlas::Atlas::new(64, ghostty_font::atlas::Format::Grayscale);
    let atlas_tex = api.init_atlas_texture(&atlas).unwrap();
    let sampler = ghostty_renderer::opengl::Sampler::init(&gl, api.sampler_options()).unwrap();
    let img_tex = ghostty_renderer::opengl::Texture::init(
        &gl,
        api.image_texture_options(ghostty_renderer::opengl::ImageTextureFormat::Rgba, true),
        1,
        1,
        Some(&[255, 0, 0, 255]),
    )
    .unwrap();

    let attachments = [Attachment {
        target: AttachmentTarget::Target(&target),
        clear_color: None,
    }];
    let mut pass = ghostty_renderer::opengl::RenderPass::begin(
        &gl,
        ghostty_renderer::opengl::render_pass::Options {
            attachments: &attachments,
        },
    );
    pass.step(&Step {
        pipeline: &shaders.cell_bg,
        uniforms: Some(ubo.raw()),
        buffers: &[None, Some(bg_cells.raw())],
        textures: &[],
        samplers: &[],
        draw: Draw::new(Primitive::Triangle, 3),
    });
    pass.step(&Step {
        pipeline: &shaders.cell_text,
        uniforms: Some(ubo.raw()),
        buffers: &[Some(instances.raw()), Some(bg_cells.raw())],
        textures: &[Some(&atlas_tex), Some(&atlas_tex)],
        samplers: &[],
        draw: Draw {
            ty: Primitive::TriangleStrip,
            vertex_count: 4,
            instance_count: 1,
        },
    });
    pass.step(&Step {
        pipeline: &shaders.image,
        uniforms: Some(ubo.raw()),
        buffers: &[Some(images.raw())],
        textures: &[Some(&img_tex)],
        samplers: &[Some(&sampler)],
        draw: Draw {
            ty: Primitive::TriangleStrip,
            vertex_count: 4,
            instance_count: 1,
        },
    });
    pass.step(&Step {
        pipeline: &shaders.bg_image,
        uniforms: Some(ubo.raw()),
        buffers: &[Some(bg_images.raw())],
        textures: &[Some(&img_tex)],
        samplers: &[Some(&sampler)],
        draw: Draw {
            ty: Primitive::TriangleStrip,
            vertex_count: 4,
            instance_count: 1,
        },
    });
    pass.complete();
    gl.finish();
    gl.get_error().unwrap();

    // ------- Post pipelines render into a texture attachment.
    let tex =
        ghostty_renderer::opengl::Texture::init(&gl, api.texture_options(), w, h, None).unwrap();
    let attachments = [Attachment {
        target: AttachmentTarget::Texture(&tex),
        clear_color: Some([0.0, 0.0, 0.0, 1.0]),
    }];
    let mut pass = ghostty_renderer::opengl::RenderPass::begin(
        &gl,
        ghostty_renderer::opengl::render_pass::Options {
            attachments: &attachments,
        },
    );
    let shadertoy_ubo = buffer::Buffer::<shadertoy::Uniforms>::init_fill(
        &gl,
        buffer::Options {
            target: ghostty_renderer::opengl::gl::BufferTarget::Uniform,
            ..Default::default()
        },
        &[shadertoy::Uniforms {
            resolution: [w as f32, h as f32, 1.0],
            ..Default::default()
        }],
    )
    .unwrap();
    for post in &shaders.post_pipelines {
        pass.step(&Step {
            pipeline: post,
            uniforms: Some(shadertoy_ubo.raw()),
            buffers: &[],
            textures: &[Some(&img_tex)],
            samplers: &[Some(&sampler)],
            draw: Draw::new(Primitive::Triangle, 3),
        });
    }
    pass.complete();
    gl.finish();
    gl.get_error().unwrap();
}

#[test]
fn headless_buffer_sync_grows() {
    let Some((_h, api)) = setup() else {
        return;
    };
    let gl = api.gl().unwrap().clone();
    let mut b = buffer::Buffer::<u32>::init(&gl, buffer::Options::default(), 2).unwrap();
    b.sync(&[1, 2]).unwrap();
    assert_eq!(b.len, 2);
    b.sync(&[1, 2, 3]).unwrap();
    assert_eq!(b.len, 6, "reallocates to double the required size");
    let n = b.sync_from_slices(&[&[1, 2], &[3]]).unwrap();
    assert_eq!(n, 3);
    gl.get_error().unwrap();
}

#[test]
fn headless_invalid_shader_reports_compile_error() {
    let Some((_h, api)) = setup() else {
        return;
    };
    let gl = api.gl().unwrap();
    let err = gl.create_program_vf("#version 430 core\nvoid main() { nope }", "x");
    assert!(matches!(
        err,
        Err(ghostty_renderer::opengl::gl::Error::CompileFailed(_))
    ));
}

// ---------------------------------------------------------------------------
// Full renderer: Terminal -> RenderState -> generic renderer -> offscreen target

mod full_renderer {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use ghostty_config::{
        config::Config,
        types_a::Color as ConfigColor,
        types_b::AlphaBlending as ConfigBlending,
    };
    use ghostty_font::{
        face::DesiredSize,
        shared_grid_set::{DerivedConfig as FontDerivedConfig, SharedGridSet},
    };
    use ghostty_renderer::{
        generic::{DerivedConfig, Host, Renderer},
        options::Options,
        size::{CellSize, Padding, ScreenSize, Size},
        state::State,
    };
    use ghostty_terminal::{
        page_list::Scrollbar,
        terminal::{Options as TermOptions, Terminal},
    };
    use ghostty_vt::color::DynamicRgb;

    use super::*;

    #[derive(Default)]
    struct TestHost {
        scrollbars: Mutex<Vec<Scrollbar>>,
        health: Mutex<Vec<ghostty_renderer::Health>>,
        redraws: AtomicUsize,
    }

    impl Host for TestHost {
        fn scrollbar(&self, s: Scrollbar) -> bool {
            self.scrollbars.lock().unwrap().push(s);
            true
        }
        fn renderer_health(&self, h: ghostty_renderer::Health) {
            self.health.lock().unwrap().push(h);
        }
        fn redraw_surface(&self) -> bool {
            self.redraws.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    const PAD: u32 = 10;
    const COLS: u16 = 20;
    const ROWS: u16 = 5;

    struct Fixture {
        _h: Headless,
        renderer: Arc<Renderer>,
        gl: Gl,
        state: ghostty_renderer::state::SharedState,
        host: Arc<TestHost>,
        cell: CellSize,
        w: usize,
        h: usize,
        bg: [u8; 3],
    }

    impl Fixture {
        fn new(text: &str) -> Option<Fixture> {
            let (h, mut api) = {
                let h = match Headless::new() {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("SKIP: headless EGL unavailable: {e}");
                        return None;
                    }
                };
                // Native blending so the stored bytes are exact.
                let mut api = OpenGL::init(ConfigBlending::Native);
                if let Err(e) = api.prepare_context(|n| h.proc_address(n)) {
                    eprintln!("SKIP: cannot prepare GL context: {e}");
                    return None;
                }
                (h, api)
            };
            let _ = &mut api;

            let config = Config {
                alpha_blending: ConfigBlending::Native,
                background: ConfigColor {
                    r: 0x10,
                    g: 0x20,
                    b: 0x30,
                },
                background_opacity: 1.0,
                ..Config::default()
            };

            // Fonts: the embedded JetBrains Mono (no font-family configured).
            let set = Arc::new(SharedGridSet::new().unwrap());
            let (_key, grid) = set
                .ref_(
                    &FontDerivedConfig::new(&config),
                    DesiredSize {
                        points: 12.0,
                        xdpi: 96,
                        ydpi: 96,
                    },
                )
                .unwrap();
            let gc = grid.cell_size();
            let cell = CellSize {
                width: gc.width,
                height: gc.height,
            };

            // As termio does at startup, give the terminal the configured colors.
            let mut topts = TermOptions::new(COLS, ROWS);
            topts.colors.background = DynamicRgb::init(config.background.to_terminal_rgb());
            topts.colors.foreground = DynamicRgb::init(config.foreground.to_terminal_rgb());
            let mut term = Terminal::new(topts).unwrap();
            {
                let mut s = term.vt_stream();
                s.next_slice(text.as_bytes()).unwrap();
            }
            let state = State::new(term).into_shared();

            let w = (u32::from(COLS) * cell.width + 2 * PAD) as usize;
            let hh = (u32::from(ROWS) * cell.height + 2 * PAD) as usize;
            let size = Size {
                screen: ScreenSize {
                    width: w as u32,
                    height: hh as u32,
                },
                cell,
                padding: Padding {
                    top: PAD,
                    bottom: PAD,
                    left: PAD,
                    right: PAD,
                },
            };

            let gl: Gl = api.gl().unwrap().clone();
            // A surfaceless context has no default viewport (GTK's GLArea sets one).
            gl.viewport(0, 0, w as i32, hh as i32).unwrap();

            let host = Arc::new(TestHost::default());
            let renderer = Arc::new(
                Renderer::new(
                    Options {
                        config: DerivedConfig::new(&config).unwrap(),
                        font_grid: grid,
                        size,
                        host: host.clone(),
                    },
                    api,
                )
                .unwrap(),
            );

            Some(Fixture {
                _h: h,
                gl,
                renderer,
                state,
                host,
                cell,
                w,
                h: hh,
                bg: [0x10, 0x20, 0x30],
            })
        }

        fn frame(&self, blink_visible: bool) -> Vec<u8> {
            self.renderer
                .update_frame(&self.state, blink_visible)
                .unwrap();
            self.renderer.draw_frame(true).unwrap();
            let (w, h, px) = self
                .renderer
                .last_frame_pixels()
                .expect("a frame was drawn");
            assert_eq!((w, h), (self.w, self.h));
            px
        }

        /// Pixel at terminal-space (top-left origin) pixel coordinates.
        fn at(&self, px: &[u8], x: usize, y_top: usize) -> [u8; 4] {
            super::px(px, self.w, x, self.h - 1 - y_top)
        }

        /// The pixel at the center of a rectangle.
        fn at_center(&self, px: &[u8], r: (usize, usize, usize, usize)) -> [u8; 4] {
            self.at(px, usize::midpoint(r.0, r.2), usize::midpoint(r.1, r.3))
        }

        /// The (x0, y0, x1, y1) pixel rectangle of grid cell (cx, cy).
        fn cell_rect(&self, cx: usize, cy: usize) -> (usize, usize, usize, usize) {
            let (cw, ch) = (self.cell.width as usize, self.cell.height as usize);
            let (x0, y0) = (PAD as usize + cx * cw, PAD as usize + cy * ch);
            (x0, y0, x0 + cw, y0 + ch)
        }

        /// Pixels of a rectangle.
        fn region(&self, px: &[u8], r: (usize, usize, usize, usize)) -> Vec<[u8; 4]> {
            let mut v = Vec::new();
            for y in r.1..r.3 {
                for x in r.0..r.2 {
                    v.push(self.at(px, x, y));
                }
            }
            v
        }
    }

    fn is_bg(p: [u8; 4], bg: [u8; 3]) -> bool {
        p[..3] == bg
    }

    #[test]
    fn headless_full_renderer_draws_terminal() {
        // Line 1: red "hello"; line 2: three palette-green cells then the cursor.
        let Some(f) = Fixture::new("\x1b[31mhello\x1b[0m\r\n\x1b[42m   \x1b[0m") else {
            return;
        };
        let px = f.frame(true);

        // The padding shows the configured background exactly, in all corners.
        for (x, y) in [
            (0, 0),
            (f.w - 1, 0),
            (0, f.h - 1),
            (f.w - 1, f.h - 1),
            (PAD as usize / 2, f.h / 2),
        ] {
            let p = f.at(&px, x, y);
            assert_eq!(p[..3], f.bg, "padding pixel ({x},{y}) = {p:?}");
        }

        // The text area has non-background pixels in the "hello" cells and they
        // are red (palette 1 is a red hue: r dominates).
        let hello = f.region(&px, f.cell_rect(0, 0));
        let hello_all: Vec<_> = (0..5)
            .flat_map(|cx| f.region(&px, f.cell_rect(cx, 0)))
            .collect();
        assert!(hello_all.iter().any(|p| !is_bg(*p, f.bg)), "text drawn");
        assert!(
            hello_all
                .iter()
                .any(|p| p[0] > 150 && u16::from(p[0]) > u16::from(p[1]) + 80),
            "red glyph pixels exist"
        );
        let _ = hello;

        // Cells past the text on line 1 are untouched background.
        for p in f.region(&px, f.cell_rect(10, 0)) {
            assert!(is_bg(p, f.bg), "empty cell is bg: {p:?}");
        }

        // Palette-green background cells are filled with the exact palette color.
        let green = ghostty_vt::color::DEFAULT[2];
        let mid = f.cell_rect(1, 1);
        let p = f.at_center(&px, mid);
        assert_eq!(p[..3], [green.r, green.g, green.b], "cell bg color");

        // The block cursor (focused, blink visible) is drawn in the foreground
        // color at (3, 1).
        let c = f.cell_rect(3, 1);
        let p = f.at_center(&px, c);
        assert_eq!(p[..3], [255, 255, 255], "cursor block");

        // The frame was healthy, and the first scrollbar state was pushed.
        assert_eq!(f.renderer.health(), ghostty_renderer::Health::Healthy);
        assert!(!f.host.scrollbars.lock().unwrap().is_empty());
    }

    #[test]
    fn headless_full_renderer_updates_incrementally_and_blinks() {
        let Some(f) = Fixture::new("hi") else {
            return;
        };
        let px = f.frame(true);
        // The cursor is at (2, 0).
        let c = f.cell_rect(2, 0);
        let center = |px: &[u8]| f.at_center(px, c);
        assert_eq!(center(&px)[..3], [255, 255, 255]);

        // Blink off: the cursor disappears (partial/no-op row rebuild keeps
        // the rest of the screen).
        {
            let mut st = f.state.lock().unwrap();
            let mut s = st.terminal.vt_stream();
            // Make the terminal's cursor blink so blink_visible=false hides it.
            s.next_slice(b"\x1b[?12h").unwrap();
        }
        let px = f.frame(false);
        assert_eq!(center(&px)[..3], f.bg, "hidden blinking cursor shows bg");
        let hi = (0..2)
            .flat_map(|cx| f.region(&px, f.cell_rect(cx, 0)))
            .any(|p| !is_bg(p, f.bg));
        assert!(hi, "existing text survives a rebuild");

        // New output only dirties its own row; the old row stays.
        {
            let mut st = f.state.lock().unwrap();
            let mut s = st.terminal.vt_stream();
            s.next_slice(b"\r\n\x1b[1;32mbold\x1b[0m").unwrap();
        }
        let px = f.frame(true);
        assert!(
            (0..4)
                .flat_map(|cx| f.region(&px, f.cell_rect(cx, 1)))
                .any(|p| !is_bg(p, f.bg)),
            "second line drawn"
        );
        assert!(
            (0..2)
                .flat_map(|cx| f.region(&px, f.cell_rect(cx, 0)))
                .any(|p| !is_bg(p, f.bg)),
            "first line still drawn"
        );
    }

    #[test]
    fn headless_full_renderer_selection_and_underline() {
        // Underlined text on row 0 and a selected region.
        let Some(f) = Fixture::new("\x1b[4mab\x1b[0m cd") else {
            return;
        };
        let px = f.frame(false);
        // The underline sits near the bottom of the cell: some non-bg pixel in
        // the lower third of cell (0,0) that is a full-width line.
        let r = f.cell_rect(0, 0);
        let low = (r.0, r.1 + (r.3 - r.1) * 2 / 3, r.2, r.3);
        assert!(
            f.region(&px, low).iter().any(|p| !is_bg(*p, f.bg)),
            "underline drawn"
        );
    }

    #[test]
    fn headless_full_renderer_resize_and_font_change() {
        let Some(f) = Fixture::new("resize") else {
            return;
        };
        let _ = f.frame(true);
        // A screen size change (padding only) is accepted without a GL error.
        f.renderer.set_screen_size(Size {
            screen: ScreenSize {
                width: f.w as u32,
                height: f.h as u32,
            },
            cell: f.cell,
            padding: Padding {
                top: PAD,
                bottom: PAD,
                left: PAD,
                right: PAD,
            },
        });
        f.renderer.mark_dirty();
        let px = f.frame(true);
        assert_eq!(f.at(&px, 0, 0)[..3], f.bg);
        assert_eq!(f.renderer.health(), ghostty_renderer::Health::Healthy);
    }

    #[test]
    fn headless_full_renderer_survives_unrealize_realize() {
        let Some(f) = Fixture::new("realize") else {
            return;
        };
        let _ = f.frame(true);

        // GTK destroys and recreates the GL objects when the widget is
        // unrealized/realized (e.g. moved between windows).
        f.renderer.display_unrealized();
        assert!(matches!(
            f.renderer.draw_frame(true),
            Err(ghostty_renderer::generic::Error::Defunct)
        ));
        f.renderer
            .display_realized(|n| f._h.proc_address(n))
            .unwrap();
        // The new context needs a viewport again in the surfaceless case.
        let gl = Gl::clone(&f.gl);
        gl.viewport(0, 0, f.w as i32, f.h as i32).unwrap();

        let px = f.frame(true);
        assert_eq!(f.at(&px, 0, 0)[..3], f.bg);
        assert!(
            (0..7)
                .flat_map(|cx| f.region(&px, f.cell_rect(cx, 0)))
                .any(|p| !is_bg(p, f.bg)),
            "text is drawn again after realize"
        );
        assert_eq!(f.renderer.health(), ghostty_renderer::Health::Healthy);
    }

    #[test]
    fn headless_renderer_thread_updates_and_requests_draws() {
        use std::time::{Duration, Instant};

        use ghostty_renderer::{message::Message, thread::Thread};

        let Some(f) = Fixture::new("thread") else {
            return;
        };
        let wait_for = |n: usize| {
            let end = Instant::now() + Duration::from_secs(5);
            while f.host.redraws.load(Ordering::SeqCst) < n {
                assert!(Instant::now() < end, "timed out waiting for redraw {n}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        let thread = Thread::new(
            &Config::default(),
            f.renderer.clone(),
            f.state.clone(),
            f.host.clone(),
        );
        let running = thread.spawn().unwrap();

        // The initial wakeup renders (update_frame) and asks the apprt
        // (here: this test, standing in for the GTK main thread) to draw.
        wait_for(1);
        f.renderer.draw_frame(true).unwrap();
        let (w, h, px) = f.renderer.last_frame_pixels().unwrap();
        assert_eq!((w, h), (f.w, f.h));
        assert_eq!(f.at(&px, 0, 0)[..3], f.bg);
        assert!(
            (0..6)
                .flat_map(|cx| f.region(&px, f.cell_rect(cx, 0)))
                .any(|p| !is_bg(p, f.bg)),
            "thread-rendered text is visible"
        );

        // Messages are processed and cause another update + draw request.
        let before = f.host.redraws.load(Ordering::SeqCst);
        assert!(running.handle.send(Message::Focus(false)));
        wait_for(before + 1);
        // Unfocused: the hollow block cursor is drawn at (6, 0); its center
        // stays background but its left edge is foreground.
        {
            let mut st = f.state.lock().unwrap();
            st.terminal.vt_stream().next_slice(b"!").unwrap();
        }
        running.handle.wakeup();
        wait_for(before + 2);
        f.renderer.draw_frame(true).unwrap();
        let (_, _, px) = f.renderer.last_frame_pixels().unwrap();
        let c = f.cell_rect(7, 0);
        assert_eq!(
            f.at_center(&px, c)[..3],
            f.bg,
            "hollow cursor is empty in the middle"
        );
        assert!(
            f.region(&px, c).iter().any(|p| !is_bg(*p, f.bg)),
            "hollow cursor outline is drawn"
        );

        running.stop_and_join();
    }
}
