//! Wrapper for handling render pipelines (`metal/Pipeline.zig`).
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor,
    MTLBlendOperation,
    MTLDevice as _,
    MTLLibrary,
    MTLPixelFormat,
    MTLRenderPipelineDescriptor,
    MTLRenderPipelineState,
    MTLVertexDescriptor,
    MTLVertexFormat,
    MTLVertexStepFunction,
};

use super::{Device, Error};
use crate::metal_layout::{
    MetalVertex,
    PREMULTIPLIED_ALPHA,
    PixelFormat,
    VertexAttribute,
    VertexStepFunction,
};

/// Describes a color attachment.
#[derive(Debug, Clone, Copy)]
pub struct Attachment {
    pub pixel_format: PixelFormat,
    pub blending_enabled: bool,
}

/// Options for initializing a render pipeline.
pub struct Options<'a> {
    /// MTLDevice
    pub device: &'a Device,

    /// Name of the vertex function
    pub vertex_fn: &'a str,
    /// Name of the fragment function
    pub fragment_fn: &'a str,

    /// MTLLibrary to get the vertex function from
    pub vertex_library: &'a ProtocolObject<dyn MTLLibrary>,
    /// MTLLibrary to get the fragment function from
    pub fragment_library: &'a ProtocolObject<dyn MTLLibrary>,

    /// Vertex step function
    pub step_fn: VertexStepFunction,

    /// Info about the color attachments used by this render pipeline.
    pub attachments: &'a [Attachment],
}

/// A render pipeline state.
pub struct Pipeline {
    /// MTLRenderPipelineState
    state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

// SAFETY: MTLRenderPipelineState objects are immutable and thread-safe.
unsafe impl Send for Pipeline {}
// SAFETY: see above.
unsafe impl Sync for Pipeline {}

impl Pipeline {
    /// The underlying MTLRenderPipelineState.
    pub fn state(&self) -> &ProtocolObject<dyn MTLRenderPipelineState> {
        &self.state
    }

    /// Create a pipeline whose vertex attributes are those of `V`.
    pub fn init<V: MetalVertex>(opts: &Options<'_>) -> Result<Pipeline, Error> {
        Self::init_impl(Some((V::ATTRIBUTES, size_of::<V>())), opts)
    }

    /// Create a pipeline without a vertex descriptor.
    pub fn init_no_attributes(opts: &Options<'_>) -> Result<Pipeline, Error> {
        Self::init_impl(None, opts)
    }

    fn init_impl(
        vertex_attributes: Option<(&[VertexAttribute], usize)>,
        opts: &Options<'_>,
    ) -> Result<Pipeline, Error> {
        // Create our descriptor
        let desc = MTLRenderPipelineDescriptor::new();

        // Get our vertex and fragment functions and add them to the descriptor.
        {
            let name = NSString::from_str(opts.vertex_fn);
            let func_vert = opts
                .vertex_library
                .newFunctionWithName(&name)
                .ok_or_else(|| Error::MissingFunction(opts.vertex_fn.to_owned()))?;
            desc.setVertexFunction(Some(&func_vert));
        }
        {
            let name = NSString::from_str(opts.fragment_fn);
            let func_frag = opts
                .fragment_library
                .newFunctionWithName(&name)
                .ok_or_else(|| Error::MissingFunction(opts.fragment_fn.to_owned()))?;
            desc.setFragmentFunction(Some(&func_frag));
        }

        // If we have vertex attributes, create and add a vertex descriptor.
        if let Some((attributes, stride)) = vertex_attributes {
            let vertex_desc = MTLVertexDescriptor::new();

            // Our attributes are the fields of the input
            let attrs = vertex_desc.attributes();
            for (i, a) in attributes.iter().enumerate() {
                // SAFETY: indexing the attribute array with a valid index
                // (the descriptor array has 31 slots); offsets and buffer
                // index are plain numbers validated by pipeline creation.
                unsafe {
                    let attr = attrs.objectAtIndexedSubscript(i);
                    attr.setFormat(MTLVertexFormat(a.format as usize));
                    attr.setOffset(a.offset);
                    attr.setBufferIndex(0);
                }
            }

            // The layout describes how and when we fetch the next vertex input.
            let layouts = vertex_desc.layouts();
            // SAFETY: buffer layout 0 exists; the stride is a plain number.
            unsafe {
                let layout = layouts.objectAtIndexedSubscript(0);
                layout.setStepFunction(MTLVertexStepFunction(opts.step_fn as usize));
                layout.setStride(stride);
            }

            desc.setVertexDescriptor(Some(&vertex_desc));
        }

        // Set our color attachment
        let attachments = desc.colorAttachments();
        for (i, at) in opts.attachments.iter().enumerate() {
            // SAFETY: color attachment indexes are validated by Metal; the
            // pipelines use a single attachment.
            let attachment = unsafe { attachments.objectAtIndexedSubscript(i) };

            attachment.setPixelFormat(MTLPixelFormat(at.pixel_format as usize));

            attachment.setBlendingEnabled(at.blending_enabled);
            // We always use premultiplied alpha blending for now.
            if at.blending_enabled {
                let b = PREMULTIPLIED_ALPHA;
                attachment.setRgbBlendOperation(MTLBlendOperation(b.rgb_operation as usize));
                attachment.setAlphaBlendOperation(MTLBlendOperation(b.alpha_operation as usize));
                attachment.setSourceRGBBlendFactor(MTLBlendFactor(b.source_rgb as usize));
                attachment.setSourceAlphaBlendFactor(MTLBlendFactor(b.source_alpha as usize));
                attachment.setDestinationRGBBlendFactor(MTLBlendFactor(b.destination_rgb as usize));
                attachment
                    .setDestinationAlphaBlendFactor(MTLBlendFactor(b.destination_alpha as usize));
            }
        }

        // Make our state
        let state = opts
            .device
            .newRenderPipelineStateWithDescriptor_error(&desc)
            .map_err(|err| Error::from_ns_error(&err))?;

        Ok(Pipeline { state })
    }
}
