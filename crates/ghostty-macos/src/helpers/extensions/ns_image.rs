//! `Helpers/Extensions/NSImage+Extension.swift`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, deprecated)]

use objc2::{AnyThread, rc::Retained};
use objc2_app_kit::{NSColor, NSCompositingOperation, NSGradient, NSGraphicsContext, NSImage};
use objc2_core_graphics::{
    CGBitmapContextCreate,
    CGBitmapContextCreateImage,
    CGBlendMode,
    CGColorSpace,
    CGContext,
    CGImageAlphaInfo,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};

fn zero_rect(size: NSSize) -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), size)
}

/// `NSImage.cgImage(forProposedRect: nil, context: nil, hints: nil)`.
#[allow(unsafe_code)]
fn cg_image(image: &NSImage) -> Option<Retained<objc2_core_graphics::CGImage>> {
    // SAFETY: nil rect/context/hints are allowed.
    unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }
}

pub trait NSImageExt {
    /// Apply a gradient onto this image, using this image as a mask.
    fn gradient(&self, colors: &[Retained<NSColor>]) -> Option<Retained<NSImage>>;
    /// Tint an NSImage with the given color by applying a basic fill on top of it.
    fn tint(&self, color: &NSColor) -> Option<Retained<NSImage>>;
}

/// `NSImage.combine(images:blendingModes:)`: combine multiple images with the
/// given blend modes. This is useful given a set of layers to create a final
/// rasterized image.
#[allow(unsafe_code)]
#[must_use]
pub fn combine(
    images: &[Retained<NSImage>],
    blending_modes: &[CGBlendMode],
) -> Option<Retained<NSImage>> {
    if images.len() != blending_modes.len() || images.is_empty() {
        return None;
    }

    // The final size will be the same size as our first image.
    let size = images[0].size();

    // Create a bitmap context manually
    let space = CGColorSpace::new_device_rgb();
    // SAFETY: null data lets CoreGraphics allocate the backing store.
    let ctx = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            size.width as usize,
            size.height as usize,
            8,
            0,
            space.as_deref(),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;

    // Clear the context
    CGContext::set_fill_color_with_color(Some(&ctx), None);
    CGContext::clear_rect(Some(&ctx), zero_rect(size));

    // Draw each image with its corresponding blend mode
    for (image, mode) in images.iter().zip(blending_modes) {
        let cg = cg_image(image)?;
        CGContext::set_blend_mode(Some(&ctx), *mode);
        CGContext::draw_image(Some(&ctx), zero_rect(size), Some(&cg));
    }

    // Create a CGImage from the context
    let combined = CGBitmapContextCreateImage(Some(&ctx))?;

    // Wrap the CGImage in an NSImage
    Some(NSImage::initWithCGImage_size(
        NSImage::alloc(),
        &combined,
        size,
    ))
}

impl NSImageExt for NSImage {
    fn gradient(&self, colors: &[Retained<NSColor>]) -> Option<Retained<NSImage>> {
        let size = self.size();
        let result = NSImage::initWithSize(NSImage::alloc(), size);
        result.lockFocus();
        let out = (|| {
            // Draw the gradient
            let array = NSArray::from_retained_slice(colors);
            let gradient = NSGradient::initWithColors(NSGradient::alloc(), &array)?;
            gradient.drawInRect_angle(zero_rect(size), 90.0);

            // Apply the mask
            self.drawAtPoint_fromRect_operation_fraction(
                NSPoint::new(0.0, 0.0),
                NSRect::ZERO,
                NSCompositingOperation::DestinationIn,
                1.0,
            );
            Some(())
        })();
        result.unlockFocus();
        out.map(|()| result)
    }

    fn tint(&self, color: &NSColor) -> Option<Retained<NSImage>> {
        let size = self.size();

        // Create a new image with the same size as the base image
        let new_image = NSImage::initWithSize(NSImage::alloc(), size);

        // Draw into the new image
        new_image.lockFocus();
        let out = (|| {
            // Set up the drawing context
            let context = NSGraphicsContext::currentContext()?.CGContext();

            // Draw the base image
            let cg = cg_image(self)?;
            CGContext::draw_image(Some(&context), zero_rect(size), Some(&cg));

            // Set the tint color and blend mode
            CGContext::set_fill_color_with_color(Some(&context), Some(&color.CGColor()));
            CGContext::set_blend_mode(Some(&context), CGBlendMode::SourceAtop);

            // Apply the tint color over the entire image
            CGContext::fill_rect(Some(&context), zero_rect(size));
            Some(context)
        })();
        if let Some(context) = &out {
            CGContext::restore_g_state(Some(context));
        }
        new_image.unlockFocus();
        out.map(|_| new_image)
    }
}
