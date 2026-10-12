//! `impl GraphicsApi for OpenGL`: adapts the OpenGL backend to the interface
//! the generic renderer is written against (see [`crate::graphics_api`]).
//!
//! The adapter delegates to the inherent OpenGL API (which keeps its richer,
//! GL-typed signatures for the apprt and the integration tests). Inherent
//! methods are always called with path syntax so they win over the trait
//! methods of the same name.
use std::sync::Arc;

use bytemuck::Pod;
use ghostty_config::types_b::AlphaBlending;
use ghostty_font::atlas::Atlas;

use super::{
    Buffer,
    Frame,
    OpenGL,
    Pipeline,
    RenderPass,
    Sampler,
    Target,
    Texture,
    gl::RawBuffer,
    shaders::Shaders,
};
use crate::{
    backend::Health,
    graphics_api::{
        ApiError,
        Attachment,
        FrameCompletion,
        GpuBuffer,
        GpuFrame,
        GpuRenderPass,
        GpuShaders,
        GpuTarget,
        GpuTexture,
        GraphicsApi,
        Step,
        TextureKind,
    },
    shader_types::Uniforms,
    shadertoy,
};

impl GraphicsApi for OpenGL {
    /// Custom shaders are converted to GLSL for OpenGL.
    const CUSTOM_SHADER_TARGET: shadertoy::Target = shadertoy::Target::Glsl;
    /// The fragCoord for OpenGL shaders is +Y = up.
    const CUSTOM_SHADER_Y_IS_DOWN: bool = false;
    /// Because OpenGL's frame completion is always
    /// sync, we have no need for multi-buffering.
    const SWAP_CHAIN_COUNT: usize = 1;

    type Target = Target;
    type Texture = Texture;
    type Sampler = Sampler;
    type Buffer<T: Pod> = Buffer<T>;
    type BufferRef = RawBuffer;
    type Pipeline = Pipeline;
    type Shaders = Shaders;
    type GpuUniforms = Uniforms;
    type Frame = GlFrame;
    type RenderPass<'a> = RenderPass<'a>;

    fn gpu_uniforms(uniforms: &Uniforms) -> Uniforms {
        *uniforms
    }

    fn blending(&self) -> AlphaBlending {
        self.blending
    }

    fn set_blending(&mut self, blending: AlphaBlending) {
        self.blending = blending;
    }

    fn init_shaders(&self, custom_shaders: &[String]) -> Result<Shaders, ApiError> {
        OpenGL::init_shaders(self, custom_shaders).map_err(ApiError::new)
    }

    fn surface_size(&self) -> Result<(u32, u32), ApiError> {
        OpenGL::surface_size(self).map_err(ApiError::new)
    }

    fn init_target(&self, width: usize, height: usize) -> Result<Target, ApiError> {
        OpenGL::init_target(self, width, height).map_err(ApiError::new)
    }

    fn init_buffer<T: Pod>(&self, len: usize) -> Result<Buffer<T>, ApiError> {
        let gl = self.gl_or_err().map_err(ApiError::new)?;
        Buffer::init(gl, self.buffer_options(), len).map_err(ApiError::new)
    }

    fn init_buffer_fill<T: Pod>(&self, data: &[T]) -> Result<Buffer<T>, ApiError> {
        let gl = self.gl_or_err().map_err(ApiError::new)?;
        Buffer::init_fill(gl, self.buffer_options(), data).map_err(ApiError::new)
    }

    fn init_texture(
        &self,
        kind: TextureKind,
        width: usize,
        height: usize,
        data: Option<&[u8]>,
    ) -> Result<Texture, ApiError> {
        let gl = self.gl_or_err().map_err(ApiError::new)?;
        let opts = match kind {
            TextureKind::CustomShader => self.texture_options(),
            TextureKind::Image { format, srgb } => self.image_texture_options(format, srgb),
        };
        Texture::init(gl, opts, width, height, data).map_err(ApiError::new)
    }

    fn init_atlas_texture(&self, atlas: &Atlas) -> Result<Texture, ApiError> {
        OpenGL::init_atlas_texture(self, atlas).map_err(ApiError::new)
    }

    fn init_sampler(&self) -> Result<Sampler, ApiError> {
        let gl = self.gl_or_err().map_err(ApiError::new)?;
        Sampler::init(gl, self.sampler_options()).map_err(ApiError::new)
    }

    fn begin_frame(&self, target: Arc<Target>) -> GlFrame {
        GlFrame(OpenGL::begin_frame(self, target))
    }

    fn present_last_target(&self) -> Result<(), ApiError> {
        OpenGL::present_last_target(self).map_err(ApiError::new)
    }

    fn clear_last_target(&self) {
        OpenGL::clear_last_target(self);
    }

    fn draw_frame_start(&mut self) {
        OpenGL::draw_frame_start(self);
    }

    fn draw_frame_end(&mut self) {
        OpenGL::draw_frame_end(self);
    }

    fn thread_enter(&self) {
        OpenGL::thread_enter(self);
    }

    fn thread_exit(&self) {
        OpenGL::thread_exit(self);
    }
}

impl<T: Pod> GpuBuffer<OpenGL, T> for Buffer<T> {
    fn raw(&self) -> RawBuffer {
        Buffer::raw(self)
    }

    fn sync(&mut self, data: &[T]) -> Result<(), ApiError> {
        Buffer::sync(self, data).map_err(ApiError::new)
    }

    fn sync_from_slices(&mut self, lists: &[&[T]]) -> Result<usize, ApiError> {
        Buffer::sync_from_slices(self, lists).map_err(ApiError::new)
    }
}

impl GpuTexture for Texture {
    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn replace_region(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        data: &[u8],
    ) -> Result<(), ApiError> {
        Texture::replace_region(self, x, y, width, height, data).map_err(ApiError::new)
    }
}

impl GpuTarget for Target {
    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }
}

impl GpuShaders<OpenGL> for Shaders {
    fn bg_color(&self) -> &Pipeline {
        &self.bg_color
    }

    fn cell_bg(&self) -> &Pipeline {
        &self.cell_bg
    }

    fn cell_text(&self) -> &Pipeline {
        &self.cell_text
    }

    fn image(&self) -> &Pipeline {
        &self.image
    }

    fn bg_image(&self) -> &Pipeline {
        &self.bg_image
    }

    fn post_pipelines(&self) -> &[Pipeline] {
        &self.post_pipelines
    }
}

/// An OpenGL [`Frame`] as seen by the generic renderer.
pub struct GlFrame(Frame);

impl GpuFrame<OpenGL> for GlFrame {
    fn render_pass<'a>(&'a self, attachments: &'a [Attachment<'a, OpenGL>]) -> RenderPass<'a>
    where
        OpenGL: 'a,
    {
        self.0.render_pass(attachments)
    }

    fn complete(self, sync: bool, completion: FrameCompletion) {
        let health: Health = self.0.complete(sync);
        completion(health);
    }
}

impl<'a> GpuRenderPass<'a, OpenGL> for RenderPass<'a> {
    fn step(&mut self, step: &Step<'_, OpenGL>) {
        RenderPass::step(self, step);
    }

    fn complete(self) {
        RenderPass::complete(self);
    }
}
