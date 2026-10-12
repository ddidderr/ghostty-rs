//! Wrapper for handling textures (`metal/Texture.zig`).
use std::{ffi::c_void, ptr::NonNull};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLDevice as _,
    MTLOrigin,
    MTLPixelFormat,
    MTLRegion,
    MTLResourceOptions,
    MTLSize,
    MTLTexture,
    MTLTextureDescriptor,
    MTLTextureUsage,
};

use super::{Device, Error};
use crate::{
    graphics_api::{ApiError, GpuTexture},
    metal_layout::{PixelFormat, ResourceOptions, TextureUsage},
};

/// Options for initializing a texture.
#[derive(Clone)]
pub struct Options {
    /// MTLDevice
    pub device: Device,
    pub pixel_format: PixelFormat,
    pub resource_options: ResourceOptions,
    pub usage: TextureUsage,
}

/// A 2D texture.
pub struct Texture {
    /// The underlying MTLTexture Object.
    texture: Retained<ProtocolObject<dyn MTLTexture>>,

    /// The width of this texture.
    pub width: usize,
    /// The height of this texture.
    pub height: usize,

    /// Bytes per pixel for this texture.
    pub bpp: usize,
}

// SAFETY: MTLTexture objects may be used from any thread (the CPU side only
// calls `replaceRegion`, which the renderer serializes through its draw
// mutex).
unsafe impl Send for Texture {}
// SAFETY: see above.
unsafe impl Sync for Texture {}

impl Texture {
    /// The underlying MTLTexture.
    pub fn texture(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }

    /// Initialize a texture
    pub fn init(
        opts: &Options,
        width: usize,
        height: usize,
        data: Option<&[u8]>,
    ) -> Result<Texture, Error> {
        // Create our descriptor and set our properties
        let desc = MTLTextureDescriptor::new();
        desc.setPixelFormat(MTLPixelFormat(opts.pixel_format as usize));
        // SAFETY: plain numbers, validated when the texture is created.
        unsafe {
            desc.setWidth(width);
            desc.setHeight(height);
        }
        desc.setResourceOptions(MTLResourceOptions(opts.resource_options.bits() as usize));
        desc.setUsage(MTLTextureUsage(opts.usage.bits() as usize));

        // Initialize
        let texture = opts
            .device
            .newTextureWithDescriptor(&desc)
            .ok_or(Error::MetalFailed)?;

        let this = Texture {
            texture,
            width,
            height,
            bpp: opts.pixel_format.bytes_per_pixel(),
        };

        // If we have data, we set it here.
        if let Some(d) = data {
            debug_assert_eq!(d.len(), width * height * this.bpp);
            this.replace_region(0, 0, width, height, d)?;
        }

        Ok(this)
    }

    /// Replace a region of the texture with the provided data.
    ///
    /// Zig does NOT check the dimensions of the data to ensure correctness;
    /// since this is a safe function we reject data that is too short.
    pub fn replace_region(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        data: &[u8],
    ) -> Result<(), Error> {
        let bytes_per_row = self.bpp * width;
        if data.len() < bytes_per_row * height {
            return Err(Error::MetalFailed);
        }
        let Some(ptr) = NonNull::new(data.as_ptr().cast_mut().cast::<c_void>()) else {
            return Err(Error::MetalFailed);
        };
        let region = MTLRegion {
            origin: MTLOrigin { x, y, z: 0 },
            size: MTLSize {
                width,
                height,
                depth: 1,
            },
        };
        // SAFETY: `ptr` points to at least `bytes_per_row * height` readable
        // bytes (checked above); Metal only reads from it.
        unsafe {
            self.texture
                .replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, ptr, bytes_per_row);
        }
        Ok(())
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
