//! The Metal backend of the embedded inspector
//! (`ghostty_inspector_metal_{init,render,shutdown}`).
//!
//! Upstream links Dear ImGui's `imgui_impl_metal.mm`; this is the same
//! renderer written against `objc2-metal` and `dear-imgui-rs`'s managed
//! texture protocol: the draw lists are uploaded to Metal buffers and drawn
//! with the `imgui_impl_metal` shaders into the render pass the host
//! provides.
#![allow(unsafe_code)]

use std::{collections::HashMap, ffi::c_void, ptr::NonNull};

use ghostty_inspector::imgui::{
    BackendFlags,
    TextureFormat,
    TextureId,
    render::{
        DrawCmd,
        DrawIdx,
        DrawVert,
        PendingFrame,
        SnapshotTextureId,
        SynchronousRendererConsumer,
        TextureFeedback,
        TextureOp,
        TextureRequest,
    },
};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor,
    MTLBlendOperation,
    MTLCommandBuffer,
    MTLCommandEncoder as _,
    MTLCompareFunction,
    MTLCullMode,
    MTLDepthStencilDescriptor,
    MTLDepthStencilState,
    MTLDevice,
    MTLIndexType,
    MTLLibrary,
    MTLOrigin,
    MTLPixelFormat,
    MTLPrimitiveType,
    MTLRegion,
    MTLRenderCommandEncoder,
    MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor,
    MTLRenderPipelineState,
    MTLResourceOptions,
    MTLScissorRect,
    MTLSize,
    MTLTexture,
    MTLTextureDescriptor,
    MTLVertexDescriptor,
    MTLVertexFormat,
    MTLVertexStepFunction,
    MTLViewport,
};

use crate::inspector::Inspector;

/// The shaders of `imgui_impl_metal.mm`.
const SHADERS: &str = r"
#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float4x4 projectionMatrix;
};

struct VertexIn {
    float2 position  [[attribute(0)]];
    float2 texCoords [[attribute(1)]];
    uchar4 color     [[attribute(2)]];
};

struct VertexOut {
    float4 position [[position]];
    float2 texCoords;
    float4 color;
};

vertex VertexOut vertex_main(VertexIn in [[stage_in]],
                             constant Uniforms &uniforms [[buffer(1)]]) {
    VertexOut out;
    out.position = uniforms.projectionMatrix * float4(in.position, 0, 1);
    out.texCoords = in.texCoords;
    out.color = float4(in.color) / float4(255.0);
    return out;
}

fragment half4 fragment_main(VertexOut in [[stage_in]],
                             texture2d<half, access::sample> texture [[texture(0)]]) {
    constexpr sampler linearSampler(coord::normalized,
                                    min_filter::linear,
                                    mag_filter::linear,
                                    mip_filter::linear);
    half4 texColor = texture.sample(linearSampler, in.texCoords);
    return half4(in.color) * texColor;
}
";

type Device = Retained<ProtocolObject<dyn MTLDevice>>;

/// A pipeline state for one render target configuration.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct PipelineKey {
    color_format: usize,
    sample_count: usize,
    depth_format: usize,
    stencil_format: usize,
}

/// The Metal draw backend of one inspector.
pub struct MetalBackend {
    device: Device,
    library: Retained<ProtocolObject<dyn MTLLibrary>>,
    depth_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
    pipelines: HashMap<PipelineKey, Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    consumer: SynchronousRendererConsumer,
    /// Managed textures by Dear ImGui's identity.
    textures: HashMap<SnapshotTextureId, (u64, Retained<ProtocolObject<dyn MTLTexture>>)>,
    next_texture_id: u64,
}

impl MetalBackend {
    /// Takes ownership of one reference to `device` (Zig: `defer
    /// device.release()`).
    ///
    /// # Safety
    /// `device` must be a retained `MTLDevice`.
    pub unsafe fn init(
        device: *mut c_void,
        ctx: &mut ghostty_inspector::imgui::Context,
    ) -> Result<MetalBackend, String> {
        // SAFETY: per the function contract we own one reference.
        let device: Device =
            unsafe { Retained::from_raw(device.cast::<ProtocolObject<dyn MTLDevice>>()) }
                .ok_or_else(|| "null Metal device".to_owned())?;

        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADERS), None)
            .map_err(|e| e.localizedDescription().to_string())?;

        let depth_desc = MTLDepthStencilDescriptor::new();
        depth_desc.setDepthCompareFunction(MTLCompareFunction::Always);
        depth_desc.setDepthWriteEnabled(false);
        let depth_state = device
            .newDepthStencilStateWithDescriptor(&depth_desc)
            .ok_or_else(|| "failed to create the depth stencil state".to_owned())?;

        // Publish the renderer capabilities and the managed texture
        // consumer.
        let consumer = ctx
            .create_synchronous_renderer_consumer()
            .map_err(|e| e.to_string())?;
        let reset = ctx
            .prepare_renderer_texture_reset(&consumer)
            .map_err(|e| e.to_string())?;
        reset.commit();
        if let Err(err) = ctx.set_renderer_name(Some("ghostty_embedded_metal")) {
            log::warn!("unable to set the imgui renderer name err={err}");
        }
        let io = ctx.io_mut();
        io.set_backend_flags(
            io.backend_flags()
                | BackendFlags::RENDERER_HAS_VTX_OFFSET
                | BackendFlags::RENDERER_HAS_TEXTURES,
        );

        Ok(MetalBackend {
            device,
            library,
            depth_state,
            pipelines: HashMap::new(),
            consumer,
            textures: HashMap::new(),
            next_texture_id: 1,
        })
    }

    /// Release the GPU objects.
    pub fn shutdown(self) {
        drop(self);
    }

    fn pipeline(
        &mut self,
        key: PipelineKey,
    ) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, String> {
        if let Some(p) = self.pipelines.get(&key) {
            return Ok(p.clone());
        }

        let desc = MTLRenderPipelineDescriptor::new();
        let vertex = self
            .library
            .newFunctionWithName(&NSString::from_str("vertex_main"))
            .ok_or("missing vertex_main")?;
        let fragment = self
            .library
            .newFunctionWithName(&NSString::from_str("fragment_main"))
            .ok_or("missing fragment_main")?;
        desc.setVertexFunction(Some(&vertex));
        desc.setFragmentFunction(Some(&fragment));

        // ImDrawVert: float2 pos, float2 uv, uchar4 col.
        let vd = MTLVertexDescriptor::new();
        let stride = size_of::<DrawVert>();
        // SAFETY: attribute/layout indexes are valid; plain numbers.
        unsafe {
            let attrs = vd.attributes();
            let a0 = attrs.objectAtIndexedSubscript(0);
            a0.setFormat(MTLVertexFormat::Float2);
            a0.setOffset(std::mem::offset_of!(DrawVert, pos));
            a0.setBufferIndex(0);
            let a1 = attrs.objectAtIndexedSubscript(1);
            a1.setFormat(MTLVertexFormat::Float2);
            a1.setOffset(std::mem::offset_of!(DrawVert, uv));
            a1.setBufferIndex(0);
            let a2 = attrs.objectAtIndexedSubscript(2);
            a2.setFormat(MTLVertexFormat::UChar4);
            a2.setOffset(std::mem::offset_of!(DrawVert, col));
            a2.setBufferIndex(0);
            let layout = vd.layouts().objectAtIndexedSubscript(0);
            layout.setStepRate(1);
            layout.setStepFunction(MTLVertexStepFunction::PerVertex);
            layout.setStride(stride);
        }
        desc.setVertexDescriptor(Some(&vd));
        desc.setRasterSampleCount(key.sample_count);

        // SAFETY: color attachment 0 exists.
        let color = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
        color.setPixelFormat(MTLPixelFormat(key.color_format));
        color.setBlendingEnabled(true);
        color.setRgbBlendOperation(MTLBlendOperation::Add);
        color.setAlphaBlendOperation(MTLBlendOperation::Add);
        color.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        color.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        color.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        desc.setDepthAttachmentPixelFormat(MTLPixelFormat(key.depth_format));
        desc.setStencilAttachmentPixelFormat(MTLPixelFormat(key.stencil_format));

        let state = self
            .device
            .newRenderPipelineStateWithDescriptor_error(&desc)
            .map_err(|e| e.localizedDescription().to_string())?;
        self.pipelines.insert(key, state.clone());
        Ok(state)
    }

    /// Create (or recreate) the texture of `key` from a full upload.
    fn create_texture(
        &mut self,
        key: SnapshotTextureId,
        format: TextureFormat,
        width: u32,
        height: u32,
        row_pitch: usize,
        pixels: &[u8],
    ) -> Result<TextureId, String> {
        let rgba = to_rgba(format, width as usize, height as usize, row_pitch, pixels)?;

        let desc = MTLTextureDescriptor::new();
        desc.setPixelFormat(MTLPixelFormat::RGBA8Unorm);
        // SAFETY: plain numbers validated by texture creation.
        unsafe {
            desc.setWidth(width as usize);
            desc.setHeight(height as usize);
        }
        let texture = self
            .device
            .newTextureWithDescriptor(&desc)
            .ok_or("failed to create the texture")?;
        replace_region(&texture, 0, 0, width as usize, height as usize, &rgba)?;

        let id = if let Some((id, _)) = self.textures.get(&key) {
            *id
        } else {
            let id = self.next_texture_id;
            self.next_texture_id += 1;
            id
        };
        self.textures.insert(key, (id, texture));
        Ok(TextureId::new(id))
    }

    fn process_request(&mut self, request: &TextureRequest) -> Result<TextureFeedback, String> {
        let key = request.texture();
        match request.operation() {
            TextureOp::Create {
                format,
                width,
                height,
                row_pitch,
                pixels,
            } => {
                let id = self.create_texture(key, *format, *width, *height, *row_pitch, pixels)?;
                request.uploaded(id).map_err(|e| e.to_string())
            }
            TextureOp::Update {
                format,
                width: _,
                height: _,
                rects,
            } => {
                let Some((id, texture)) = self.textures.get(&key) else {
                    // The texture was never created or was destroyed.
                    return Ok(request.superseded());
                };
                for r in rects {
                    let (w, h) = (usize::from(r.rect.w), usize::from(r.rect.h));
                    let rgba = to_rgba(*format, w, h, r.row_pitch, &r.data)?;
                    replace_region(
                        texture,
                        usize::from(r.rect.x),
                        usize::from(r.rect.y),
                        w,
                        h,
                        &rgba,
                    )?;
                }
                request
                    .uploaded(TextureId::new(*id))
                    .map_err(|e| e.to_string())
            }
            TextureOp::Destroy => {
                self.textures.remove(&key);
                request.destroyed().map_err(|e| e.to_string())
            }
        }
    }

    /// Reconcile the managed textures of the frame and draw it into
    /// `encoder` (`ImGui_ImplMetal_RenderDrawData`).
    fn render(
        &mut self,
        frame: PendingFrame<'_>,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        desc: &MTLRenderPassDescriptor,
    ) -> Result<(), String> {
        let mut feedback = Vec::with_capacity(frame.texture_requests().len());
        for request in frame.texture_requests() {
            feedback.push(self.process_request(request)?);
        }
        let frame = frame
            .reconcile_texture_feedback(feedback)
            .map_err(|e| e.to_string())?;
        let draw_data = frame.draw_data();

        let [fb_w, fb_h] = {
            let size = draw_data.display_size();
            let scale = draw_data.framebuffer_scale();
            [size[0] * scale[0], size[1] * scale[1]]
        };
        // Avoid rendering when minimized.
        if fb_w <= 0.0 || fb_h <= 0.0 || draw_data.draw_lists_count() == 0 {
            return Ok(());
        }

        // The pipeline matches the render target the host gave us.
        // SAFETY: attachment 0 exists.
        let color_tex = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) }.texture();
        let depth_tex = desc.depthAttachment().texture();
        let stencil_tex = desc.stencilAttachment().texture();
        let key = PipelineKey {
            color_format: color_tex
                .as_ref()
                .map_or(MTLPixelFormat::BGRA8Unorm.0, |t| t.pixelFormat().0),
            sample_count: color_tex.as_ref().map_or(1, |t| t.sampleCount()),
            depth_format: depth_tex
                .as_ref()
                .map_or(MTLPixelFormat::Invalid.0, |t| t.pixelFormat().0),
            stencil_format: stencil_tex
                .as_ref()
                .map_or(MTLPixelFormat::Invalid.0, |t| t.pixelFormat().0),
        };
        let pipeline = self.pipeline(key)?;

        let encoder = command_buffer
            .renderCommandEncoderWithDescriptor(desc)
            .ok_or("failed to create the render command encoder")?;

        // Render state.
        encoder.setCullMode(MTLCullMode::None);
        encoder.setDepthStencilState(Some(&self.depth_state));
        encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: f64::from(fb_w),
            height: f64::from(fb_h),
            znear: 0.0,
            zfar: 1.0,
        });
        encoder.setRenderPipelineState(&pipeline);

        // The orthographic projection of the display.
        let [dx, dy] = draw_data.display_pos();
        let [dw, dh] = draw_data.display_size();
        let (l, r, t, b) = (dx, dx + dw, dy, dy + dh);
        #[rustfmt::skip]
        let projection: [[f32; 4]; 4] = [
            [2.0 / (r - l), 0.0, 0.0, 0.0],
            [0.0, 2.0 / (t - b), 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [(r + l) / (l - r), (t + b) / (b - t), 0.0, 1.0],
        ];
        // SAFETY: the pointer/length describe `projection`, which outlives
        // the call (Metal copies the bytes).
        unsafe {
            encoder.setVertexBytes_length_atIndex(
                NonNull::from(&projection).cast::<c_void>(),
                size_of::<[[f32; 4]; 4]>(),
                1,
            );
        }

        let clip_off = draw_data.display_pos();
        let clip_scale = draw_data.framebuffer_scale();
        for list in draw_data.draw_lists() {
            let vertices = list.vtx_buffer();
            let indices = list.idx_buffer();
            if vertices.is_empty() || indices.is_empty() {
                continue;
            }
            let vbuf =
                self.new_buffer(vertices.as_ptr().cast(), std::mem::size_of_val(vertices))?;
            let ibuf = self.new_buffer(indices.as_ptr().cast(), std::mem::size_of_val(indices))?;
            // SAFETY: slot 0 is the vertex stream.
            unsafe { encoder.setVertexBuffer_offset_atIndex(Some(&vbuf), 0, 0) };

            for cmd in list.commands() {
                match cmd {
                    DrawCmd::Elements { count, cmd_params } => {
                        // Project the scissor rectangle into framebuffer
                        // space and clamp it to the framebuffer.
                        let cr = cmd_params.clip_rect;
                        let min_x = ((cr[0] - clip_off[0]) * clip_scale[0]).max(0.0);
                        let min_y = ((cr[1] - clip_off[1]) * clip_scale[1]).max(0.0);
                        let max_x = ((cr[2] - clip_off[0]) * clip_scale[0]).min(fb_w);
                        let max_y = ((cr[3] - clip_off[1]) * clip_scale[1]).min(fb_h);
                        if max_x <= min_x || max_y <= min_y {
                            continue;
                        }
                        encoder.setScissorRect(MTLScissorRect {
                            x: min_x as usize,
                            y: min_y as usize,
                            width: (max_x - min_x) as usize,
                            height: (max_y - min_y) as usize,
                        });

                        // Bind the texture of the command.
                        let id = cmd_params.texture_id.id();
                        let Some((_, tex)) = self.textures.values().find(|(tid, _)| *tid == id)
                        else {
                            continue;
                        };
                        // SAFETY: slot 0 of the fragment stage.
                        unsafe { encoder.setFragmentTexture_atIndex(Some(tex), 0) };

                        // The vertex offset applies through the buffer
                        // offset (`RENDERER_HAS_VTX_OFFSET`).
                        // SAFETY: the offset stays inside the buffer.
                        unsafe {
                            encoder.setVertexBufferOffset_atIndex(
                                cmd_params.vtx_offset * size_of::<DrawVert>(),
                                0,
                            );
                            encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                                MTLPrimitiveType::Triangle,
                                count,
                                if size_of::<DrawIdx>() == 2 {
                                    MTLIndexType::UInt16
                                } else {
                                    MTLIndexType::UInt32
                                },
                                &ibuf,
                                cmd_params.idx_offset * size_of::<DrawIdx>(),
                            );
                        }
                    }
                    // Nothing to reset: state is set per command.
                    DrawCmd::ResetRenderState
                    | DrawCmd::SetSamplerLinear
                    | DrawCmd::SetSamplerNearest => {}
                    DrawCmd::RawCallback(_) => {
                        log::warn!("ignoring a raw imgui draw callback");
                    }
                }
            }
        }

        encoder.endEncoding();
        Ok(())
    }

    fn new_buffer(
        &self,
        ptr: *const c_void,
        len: usize,
    ) -> Result<Retained<ProtocolObject<dyn objc2_metal::MTLBuffer>>, String> {
        let ptr = NonNull::new(ptr.cast_mut()).ok_or("null buffer")?;
        // SAFETY: `ptr` points to `len` readable bytes (a slice of the draw
        // list); Metal copies them.
        unsafe {
            self.device.newBufferWithBytes_length_options(
                ptr,
                len,
                MTLResourceOptions::StorageModeShared,
            )
        }
        .ok_or_else(|| "failed to create a buffer".to_owned())
    }
}

/// The pixels as tightly packed RGBA8 (alpha-only textures become white with
/// that alpha).
fn to_rgba(
    format: TextureFormat,
    width: usize,
    height: usize,
    row_pitch: usize,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let bpp = match format {
        TextureFormat::RGBA32 => 4,
        TextureFormat::Alpha8 => 1,
    };
    let pitch = if row_pitch == 0 {
        width * bpp
    } else {
        row_pitch
    };
    if height > 0 && data.len() < pitch * (height - 1) + width * bpp {
        return Err("texture upload is too short".to_owned());
    }
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = &data[y * pitch..y * pitch + width * bpp];
        match format {
            TextureFormat::RGBA32 => out.extend_from_slice(row),
            TextureFormat::Alpha8 => {
                for a in row {
                    out.extend_from_slice(&[255, 255, 255, *a]);
                }
            }
        }
    }
    Ok(out)
}

fn replace_region(
    texture: &ProtocolObject<dyn MTLTexture>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    rgba: &[u8],
) -> Result<(), String> {
    let ptr = NonNull::new(rgba.as_ptr().cast_mut().cast::<c_void>()).ok_or("empty texture")?;
    let region = MTLRegion {
        origin: MTLOrigin { x, y, z: 0 },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
    };
    // SAFETY: `rgba` has `width * 4 * height` bytes (built by `to_rgba`).
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, ptr, width * 4);
    }
    Ok(())
}

/// Render the inspector (Zig: `Inspector.renderMetal`). Takes ownership of
/// one reference each to `command_buffer` and `descriptor`.
///
/// # Safety
/// Both must be retained Metal objects.
pub unsafe fn render(
    inspector: &mut Inspector,
    command_buffer: *mut c_void,
    descriptor: *mut c_void,
) {
    // SAFETY: per the function contract we own one reference to each; they
    // are released when these `Retained`s drop.
    let command_buffer: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> =
        unsafe { Retained::from_raw(command_buffer.cast()) };
    // SAFETY: as above.
    let descriptor: Option<Retained<MTLRenderPassDescriptor>> =
        unsafe { Retained::from_raw(descriptor.cast()) };
    let (Some(command_buffer), Some(descriptor)) = (command_buffer, descriptor) else {
        return;
    };
    if let Err(err) = render_frames(inspector, &command_buffer, &descriptor) {
        log::error!("error rendering inspector err={err}");
    }
}

fn render_frames(
    inspector: &mut Inspector,
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    descriptor: &MTLRenderPassDescriptor,
) -> Result<(), String> {
    let mut backend = inspector
        .take_backend()
        .ok_or("the Metal backend isn't initialized")?;
    let result = draw_frame(&mut backend, inspector, command_buffer, descriptor);
    inspector.set_backend(backend);
    result
}

fn draw_frame(
    backend: &mut MetalBackend,
    inspector: &mut Inspector,
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    descriptor: &MTLRenderPassDescriptor,
) -> Result<(), String> {
    // Setup our imgui frame. We need to render multiple frames to ensure
    // ImGui completes all its state processing. I don't know how to fix
    // this.
    inspector.build_frame();
    inspector.context().end_frame();

    inspector.build_frame();
    let pending = inspector.context().render(&backend.consumer);
    backend.render(pending, command_buffer, descriptor)
}
