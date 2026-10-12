//! Represents a render target (`metal/Target.zig`).
//!
//! In this case, an IOSurface-backed MTLTexture.
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceDisplayP3};
use objc2_io_surface::{
    IOSurfaceRef,
    kIOSurfaceBytesPerElement,
    kIOSurfaceColorSpace,
    kIOSurfaceHeight,
    kIOSurfacePixelFormat,
    kIOSurfaceWidth,
};
use objc2_metal::{
    MTLDevice as _,
    MTLPixelFormat,
    MTLResourceOptions,
    MTLTexture,
    MTLTextureDescriptor,
    MTLTextureUsage,
};

use super::{Device, Error};
use crate::{
    graphics_api::GpuTarget,
    metal_layout::{PixelFormat, ResourceOptions, StorageMode, TextureUsage},
};

/// `kCVPixelFormatType_32BGRA` ('BGRA'): `video.PixelFormat.@"32BGRA"`.
pub const PIXEL_FORMAT_32BGRA: i32 = 0x4247_5241;

/// `kIOSurfacePurgeableEmpty`.
const IOSURFACE_PURGEABLE_EMPTY: u32 = 2;

/// Options for initializing a Target
pub struct Options {
    /// MTLDevice
    pub device: Device,

    /// Desired width
    pub width: usize,
    /// Desired height
    pub height: usize,

    /// Pixel format for the MTLTexture
    pub pixel_format: PixelFormat,
    /// Storage mode for the MTLTexture
    pub storage_mode: StorageMode,
}

/// An IOSurface-backed render target.
pub struct Target {
    /// The underlying IOSurface.
    surface: CFRetained<IOSurfaceRef>,

    /// The underlying MTLTexture.
    texture: Retained<ProtocolObject<dyn MTLTexture>>,

    /// Current width of this target.
    pub width: usize,
    /// Current height of this target.
    pub height: usize,
}

// SAFETY: IOSurfaces and MTLTextures are thread-safe reference counted
// objects; the target is only mutated by the GPU.
unsafe impl Send for Target {}
// SAFETY: see above.
unsafe impl Sync for Target {}

impl Target {
    /// The underlying IOSurface.
    pub fn surface(&self) -> &CFRetained<IOSurfaceRef> {
        &self.surface
    }

    /// The underlying MTLTexture.
    pub fn texture(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }

    pub fn init(opts: &Options) -> Result<Target, Error> {
        // We set our surface's color space to Display P3.
        // This allows us to have "Apple-style" alpha blending,
        // since it seems to be the case that Apple apps like
        // Terminal and TextEdit render text in the display's
        // color space using converted colors, which reduces,
        // but does not fully eliminate blending artifacts.
        //
        // SAFETY: a constant CFString exported by CoreGraphics.
        let colorspace = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceDisplayP3 }))
            .ok_or(Error::MetalFailed)?;

        let surface = create_surface(opts.width, opts.height, &colorspace)?;

        // Create our descriptor and set our properties
        let desc = MTLTextureDescriptor::new();
        // SAFETY: plain numbers, validated when the texture is created.
        unsafe {
            desc.setWidth(opts.width);
            desc.setHeight(opts.height);
        }
        desc.setPixelFormat(MTLPixelFormat(opts.pixel_format as usize));
        desc.setUsage(MTLTextureUsage(
            TextureUsage {
                render_target: true,
                ..TextureUsage::default()
            }
            .bits() as usize,
        ));
        desc.setResourceOptions(MTLResourceOptions(
            // Indicate that the CPU writes to this resource but never reads it.
            ResourceOptions::write_combined(opts.storage_mode).bits() as usize,
        ));

        let texture = opts
            .device
            .newTextureWithDescriptor_iosurface_plane(&desc, &surface, 0)
            .ok_or(Error::MetalFailed)?;

        Ok(Target {
            surface,
            texture,
            width: opts.width,
            height: opts.height,
        })
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        // We mark it purgeable so that it is immediately unloaded, so that we
        // don't have to wait for CoreFoundation garbage collection to trigger.
        //
        // SAFETY: a null `old_state` is allowed.
        unsafe {
            self.surface
                .set_purgeable(IOSURFACE_PURGEABLE_EMPTY, std::ptr::null_mut());
        }
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

/// `IOSurface.init`: a BGRA IOSurface of the given size with a color space.
fn create_surface(
    width: usize,
    height: usize,
    colorspace: &CGColorSpace,
) -> Result<CFRetained<IOSurfaceRef>, Error> {
    let w = CFNumber::new_i32(i32::try_from(width).map_err(|_| Error::MetalFailed)?);
    let h = CFNumber::new_i32(i32::try_from(height).map_err(|_| Error::MetalFailed)?);
    let pf = CFNumber::new_i32(PIXEL_FORMAT_32BGRA);
    let bpe = CFNumber::new_i32(4);

    // SAFETY: constant CFStrings exported by IOSurface.
    let keys: [&CFString; 4] = unsafe {
        [
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfacePixelFormat,
            kIOSurfaceBytesPerElement,
        ]
    };
    let values: [&CFNumber; 4] = [&w, &h, &pf, &bpe];
    let properties = CFDictionary::<CFString, CFNumber>::from_slices(&keys, &values);

    // SAFETY: the dictionary has the CFString keys / CFNumber values
    // IOSurfaceCreate expects.
    let surface = unsafe { IOSurfaceRef::new(properties.as_opaque()) }.ok_or(Error::MetalFailed)?;

    // setColorSpace: serialize the colorspace into the surface properties.
    if let Some(serialized) = colorspace.property_list() {
        let value: &CFType = serialized.as_ref();
        // SAFETY: the color space key takes a property list value.
        unsafe { surface.set_value(kIOSurfaceColorSpace, value) };
    }

    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_fourcc() {
        assert_eq!(PIXEL_FORMAT_32BGRA, i32::from_be_bytes(*b"BGRA"));
    }
}
