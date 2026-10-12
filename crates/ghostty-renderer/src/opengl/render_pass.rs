//! Wrapper for handling render passes (`opengl/RenderPass.zig`).
use super::{
    gl::{Attachment as GlAttachment, BufferTarget, FramebufferTarget, Gl, Primitive, RawBuffer},
    pipeline::Pipeline,
    sampler::Sampler,
    target::Target,
    texture::Texture,
};

/// What a color attachment renders into.
pub enum AttachmentTarget<'a> {
    Texture(&'a Texture),
    Target(&'a Target),
}

/// Describes a color attachment.
pub struct Attachment<'a> {
    pub target: AttachmentTarget<'a>,
    pub clear_color: Option<[f32; 4]>,
}

/// Options for beginning a render pass.
#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// Color attachments for this render pass.
    pub attachments: &'a [Attachment<'a>],
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
pub struct Step<'a> {
    pub pipeline: &'a Pipeline,
    pub uniforms: Option<RawBuffer>,
    pub buffers: &'a [Option<RawBuffer>],
    pub textures: &'a [Option<&'a Texture>],
    pub samplers: &'a [Option<&'a Sampler>],
    pub draw: Draw,
}

pub struct RenderPass<'a> {
    gl: Gl,
    attachments: &'a [Attachment<'a>],
    step_number: usize,
}

impl<'a> RenderPass<'a> {
    /// Begin a render pass.
    pub fn begin(gl: &Gl, opts: Options<'a>) -> RenderPass<'a> {
        RenderPass {
            gl: gl.clone(),
            attachments: opts.attachments,
            step_number: 0,
        }
    }

    /// Add a step to this render pass.
    ///
    /// TODO: Errors are silently ignored in this function, maybe they shouldn't be?
    /// (Same as upstream; failures are logged at debug level here.)
    pub fn step(&mut self, s: &Step<'_>) {
        if s.draw.instance_count == 0 {
            return;
        }
        if self.step_(s).is_none() {
            log::debug!("render pass step failed");
        }
        self.step_number += 1;
    }

    fn step_(&mut self, s: &Step<'_>) -> Option<()> {
        let gl = self.gl.clone();

        gl.use_program(Some(s.pipeline.program)).ok()?;
        // Unbinding on all exit paths.
        let r = (|| {
            gl.bind_vertex_array(Some(s.pipeline.vao)).ok()?;
            let r = self.step_with_vao(s);
            let _ = gl.bind_vertex_array(None);
            r
        })();
        let _ = gl.use_program(None);
        r
    }

    fn step_with_vao(&mut self, s: &Step<'_>) -> Option<()> {
        let gl = self.gl.clone();

        let fbobind = match &self.attachments[0].target {
            AttachmentTarget::Target(t) => {
                gl.bind_framebuffer(t.framebuffer(), FramebufferTarget::Framebuffer)
            }
            AttachmentTarget::Texture(t) => {
                let fbobind = gl.bind_framebuffer(s.pipeline.fbo, FramebufferTarget::Framebuffer);
                if gl
                    .framebuffer_texture_2d(&fbobind, GlAttachment::Color0, t.target, t.raw(), 0)
                    .is_err()
                {
                    gl.unbind_framebuffer(fbobind);
                    return None;
                }
                fbobind
            }
        };

        let r = self.step_with_fbo(s);
        self.gl.unbind_framebuffer(fbobind);
        r
    }

    fn step_with_fbo(&mut self, s: &Step<'_>) -> Option<()> {
        let gl = &self.gl;

        // If we have a clear color and this is the
        // first step in the pass, go ahead and clear.
        if self.step_number == 0
            && let Some(c) = self.attachments[0].clear_color
        {
            gl.clear_color(c[0], c[1], c[2], c[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }

        // Bind the uniform buffer we bind at index 1 to align with Metal.
        if let Some(ubo) = s.uniforms {
            gl.bind_buffer_base(BufferTarget::Uniform, 1, ubo).ok()?;
        }

        // Bind relevant texture units.
        for (i, t) in s.textures.iter().enumerate() {
            if let Some(tex) = t {
                gl.active_texture(i as u32).ok()?;
                gl.bind_texture(tex.target, Some(tex.raw())).ok()?;
            }
        }

        // Bind relevant samplers.
        for (i, sampler) in s.samplers.iter().enumerate() {
            if let Some(sampler) = sampler {
                gl.bind_sampler(i as u32, sampler.raw()).ok()?;
            }
        }

        // Bind 0th buffer as the vertex buffer,
        // and bind the rest as storage buffers.
        if !s.buffers.is_empty() {
            if let Some(vbo) = s.buffers[0] {
                gl.bind_vertex_buffer(0, vbo, 0, s.pipeline.stride as i32)
                    .ok()?;
            }

            for (i, b) in s.buffers.iter().enumerate().skip(1) {
                if let Some(buf) = b {
                    gl.bind_buffer_base(BufferTarget::Storage, i as u32, *buf)
                        .ok()?;
                }
            }
        }

        if s.pipeline.blending_enabled {
            gl.enable(glow::BLEND).ok()?;
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA).ok()?;
        } else {
            gl.disable(glow::BLEND).ok()?;
        }

        gl.draw_arrays_instanced(
            s.draw.ty,
            0,
            s.draw.vertex_count as i32,
            s.draw.instance_count as i32,
        )
        .ok()
    }

    /// Complete this render pass.
    /// This struct can no longer be used after calling this.
    pub fn complete(self) {
        self.gl.flush();
    }
}
