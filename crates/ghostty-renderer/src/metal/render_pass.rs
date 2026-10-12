//! Wrapper for handling render passes (`metal/RenderPass.zig`).
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLClearColor,
    MTLCommandBuffer,
    MTLCommandEncoder as _,
    MTLLoadAction,
    MTLPrimitiveType,
    MTLRenderCommandEncoder,
    MTLRenderPassDescriptor,
    MTLStoreAction,
};

use super::Metal;
use crate::{
    graphics_api::{Attachment, AttachmentTarget, GpuRenderPass, Primitive, Step},
    metal_layout::{UNIFORMS_BUFFER_INDEX, buffer_index},
};

/// The `MTLPrimitiveType` of a [`Primitive`].
fn primitive_type(p: Primitive) -> MTLPrimitiveType {
    match p {
        Primitive::Point => MTLPrimitiveType::Point,
        Primitive::Line => MTLPrimitiveType::Line,
        Primitive::LineStrip => MTLPrimitiveType::LineStrip,
        Primitive::Triangle => MTLPrimitiveType::Triangle,
        Primitive::TriangleStrip => MTLPrimitiveType::TriangleStrip,
    }
}

/// A render pass being encoded.
pub struct RenderPass {
    /// MTLRenderCommandEncoder (`None` if the encoder couldn't be created, in
    /// which case every step is a no-op).
    encoder: Option<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>>,
}

impl RenderPass {
    /// Begin a render pass.
    pub fn begin(
        command_buffer: Option<&ProtocolObject<dyn MTLCommandBuffer>>,
        attachments: &[Attachment<'_, Metal>],
    ) -> RenderPass {
        let Some(command_buffer) = command_buffer else {
            return RenderPass { encoder: None };
        };
        // Create a pass descriptor
        let desc = MTLRenderPassDescriptor::renderPassDescriptor();

        // Set our color attachment to be our drawable surface.
        let color_attachments = desc.colorAttachments();
        for (i, at) in attachments.iter().enumerate() {
            // SAFETY: color attachment indexes are validated by Metal.
            let attachment = unsafe { color_attachments.objectAtIndexedSubscript(i) };

            attachment.setLoadAction(if at.clear_color.is_some() {
                MTLLoadAction::Clear
            } else {
                MTLLoadAction::Load
            });
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setTexture(Some(match &at.target {
                AttachmentTarget::Texture(t) => t.texture(),
                AttachmentTarget::Target(t) => t.texture(),
            }));
            if let Some(c) = at.clear_color {
                attachment.setClearColor(MTLClearColor {
                    red: f64::from(c[0]),
                    green: f64::from(c[1]),
                    blue: f64::from(c[2]),
                    alpha: f64::from(c[3]),
                });
            }
        }

        // MTLRenderCommandEncoder
        let encoder = command_buffer.renderCommandEncoderWithDescriptor(&desc);
        if encoder.is_none() {
            log::warn!("unable to create a Metal render command encoder");
        }

        RenderPass { encoder }
    }

    /// Add a step to this render pass.
    pub fn step(&self, s: &Step<'_, Metal>) {
        let Some(encoder) = &self.encoder else {
            return;
        };
        if s.draw.instance_count == 0 {
            return;
        }

        // Set pipeline state
        encoder.setRenderPipelineState(s.pipeline.state());

        if !s.buffers.is_empty() {
            // We set buffers for both stages, at index 0 for the vertex
            // buffer (this isn't very flexible but it lines up with the API we
            // have for OpenGL) and the rest starting at index 2, so that we
            // can use index 1 for the uniforms if present.
            //
            // Also, we set buffers (and textures) for both stages.
            //
            // Again, not very flexible, but it's consistent and predictable,
            // and we need to treat the uniforms as special because of OpenGL.
            //
            // TODO: Maybe in the future add info to the pipeline struct which
            //       allows it to define a mapping between provided buffers and
            //       what index they get set at for the vertex / fragment stage.
            for (i, b) in s.buffers.iter().enumerate() {
                if let Some(buf) = b {
                    // SAFETY: the buffer is a live MTLBuffer.
                    unsafe {
                        encoder.setVertexBuffer_offset_atIndex(Some(buf), 0, buffer_index(i));
                        encoder.setFragmentBuffer_offset_atIndex(Some(buf), 0, buffer_index(i));
                    }
                }
            }
        }

        // Set the uniforms as buffer index 1 if present.
        if let Some(buf) = &s.uniforms {
            // SAFETY: the buffer is a live MTLBuffer.
            unsafe {
                encoder.setVertexBuffer_offset_atIndex(Some(buf), 0, UNIFORMS_BUFFER_INDEX);
                encoder.setFragmentBuffer_offset_atIndex(Some(buf), 0, UNIFORMS_BUFFER_INDEX);
            }
        }

        // Set textures.
        for (i, t) in s.textures.iter().enumerate() {
            if let Some(tex) = t {
                // SAFETY: the texture is a live MTLTexture.
                unsafe {
                    encoder.setVertexTexture_atIndex(Some(tex.texture()), i);
                    encoder.setFragmentTexture_atIndex(Some(tex.texture()), i);
                }
            }
        }

        // Set samplers.
        for (i, samp) in s.samplers.iter().enumerate() {
            if let Some(sampler) = samp {
                // SAFETY: the sampler is a live MTLSamplerState.
                unsafe {
                    encoder.setFragmentSamplerState_atIndex(Some(sampler.sampler()), i);
                }
            }
        }

        // Draw!
        //
        // SAFETY: all resources the pipeline reads were bound above (the
        // same contract as the Zig renderer; Metal validation reports misuse).
        unsafe {
            encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                primitive_type(s.draw.ty),
                0,
                s.draw.vertex_count,
                s.draw.instance_count,
            );
        }
    }

    /// Complete this render pass.
    /// This struct can no longer be used after calling this.
    pub fn complete(self) {
        if let Some(encoder) = &self.encoder {
            encoder.endEncoding();
        }
    }
}

impl GpuRenderPass<'_, Metal> for RenderPass {
    fn step(&mut self, step: &Step<'_, Metal>) {
        RenderPass::step(self, step);
    }

    fn complete(self) {
        RenderPass::complete(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_mapping() {
        // MTLPrimitiveType raw values (api.zig).
        assert_eq!(primitive_type(Primitive::Point).0, 0);
        assert_eq!(primitive_type(Primitive::Line).0, 1);
        assert_eq!(primitive_type(Primitive::LineStrip).0, 2);
        assert_eq!(primitive_type(Primitive::Triangle).0, 3);
        assert_eq!(primitive_type(Primitive::TriangleStrip).0, 4);
    }
}
