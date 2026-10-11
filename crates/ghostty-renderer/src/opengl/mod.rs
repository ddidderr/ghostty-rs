//! Graphics API wrapper for OpenGL (`renderer/OpenGL.zig` and `renderer/opengl/*`).
//!
//! The context itself is owned by the apprt (a `GtkGLArea` for GTK). The
//! renderer never creates a context: it is given a `get_proc_address` loader
//! closure (see [`OpenGL::prepare_context`]) by the apprt once the context is
//! current, and everything afterwards goes through the [`Gl`] handle.

pub mod buffer;
pub mod frame;
#[allow(unsafe_code)]
pub mod gl;
pub mod pipeline;
pub mod render_pass;
pub mod sampler;
pub mod shaders;
pub mod target;
pub mod texture;

use std::ffi::c_void;
use std::sync::Arc;

use ghostty_font::atlas::{Atlas, Format as AtlasFormat};

pub use self::buffer::Buffer;
pub use self::frame::Frame;
pub use self::gl::Gl;
use self::gl::{
    BufferTarget, BufferUsage, FramebufferTarget, InternalFormat, MagFilter, MinFilter,
    PixelFormat, TextureTarget, Wrap,
};
pub use self::pipeline::Pipeline;
pub use self::render_pass::RenderPass;
pub use self::sampler::Sampler;
pub use self::target::Target;
pub use self::texture::Texture;
use crate::shadertoy;

pub type GraphicsApi = OpenGL;

/// Custom shaders are converted to GLSL for OpenGL.
pub const CUSTOM_SHADER_TARGET: shadertoy::Target = shadertoy::Target::Glsl;
/// The fragCoord for OpenGL shaders is +Y = up.
pub const CUSTOM_SHADER_Y_IS_DOWN: bool = false;

/// Because OpenGL's frame completion is always
/// sync, we have no need for multi-buffering.
pub const SWAP_CHAIN_COUNT: usize = 1;

/// We require at least OpenGL 4.3
pub const MIN_VERSION_MAJOR: u32 = 4;
pub const MIN_VERSION_MINOR: u32 = 3;

/// `config.AlphaBlending`.
pub use ghostty_config::types_b::AlphaBlending;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "OpenGL version is too old, Ghostty requires OpenGL {MIN_VERSION_MAJOR}.{MIN_VERSION_MINOR} (got {0}.{1})"
    )]
    OpenGLOutdated(u32, u32),
    #[error("OpenGL context has not been prepared yet")]
    NoContext,
    #[error(transparent)]
    Gl(#[from] gl::Error),
    #[error(transparent)]
    Texture(#[from] texture::Error),
}

/// The OpenGL graphics API backend.
pub struct OpenGL {
    /// Loaded GL context; `None` until [`OpenGL::prepare_context`] (apprt
    /// supplies the loader once the GL context exists).
    gl: Option<Gl>,

    /// Alpha blending mode
    pub blending: AlphaBlending,

    /// The most recently presented target, in case we need to present it again.
    last_target: Option<Arc<Target>>,
}

/// Pixel format for image texture options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTextureFormat {
    /// 1 byte per pixel grayscale.
    Gray,
    /// 4 bytes per pixel RGBA.
    Rgba,
    /// 4 bytes per pixel BGRA.
    Bgra,
}

impl ImageTextureFormat {
    fn to_pixel_format(self) -> PixelFormat {
        match self {
            ImageTextureFormat::Gray => PixelFormat::Red,
            ImageTextureFormat::Rgba => PixelFormat::Rgba,
            ImageTextureFormat::Bgra => PixelFormat::Bgra,
        }
    }
}

impl OpenGL {
    /// Create the backend. `OpenGL.init`; the GL context is attached later
    /// with [`OpenGL::prepare_context`] (GTK: from the GLArea `realize`).
    pub fn init(blending: AlphaBlending) -> OpenGL {
        OpenGL {
            gl: None,
            blending,
            last_target: None,
        }
    }

    /// Create the backend around an already loaded context (used by tests and
    /// by apprts that load GL themselves).
    pub fn with_gl(blending: AlphaBlending, gl: Gl) -> OpenGL {
        OpenGL {
            gl: Some(gl),
            blending,
            last_target: None,
        }
    }

    /// The loaded GL handle, if the context has been prepared.
    pub fn gl(&self) -> Option<&Gl> {
        self.gl.as_ref()
    }

    fn gl_or_err(&self) -> Result<&Gl, Error> {
        self.gl.as_ref().ok_or(Error::NoContext)
    }

    /// Prepares the provided GL context, loading it with `get_proc_address`
    /// (`prepareContext`). The context must be current on the calling thread.
    pub fn prepare_context<F>(&mut self, get_proc_address: F) -> Result<(), Error>
    where
        F: FnMut(&str) -> *const c_void,
    {
        let gl = Gl::from_loader_with_debug(get_proc_address);
        let (major, minor) = gl.version();
        log::info!("loaded OpenGL {major}.{minor}");

        // Need to check version before trying to enable it
        if major < MIN_VERSION_MAJOR || (major == MIN_VERSION_MAJOR && minor < MIN_VERSION_MINOR) {
            log::warn!(
                "OpenGL version is too old. Ghostty requires OpenGL {MIN_VERSION_MAJOR}.{MIN_VERSION_MINOR}"
            );
            return Err(Error::OpenGLOutdated(major, minor));
        }

        // Enable debug output for the context.
        gl.enable(glow::DEBUG_OUTPUT)?;

        // Enable SRGB framebuffer for linear blending support.
        gl.enable(glow::FRAMEBUFFER_SRGB)?;

        self.gl = Some(gl);
        Ok(())
    }

    /// This is called early right after surface creation.
    ///
    /// GTK uses a global OpenGL context, so the apprt passes its loader here.
    pub fn surface_init<F>(&mut self, get_proc_address: F) -> Result<(), Error>
    where
        F: FnMut(&str) -> *const c_void,
    {
        self.prepare_context(get_proc_address)
    }

    /// This is called just prior to spinning up the renderer
    /// thread for final main thread setup requirements.
    pub fn finalize_surface_init(&self) {}

    /// Callback called by the renderer thread when it begins.
    ///
    /// GTK doesn't support threaded OpenGL operations as far as I can
    /// tell, so we use the renderer thread to setup all the state
    /// but then do the actual draws and texture syncs and all that
    /// on the main thread. As such, we don't do anything here.
    pub fn thread_enter(&self) {}

    /// Callback called by the renderer thread when it exits.
    ///
    /// We don't need to do any unloading for GTK because we may
    /// be sharing the global bindings with other windows.
    pub fn thread_exit(&self) {}

    /// Called by the apprt when the display (GLArea) is (re)realized.
    pub fn display_realized<F>(&mut self, get_proc_address: F)
    where
        F: FnMut(&str) -> *const c_void,
    {
        if let Err(err) = self.prepare_context(get_proc_address) {
            log::warn!("Error preparing GL context in displayRealized, err={err}");
        }
    }

    /// Actions taken before doing anything in `drawFrame`.
    ///
    /// Right now there's nothing we need to do for OpenGL.
    pub fn draw_frame_start(&mut self) {}

    /// Actions taken after `drawFrame` is done.
    ///
    /// Right now there's nothing we need to do for OpenGL.
    pub fn draw_frame_end(&mut self) {}

    /// Build the shader pipelines, including the post-processing pipelines
    /// for `custom_shaders` (GLSL produced by [`shadertoy::load_from_files`]).
    pub fn init_shaders<S: AsRef<str>>(
        &self,
        custom_shaders: &[S],
    ) -> Result<shaders::Shaders, Error> {
        Ok(shaders::Shaders::init(self.gl_or_err()?, custom_shaders)?)
    }

    /// Get the current size of the runtime surface: `(width, height)`.
    pub fn surface_size(&self) -> Result<(u32, u32), Error> {
        let viewport = self.gl_or_err()?.get_viewport();
        Ok((viewport[2] as u32, viewport[3] as u32))
    }

    /// Initialize a new render target which can be presented by this API.
    pub fn init_target(&self, width: usize, height: usize) -> Result<Target, Error> {
        Ok(Target::init(
            self.gl_or_err()?,
            target::Options {
                internal_format: if self.blending.is_linear() {
                    InternalFormat::Srgba
                } else {
                    InternalFormat::Rgba
                },
                width,
                height,
            },
        )?)
    }

    /// Present the provided target.
    pub fn present(&mut self, target: &Arc<Target>) -> Result<(), Error> {
        let gl = self.gl_or_err()?.clone();

        // In order to present a target we blit it to the default framebuffer.

        // We disable GL_FRAMEBUFFER_SRGB while doing this blit, otherwise the
        // values may be linearized as they're copied, but even though the draw
        // framebuffer has a linear internal format, the values in it should be
        // sRGB, not linear!
        gl.disable(glow::FRAMEBUFFER_SRGB)?;

        // Bind the target for reading.
        let fbobind = gl.bind_framebuffer(target.framebuffer(), FramebufferTarget::Read);

        // Blit
        let (w, h) = (target.width as i32, target.height as i32);
        gl.blit_framebuffer(
            (0, 0, w, h),
            (0, 0, w, h),
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );

        gl.unbind_framebuffer(fbobind);
        if let Err(err) = gl.enable(glow::FRAMEBUFFER_SRGB) {
            log::error!("Error re-enabling GL_FRAMEBUFFER_SRGB, err={err}");
        }

        // Keep track of this target in case we need to repeat it.
        self.last_target = Some(Arc::clone(target));
        Ok(())
    }

    /// Present the last presented target again.
    pub fn present_last_target(&mut self) -> Result<(), Error> {
        if let Some(target) = self.last_target.clone() {
            self.present(&target)?;
        }
        Ok(())
    }

    /// The most recently presented target, if any (also set when the blit to
    /// the default framebuffer is a no-op, e.g. on a surfaceless context).
    pub fn last_target(&self) -> Option<&Arc<Target>> {
        self.last_target.as_ref()
    }

    /// Drop the remembered last-presented target (call when the target it
    /// refers to is being replaced, e.g. on resize).
    pub fn clear_last_target(&mut self) {
        self.last_target = None;
    }

    /// Returns the options to use when constructing buffers.
    pub fn buffer_options(&self) -> buffer::Options {
        buffer::Options {
            target: BufferTarget::Array,
            usage: BufferUsage::DynamicDraw,
        }
    }

    pub fn instance_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }
    pub fn uniform_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }
    pub fn fg_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }
    pub fn bg_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }
    pub fn image_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }
    pub fn bg_image_buffer_options(&self) -> buffer::Options {
        self.buffer_options()
    }

    /// Returns the options to use when constructing textures.
    pub fn texture_options(&self) -> texture::Options {
        texture::Options {
            format: PixelFormat::Rgba,
            internal_format: InternalFormat::Srgba,
            target: TextureTarget::D2,
            min_filter: MinFilter::Linear,
            mag_filter: MagFilter::Linear,
            wrap_s: Wrap::ClampToEdge,
            wrap_t: Wrap::ClampToEdge,
        }
    }

    /// Returns the options to use when constructing samplers.
    pub fn sampler_options(&self) -> sampler::Options {
        sampler::Options {
            min_filter: MinFilter::Linear,
            mag_filter: MagFilter::Linear,
            wrap_s: Wrap::ClampToEdge,
            wrap_t: Wrap::ClampToEdge,
        }
    }

    /// Returns the options to use when constructing textures for images.
    pub fn image_texture_options(
        &self,
        format: ImageTextureFormat,
        srgb: bool,
    ) -> texture::Options {
        texture::Options {
            format: format.to_pixel_format(),
            internal_format: if srgb {
                InternalFormat::Srgba
            } else {
                InternalFormat::Rgba
            },
            target: TextureTarget::D2,
            // TODO: Generate mipmaps for image textures and use
            //       linear_mipmap_linear filtering so that they
            //       look good even when scaled way down.
            min_filter: MinFilter::Linear,
            mag_filter: MagFilter::Linear,
            // TODO: Separate out background image options, use
            //       repeating coordinate modes so we don't have
            //       to do the modulus in the shader.
            wrap_s: Wrap::ClampToEdge,
            wrap_t: Wrap::ClampToEdge,
        }
    }

    /// Initializes a Texture suitable for the provided font atlas.
    pub fn init_atlas_texture(&self, atlas: &Atlas) -> Result<Texture, Error> {
        let (format, internal_format) = match atlas.format {
            AtlasFormat::Grayscale => (PixelFormat::Red, InternalFormat::Red),
            AtlasFormat::Bgra => (PixelFormat::Bgra, InternalFormat::Srgba),
            AtlasFormat::Bgr => panic!("unsupported atlas format for OpenGL texture"),
        };

        Ok(Texture::init(
            self.gl_or_err().map_err(|_| texture::Error::OpenGLFailed)?,
            texture::Options {
                format,
                internal_format,
                target: TextureTarget::Rectangle,
                min_filter: MinFilter::Nearest,
                mag_filter: MagFilter::Nearest,
                wrap_s: Wrap::ClampToEdge,
                wrap_t: Wrap::ClampToEdge,
            },
            atlas.size as usize,
            atlas.size as usize,
            None,
        )?)
    }

    /// Begin a frame. The target is presented via this API when the frame is
    /// completed (see [`Frame::complete`]).
    pub fn begin_frame(&mut self, target: Arc<Target>) -> Frame<'_> {
        Frame::begin(frame::Options::default(), self, target)
    }
}
