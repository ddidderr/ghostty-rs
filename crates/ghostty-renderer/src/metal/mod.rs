//! Graphics API wrapper for Metal (`renderer/Metal.zig` and `renderer/metal/*`).
//!
//! The renderer draws into IOSurface-backed textures ([`target::Target`]) and
//! presents them by setting the `contents` of an [`IOSurfaceLayer`], which is
//! installed on the apprt's NSView (macOS: made "layer-hosting") or UIView
//! (iOS: added as a sublayer).
//!
//! The platform-independent parts (enum values, GPU data layouts, pixel
//! format selection, vertex attribute tables) live in
//! [`crate::metal_layout`] so they can be unit tested on any host.
//!
//! Safety: this module talks to Metal / QuartzCore through `objc2`, which is
//! mostly safe; `unsafe` is limited to encoder calls that take raw resources,
//! raw pointer copies and the autorelease pool, each with a SAFETY comment.

use std::{ffi::c_void, sync::Arc};

use bytemuck::Pod;
use ghostty_config::types_b::AlphaBlending;
use ghostty_font::atlas::Atlas;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_foundation::NSError;
use objc2_metal::{
    MTLBuffer,
    MTLCommandQueue,
    MTLDevice,
    MTLGPUFamily,
    MTLSamplerAddressMode,
    MTLSamplerMinMagFilter,
};

pub mod buffer;
pub mod frame;
pub mod io_surface_layer;
pub mod pipeline;
pub mod render_pass;
pub mod sampler;
pub mod shaders;
pub mod target;
pub mod texture;

pub use self::{
    buffer::Buffer,
    frame::Frame,
    io_surface_layer::IOSurfaceLayer,
    pipeline::Pipeline,
    render_pass::RenderPass,
    sampler::Sampler,
    shaders::Shaders,
    target::Target,
    texture::Texture,
};
use crate::{
    graphics_api::{ApiError, GraphicsApi, ImageTextureFormat, TextureKind},
    metal_layout::{
        MetalUniforms,
        PixelFormat,
        ResourceOptions,
        StorageMode,
        TextureUsage,
        atlas_pixel_format,
        color_pixel_format,
        default_storage_mode,
        image_pixel_format,
        max_texture_size,
        surface_size,
    },
    shader_types::Uniforms,
    shadertoy,
};

/// A `MTLDevice`.
pub type Device = Retained<ProtocolObject<dyn MTLDevice>>;
/// A `MTLCommandQueue`.
pub type Queue = Retained<ProtocolObject<dyn MTLCommandQueue>>;

/// Custom shaders are converted to MSL for Metal.
pub const CUSTOM_SHADER_TARGET: shadertoy::Target = shadertoy::Target::Msl;
/// The fragCoord for Metal shaders is +Y = down.
pub const CUSTOM_SHADER_Y_IS_DOWN: bool = true;
/// Triple buffering.
pub const SWAP_CHAIN_COUNT: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A Metal API call failed.
    #[error("Metal API call failed")]
    MetalFailed,
    /// No usable Metal device exists.
    #[error("no Metal device")]
    NoMetalDevice,
    /// An Objective-C runtime operation failed.
    #[error("Objective-C runtime failure")]
    ObjCFailed,
    /// A shader function was not found in its library.
    #[error("shader function `{0}` not found")]
    MissingFunction(String),
    /// Metal reported an error (shader compilation, pipeline creation, ...).
    #[error("metal error: {0}")]
    Message(String),
}

impl Error {
    /// Log and convert an `NSError` (`checkError`).
    pub(crate) fn from_ns_error(err: &NSError) -> Error {
        let desc = err.localizedDescription().to_string();
        log::error!("metal error={desc}");
        Error::Message(desc)
    }
}

/// The NSView (macOS) / UIView (iOS) the renderer draws in, plus its content
/// scale (Zig: `rt_surface.platform` and `rt_surface.content_scale.x`).
pub struct ViewInfo<'a> {
    /// The NSView / UIView.
    pub view: &'a AnyObject,
    /// The scale factor of the window (backing scale factor).
    pub scale_factor: f64,
}

/// An autorelease pool token (`objc_autoreleasePoolPush`).
struct PoolToken(*mut c_void);

// SAFETY: the token is only used from the thread that pushed it (draw_frame
// starts and ends the pool on one thread); `Send` is needed because the
// graphics API lives inside the renderer's mutex.
unsafe impl Send for PoolToken {}

unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// The Metal graphics API.
pub struct Metal {
    layer: IOSurfaceLayer,

    /// MTLDevice
    device: Device,
    /// MTLCommandQueue
    queue: Queue,

    /// Alpha blending mode
    blending: AlphaBlending,

    /// The default storage mode to use for resources created with our device.
    ///
    /// This is based on whether the device is a discrete GPU or not, since
    /// discrete GPUs do not have unified memory and therefore do not support
    /// the "shared" storage mode, instead we have to use the "managed" mode.
    default_storage_mode: StorageMode,

    /// The maximum 2D texture width and height supported by the device.
    max_texture_size: u32,

    /// We start an AutoreleasePool before `drawFrame` and end it afterwards.
    autorelease_pool: Option<PoolToken>,
}

impl Metal {
    /// Create the Metal backend and attach its layer to `view`.
    ///
    /// Must be called on the thread that owns the view (the main thread).
    pub fn init(blending: AlphaBlending, info: &ViewInfo<'_>) -> Result<Metal, Error> {
        // Choose our MTLDevice and create a MTLCommandQueue for that device.
        let device = choose_device()?;
        let queue = device.newCommandQueue().ok_or(Error::MetalFailed)?;

        // Grab metadata about the device.
        let has_unified_memory = device.hasUnifiedMemory();
        let default_storage_mode =
            default_storage_mode(cfg!(target_os = "ios"), has_unified_memory);
        let max_texture_size = query_max_texture_size(&device);
        log::debug!(
            "device properties default_storage_mode={default_storage_mode:?} max_texture_size={max_texture_size}"
        );

        // Create an IOSurfaceLayer which we can assign to the view to make
        // it in to a "layer-hosting view", so that we can manually control
        // the layer contents.
        let layer = IOSurfaceLayer::new()?;
        let layer_object: &AnyObject = layer.layer().as_ref();
        let view = info.view;

        // Add our layer to the view.
        //
        // On macOS we do this by making the view "layer-hosting"
        // by assigning it to the view's `layer` property BEFORE
        // setting `wantsLayer` to `true`.
        //
        // On iOS, views are always layer-backed, and `layer`
        // is readonly, so instead we add it as a sublayer.
        //
        // SAFETY: `view` is an NSView / UIView (a documented requirement of
        // `ViewInfo`), which implements these selectors with these types.
        unsafe {
            #[cfg(target_os = "macos")]
            {
                let _: () = msg_send![view, setLayer: layer_object];
                let _: () = msg_send![view, setWantsLayer: true];
            }
            #[cfg(not(target_os = "macos"))]
            {
                let view_layer: Option<Retained<AnyObject>> = msg_send![view, layer];
                let view_layer = view_layer.ok_or(Error::ObjCFailed)?;
                let _: () = msg_send![&*view_layer, addSublayer: layer_object];
            }

            // Ensure that if our layer is oversized it does not overflow the
            // bounds of the view. (`clipsToBounds` is NSView API since macOS
            // 14; older systems don't have the setter.)
            let responds: bool = msg_send![view, respondsToSelector: sel!(setClipsToBounds:)];
            if responds {
                let _: () = msg_send![view, setClipsToBounds: true];
            }
        }

        // Ensure that our layer has a content scale set to
        // match the scale factor of the window. This avoids
        // magnification issues leading to blurry rendering.
        layer.layer().setContentsScale(info.scale_factor);

        // This makes it so that our display callback will actually be called.
        layer.layer().setNeedsDisplayOnBoundsChange(true);

        Ok(Metal {
            layer,
            device,
            queue,
            blending,
            default_storage_mode,
            max_texture_size,
            autorelease_pool: None,
        })
    }

    /// The layer the renderer presents on.
    pub fn layer(&self) -> &IOSurfaceLayer {
        &self.layer
    }

    /// The device the renderer draws with.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The default storage mode of resources created with this device.
    pub fn default_storage_mode(&self) -> StorageMode {
        self.default_storage_mode
    }

    /// The maximum 2D texture size of the device.
    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    /// The pixel format of targets, custom shader textures and pipelines.
    fn color_format(&self) -> PixelFormat {
        color_pixel_format(self.blending)
    }

    /// Returns the options to use when constructing buffers.
    pub fn buffer_options(&self) -> buffer::Options {
        buffer::Options {
            device: self.device.clone(),
            // Indicate that the CPU writes to this resource but never reads it.
            resource_options: ResourceOptions::write_combined(self.default_storage_mode),
        }
    }

    /// Returns the options to use when constructing textures.
    pub fn texture_options(&self) -> texture::Options {
        texture::Options {
            device: self.device.clone(),
            // Using an `*_srgb` pixel format makes Metal gamma encode the
            // pixels written to it *after* blending, which means we get linear
            // alpha blending rather than gamma-incorrect blending.
            pixel_format: self.color_format(),
            resource_options: ResourceOptions::write_combined(self.default_storage_mode),
            // textureOptions is currently only used for custom shaders.
            usage: TextureUsage::custom_shader(),
        }
    }

    /// Returns the options to use when constructing samplers.
    pub fn sampler_options(&self) -> sampler::Options {
        sampler::Options {
            device: self.device.clone(),

            // These parameters match Shadertoy behaviors.
            min_filter: MTLSamplerMinMagFilter::Linear,
            mag_filter: MTLSamplerMinMagFilter::Linear,
            s_address_mode: MTLSamplerAddressMode::ClampToEdge,
            t_address_mode: MTLSamplerAddressMode::ClampToEdge,
        }
    }

    /// Returns the options to use when constructing textures for images.
    pub fn image_texture_options(
        &self,
        format: ImageTextureFormat,
        srgb: bool,
    ) -> texture::Options {
        texture::Options {
            device: self.device.clone(),
            pixel_format: image_pixel_format(format, srgb),
            resource_options: ResourceOptions::write_combined(self.default_storage_mode),
            // We only need to read from this texture from a shader.
            usage: TextureUsage::shader_read_only(),
        }
    }

    /// Initialize a new render target which can be presented by this API.
    pub fn init_target(&self, width: usize, height: usize) -> Result<Target, Error> {
        Target::init(&target::Options {
            device: self.device.clone(),
            // Using an `*_srgb` pixel format makes Metal gamma encode the
            // pixels written to it *after* blending, which means we get linear
            // alpha blending rather than gamma-incorrect blending.
            pixel_format: self.color_format(),
            storage_mode: self.default_storage_mode,
            width,
            height,
        })
    }

    /// Initializes a Texture suitable for the provided font atlas.
    pub fn init_atlas_texture(&self, atlas: &Atlas) -> Result<Texture, Error> {
        let pixel_format =
            atlas_pixel_format(atlas.format).expect("unsupported atlas format for Metal texture");

        Texture::init(
            &texture::Options {
                device: self.device.clone(),
                pixel_format,
                resource_options: ResourceOptions::write_combined(self.default_storage_mode),
                // We only need to read from this texture from a shader.
                usage: TextureUsage::shader_read_only(),
            },
            atlas.size as usize,
            atlas.size as usize,
            None,
        )
    }

    /// Get the current size of the runtime surface.
    pub fn surface_size(&self) -> (u32, u32) {
        let (width, height, scale) = self.layer.bounds_and_scale();

        // We need to clamp our runtime surface size to the maximum
        // possible texture size since we can't create a screen buffer
        // (texture) larger than that.
        surface_size(width, height, scale, self.max_texture_size)
    }
}

impl Drop for Metal {
    fn drop(&mut self) {
        // Nothing may call back into a renderer that is going away.
        self.layer.set_display_callback(None);
        if let Some(pool) = self.autorelease_pool.take() {
            // SAFETY: the pool was pushed by `draw_frame_start` on this thread.
            unsafe { objc_autoreleasePoolPop(pool.0) };
        }
    }
}

/// Determines the maximum 2D texture size supported by the device.
/// We need to clamp our frame size to this if it's larger.
fn query_max_texture_size(device: &ProtocolObject<dyn MTLDevice>) -> u32 {
    // https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf
    max_texture_size(
        device.supportsFamily(MTLGPUFamily::Apple10),
        device.supportsFamily(MTLGPUFamily::Apple3),
    )
}

#[cfg(target_os = "macos")]
fn choose_device() -> Result<Device, Error> {
    let devices = objc2_metal::MTLCopyAllDevices();

    let mut chosen_device: Option<Device> = None;
    for device in &devices {
        // We want a GPU that’s connected to a display.
        if device.isHeadless() {
            continue;
        }
        chosen_device = Some(device.clone());
        // If the user has an eGPU plugged in, they probably want
        // to use it. Otherwise, integrated GPUs are better for
        // battery life and thermals.
        if device.isRemovable() || device.isLowPower() {
            break;
        }
    }

    chosen_device.ok_or(Error::NoMetalDevice)
}

#[cfg(not(target_os = "macos"))]
fn choose_device() -> Result<Device, Error> {
    objc2_metal::MTLCreateSystemDefaultDevice().ok_or(Error::NoMetalDevice)
}

impl GraphicsApi for Metal {
    const CUSTOM_SHADER_TARGET: shadertoy::Target = CUSTOM_SHADER_TARGET;
    const CUSTOM_SHADER_Y_IS_DOWN: bool = CUSTOM_SHADER_Y_IS_DOWN;
    const SWAP_CHAIN_COUNT: usize = SWAP_CHAIN_COUNT;

    type Target = Target;
    type Texture = Texture;
    type Sampler = Sampler;
    type Buffer<T: Pod> = Buffer<T>;
    type BufferRef = Retained<ProtocolObject<dyn MTLBuffer>>;
    type Pipeline = Pipeline;
    type Shaders = Shaders;
    type GpuUniforms = MetalUniforms;
    type Frame = Frame;
    type RenderPass<'a> = RenderPass;

    fn gpu_uniforms(uniforms: &Uniforms) -> MetalUniforms {
        MetalUniforms::from(uniforms)
    }

    fn blending(&self) -> AlphaBlending {
        self.blending
    }

    fn set_blending(&mut self, blending: AlphaBlending) {
        self.blending = blending;
    }

    fn init_shaders(&self, custom_shaders: &[String]) -> Result<Shaders, ApiError> {
        Shaders::init(
            &self.device,
            custom_shaders,
            // Using an `*_srgb` pixel format makes Metal gamma encode the
            // pixels written to it *after* blending, which means we get linear
            // alpha blending rather than gamma-incorrect blending.
            self.color_format(),
        )
        .map_err(ApiError::new)
    }

    fn surface_size(&self) -> Result<(u32, u32), ApiError> {
        Ok(Metal::surface_size(self))
    }

    fn init_target(&self, width: usize, height: usize) -> Result<Target, ApiError> {
        Metal::init_target(self, width, height).map_err(ApiError::new)
    }

    fn init_buffer<T: Pod>(&self, len: usize) -> Result<Buffer<T>, ApiError> {
        Buffer::init(self.buffer_options(), len).map_err(ApiError::new)
    }

    fn init_buffer_fill<T: Pod>(&self, data: &[T]) -> Result<Buffer<T>, ApiError> {
        Buffer::init_fill(self.buffer_options(), data).map_err(ApiError::new)
    }

    fn init_texture(
        &self,
        kind: TextureKind,
        width: usize,
        height: usize,
        data: Option<&[u8]>,
    ) -> Result<Texture, ApiError> {
        let opts = match kind {
            TextureKind::CustomShader => self.texture_options(),
            TextureKind::Image { format, srgb } => self.image_texture_options(format, srgb),
        };
        Texture::init(&opts, width, height, data).map_err(ApiError::new)
    }

    fn init_atlas_texture(&self, atlas: &Atlas) -> Result<Texture, ApiError> {
        Metal::init_atlas_texture(self, atlas).map_err(ApiError::new)
    }

    fn init_sampler(&self) -> Result<Sampler, ApiError> {
        Sampler::init(&self.sampler_options()).map_err(ApiError::new)
    }

    fn begin_frame(&self, target: Arc<Target>) -> Frame {
        Frame::begin(&self.queue, self.layer.clone(), target)
    }

    /// Present the last presented target again. (noop for Metal)
    fn present_last_target(&self) -> Result<(), ApiError> {
        Ok(())
    }

    fn clear_last_target(&self) {}

    /// Actions taken before doing anything in `drawFrame`.
    ///
    /// Right now we use this to start an AutoreleasePool.
    fn draw_frame_start(&mut self) {
        debug_assert!(self.autorelease_pool.is_none());
        // SAFETY: balanced with the pop in `draw_frame_end` / `Drop`.
        self.autorelease_pool = Some(PoolToken(unsafe { objc_autoreleasePoolPush() }));
    }

    /// Actions taken after `drawFrame` is done.
    ///
    /// Right now we use this to end our AutoreleasePool.
    fn draw_frame_end(&mut self) {
        debug_assert!(self.autorelease_pool.is_some());
        if let Some(pool) = self.autorelease_pool.take() {
            // SAFETY: the pool was pushed by `draw_frame_start` on this thread.
            unsafe { objc_autoreleasePoolPop(pool.0) };
        }
    }

    fn loop_enter(&mut self, draw: Arc<dyn Fn() + Send + Sync>) {
        self.layer.set_display_callback(Some(draw));
    }
}
