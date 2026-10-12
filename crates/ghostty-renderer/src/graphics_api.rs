//! The graphics API abstraction used by the generic renderer.
//!
//! Zig's `GenericRenderer(comptime GraphicsAPI: type)` duck-types its graphics
//! API (`Metal`, `OpenGL`): it reads associated declarations such as
//! `GraphicsAPI.Target`, `GraphicsAPI.Buffer`, `GraphicsAPI.shaders`,
//! `GraphicsAPI.swap_chain_count`, `custom_shader_target`,
//! `custom_shader_y_is_down`, and calls methods like `initTarget`,
//! `beginFrame`, `drawFrameStart`. [`GraphicsApi`] is the Rust spelling of that
//! implicit interface: associated types for the GPU objects, associated
//! consts for the per-API constants, and methods for the operations.
//!
//! `renderer.zig` picks the implementation by `build_config.renderer`
//! (`.metal` => `GenericRenderer(Metal)`, `.opengl` => `GenericRenderer(OpenGL)`);
//! `lib.rs` does the same with `cfg(target_vendor = "apple")`.

use std::{error::Error as StdError, fmt, sync::Arc};

use bytemuck::Pod;
use ghostty_config::types_b::AlphaBlending;
use ghostty_font::atlas::Atlas;

use crate::{backend::Health, shader_types::Uniforms, shadertoy};

/// A graphics API failure (OpenGL error, Metal failure, ...). The generic
/// renderer only reports these, so the concrete error is boxed.
#[derive(Debug)]
pub struct ApiError(Box<dyn StdError + Send + Sync>);

impl ApiError {
    /// Wrap a concrete error.
    pub fn new<E: StdError + Send + Sync + 'static>(err: E) -> Self {
        ApiError(Box::new(err))
    }

    /// An error that is only a message.
    pub fn msg(msg: impl Into<String>) -> Self {
        ApiError(msg.into().into())
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StdError for ApiError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

/// Called with the health of a frame once it has completed. For synchronous
/// APIs (OpenGL, or Metal with `sync = true`) this happens inside
/// [`GpuFrame::complete`]; for asynchronous Metal frames it happens on the
/// command buffer's completion thread (Zig: `renderer.frameCompleted`).
pub type FrameCompletion = Arc<dyn Fn(Health) + Send + Sync>;

/// Primitive types we support drawing (same subset in OpenGL and Metal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Point,
    Line,
    LineStrip,
    Triangle,
    TriangleStrip,
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

/// Which texture options (`textureOptions` / `imageTextureOptions`) to create
/// a texture with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// `api.textureOptions()`: the front/back textures of custom shaders
    /// (shader read + render target).
    CustomShader,
    /// `api.imageTextureOptions(format, srgb)`: shader-read-only images.
    Image {
        format: ImageTextureFormat,
        srgb: bool,
    },
}

/// What a color attachment renders into.
pub enum AttachmentTarget<'a, G: GraphicsApi> {
    Texture(&'a G::Texture),
    Target(&'a G::Target),
}

/// Describes a color attachment.
pub struct Attachment<'a, G: GraphicsApi> {
    pub target: AttachmentTarget<'a, G>,
    pub clear_color: Option<[f32; 4]>,
}

/// Describes the draw call for a step.
#[derive(Debug, Clone, Copy)]
pub struct Draw {
    pub ty: Primitive,
    pub vertex_count: usize,
    pub instance_count: usize,
}

impl Draw {
    pub fn new(ty: Primitive, vertex_count: usize) -> Self {
        Draw {
            ty,
            vertex_count,
            instance_count: 1,
        }
    }
}

/// Describes a step in a render pass.
///
/// Buffer slot conventions are shared by both APIs: `uniforms` is bound at
/// index 1, `buffers[0]` is the vertex buffer, the rest are storage buffers
/// bound at their slice index; `textures` / `samplers` bind at their slice
/// index.
pub struct Step<'a, G: GraphicsApi> {
    pub pipeline: &'a G::Pipeline,
    pub uniforms: Option<G::BufferRef>,
    pub buffers: &'a [Option<G::BufferRef>],
    pub textures: &'a [Option<&'a G::Texture>],
    pub samplers: &'a [Option<&'a G::Sampler>],
    pub draw: Draw,
}

/// A GPU buffer holding `T`s (`GraphicsAPI.Buffer(T)`).
pub trait GpuBuffer<G: GraphicsApi, T: Pod> {
    /// The raw handle passed to render pass steps.
    fn raw(&self) -> G::BufferRef;

    /// Sync new contents to the buffer. The data is expected to be the
    /// complete contents of the buffer. If the amount of data is larger
    /// than the buffer length, the buffer will be reallocated.
    fn sync(&mut self, data: &[T]) -> Result<(), ApiError>;

    /// Like `sync` but takes data from several slices. Returns the number
    /// of items synced.
    fn sync_from_slices(&mut self, lists: &[&[T]]) -> Result<usize, ApiError>;
}

/// A GPU texture (`GraphicsAPI.Texture`).
pub trait GpuTexture {
    fn width(&self) -> usize;
    fn height(&self) -> usize;

    /// Replace a region of the texture with the provided data. Does NOT
    /// check the dimensions of the data to ensure correctness.
    fn replace_region(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        data: &[u8],
    ) -> Result<(), ApiError>;
}

/// A render target that can be presented (`GraphicsAPI.Target`).
pub trait GpuTarget {
    fn width(&self) -> usize;
    fn height(&self) -> usize;
}

/// The collection of shader pipelines (`GraphicsAPI.shaders.Shaders`).
pub trait GpuShaders<G: GraphicsApi> {
    fn bg_color(&self) -> &G::Pipeline;
    fn cell_bg(&self) -> &G::Pipeline;
    fn cell_text(&self) -> &G::Pipeline;
    fn image(&self) -> &G::Pipeline;
    fn bg_image(&self) -> &G::Pipeline;

    /// Custom shaders to run against the final drawable texture, each one
    /// against the output of the previous.
    fn post_pipelines(&self) -> &[G::Pipeline];
}

/// A frame being encoded (`GraphicsAPI.Frame`).
pub trait GpuFrame<G: GraphicsApi> {
    /// Add a render pass to this frame with the provided attachments.
    fn render_pass<'a>(&'a self, attachments: &'a [Attachment<'a, G>]) -> G::RenderPass<'a>
    where
        G: 'a;

    /// Complete this frame and present the target. If `sync` is true, this
    /// blocks until the frame is presented. The frame's health is reported
    /// through `completion` (before this returns if the frame completes
    /// synchronously).
    fn complete(self, sync: bool, completion: FrameCompletion);
}

/// A render pass being encoded (`GraphicsAPI.RenderPass`).
pub trait GpuRenderPass<'a, G: GraphicsApi> {
    /// Add a step to this render pass.
    fn step(&mut self, step: &Step<'_, G>);

    /// Complete this render pass. It can no longer be used after this.
    fn complete(self);
}

/// The graphics API the generic renderer is instantiated with.
pub trait GraphicsApi: Sized + Send + 'static {
    /// `custom_shader_target`: Metal => MSL, OpenGL => GLSL.
    const CUSTOM_SHADER_TARGET: shadertoy::Target;
    /// `custom_shader_y_is_down`: the fragCoord for Metal shaders is +Y = down.
    const CUSTOM_SHADER_Y_IS_DOWN: bool;
    /// `swap_chain_count`: Metal triple buffers, OpenGL completes frames
    /// synchronously and needs no multi-buffering.
    const SWAP_CHAIN_COUNT: usize;

    type Target: GpuTarget + Send + Sync + 'static;
    type Texture: GpuTexture + Send + 'static;
    type Sampler: Send + 'static;
    type Buffer<T: Pod>: GpuBuffer<Self, T> + Send + 'static;
    /// The raw buffer handle bound by render pass steps.
    type BufferRef: Clone;
    type Pipeline;
    type Shaders: GpuShaders<Self> + Send + 'static;
    /// The GPU-side layout of the uniform block (`shaders.Uniforms`).
    type GpuUniforms: Pod;
    type Frame: GpuFrame<Self>;
    type RenderPass<'a>: GpuRenderPass<'a, Self>
    where
        Self: 'a;

    /// Convert the renderer's CPU-side uniforms to the GPU layout.
    fn gpu_uniforms(uniforms: &Uniforms) -> Self::GpuUniforms;

    /// The current alpha blending mode.
    fn blending(&self) -> AlphaBlending;

    /// Change the alpha blending mode (`api.blending = ..`). Targets and
    /// textures must be recreated by the caller afterwards.
    fn set_blending(&mut self, blending: AlphaBlending);

    /// Build the shader pipelines, including the post-processing pipelines
    /// for `custom_shaders` (in [`Self::CUSTOM_SHADER_TARGET`] format).
    fn init_shaders(&self, custom_shaders: &[String]) -> Result<Self::Shaders, ApiError>;

    /// Get the current size of the runtime surface.
    fn surface_size(&self) -> Result<(u32, u32), ApiError>;

    /// Initialize a new render target which can be presented by this API.
    fn init_target(&self, width: usize, height: usize) -> Result<Self::Target, ApiError>;

    /// `Buffer.init` with the API's buffer options.
    fn init_buffer<T: Pod>(&self, len: usize) -> Result<Self::Buffer<T>, ApiError>;

    /// `Buffer.initFill` with the API's buffer options.
    fn init_buffer_fill<T: Pod>(&self, data: &[T]) -> Result<Self::Buffer<T>, ApiError>;

    /// Initialize a texture with the options of `kind`.
    fn init_texture(
        &self,
        kind: TextureKind,
        width: usize,
        height: usize,
        data: Option<&[u8]>,
    ) -> Result<Self::Texture, ApiError>;

    /// Initializes a Texture suitable for the provided font atlas.
    fn init_atlas_texture(&self, atlas: &Atlas) -> Result<Self::Texture, ApiError>;

    /// Initialize the sampler used by custom shaders (`samplerOptions`).
    fn init_sampler(&self) -> Result<Self::Sampler, ApiError>;

    /// Begin a frame. The target is presented once the frame is completed
    /// ([`GpuFrame::complete`]).
    fn begin_frame(&self, target: Arc<Self::Target>) -> Self::Frame;

    /// Present the last presented target again (a no-op for Metal).
    fn present_last_target(&self) -> Result<(), ApiError>;

    /// Forget the remembered last-presented target (it is being replaced).
    fn clear_last_target(&self);

    /// Actions taken before doing anything in `drawFrame` (Metal starts an
    /// autorelease pool).
    fn draw_frame_start(&mut self) {}

    /// Actions taken after `drawFrame` is done.
    fn draw_frame_end(&mut self) {}

    /// Called by the renderer thread when it begins (`threadEnter`).
    fn thread_enter(&self) {}

    /// Called by the renderer thread when it exits (`threadExit`).
    fn thread_exit(&self) {}

    /// Called when the renderer thread starts its main loop (`loopEnter`).
    /// `draw` draws a frame synchronously; Metal installs it as the layer's
    /// display callback.
    fn loop_enter(&mut self, draw: Arc<dyn Fn() + Send + Sync>) {
        let _ = draw;
    }

    /// Called when the renderer thread leaves its main loop (`loopExit`).
    fn loop_exit(&mut self) {}

    /// Called when the display is unrealized (`displayUnrealized`).
    fn display_unrealized(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_display() {
        let e = ApiError::msg("boom");
        assert_eq!(e.to_string(), "boom");
        let e = ApiError::new(std::io::Error::other("io"));
        assert_eq!(e.to_string(), "io");
    }

    #[test]
    fn draw_defaults_to_one_instance() {
        let d = Draw::new(Primitive::Triangle, 3);
        assert_eq!(d.instance_count, 1);
        assert_eq!(d.vertex_count, 3);
    }
}
