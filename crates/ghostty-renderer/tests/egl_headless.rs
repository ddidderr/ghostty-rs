//! Headless GL integration test: creates a surfaceless EGL context (Mesa),
//! builds every pipeline, compiles the converted shadertoy shaders and runs a
//! tiny render pass, reading the pixels back.
//!
//! Skipped (returns early with a message) when no EGL display / GL 4.3 core
//! context can be created, e.g. on CI without Mesa.

// The EGL loader calls are `unsafe fn`s in khronos-egl (test-only usage).
#![allow(unsafe_code, clippy::unwrap_used)]

use std::ffi::c_void;
use std::sync::Arc;

use ghostty_renderer::opengl::buffer;
use ghostty_renderer::opengl::gl::{Gl, Primitive};
use ghostty_renderer::opengl::render_pass::{Attachment, AttachmentTarget, Draw, Step};
use ghostty_renderer::opengl::shaders::{
    BgImage, BgImageFit, BgImageInfo, BgImagePosition, CellText, Image, Uniforms,
};
use ghostty_renderer::opengl::{AlphaBlending, OpenGL};
use ghostty_renderer::shadertoy;
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
    let Some((_h, mut api)) = setup() else {
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
