//! `Helpers/VibrantLayer.{h,m}`.
//!
//! This layer can be used to recreate the "vibrant" appearance you see of
//! views placed inside `NSVisualEffectView`s. When a light NSAppearance is
//! active, we will use the private "plus darker" blend mode. For dark
//! appearances we use "plus lighter".

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::cell::Cell;

use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::NSString;
use objc2_quartz_core::CALayer;

/// `VibrantLayerType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VibrantLayerType {
    Light,
    Dark,
}

/// The `compositingFilter` name for a layer type (`@"plusD"` / `@"plusL"`).
#[must_use]
pub fn compositing_filter_name(ty: VibrantLayerType) -> &'static str {
    match ty {
        VibrantLayerType::Light => "plusD",
        VibrantLayerType::Dark => "plusL",
    }
}

define_class!(
    // SAFETY: CALayer has no subclassing requirements we violate; no Drop.
    #[unsafe(super(CALayer))]
    #[name = "VibrantLayer"]
    #[ivars = Cell<VibrantLayerType>]
    pub struct VibrantLayer;

    impl VibrantLayer {
        // SAFETY: matches `- (id)compositingFilter`.
        #[unsafe(method_id(compositingFilter))]
        fn compositing_filter(&self) -> Option<Retained<AnyObject>> {
            let name = NSString::from_str(compositing_filter_name(self.ivars().get()));
            // SAFETY: NSString is an object; upcasting to AnyObject is valid.
            Some(unsafe { Retained::cast_unchecked::<AnyObject>(name) })
        }
    }
);

impl VibrantLayer {
    /// `init(forAppearance:)`.
    #[must_use]
    pub fn init_for_appearance(ty: VibrantLayerType) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Cell::new(ty));
        // SAFETY: `init` is the designated initializer of CALayer.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_names() {
        assert_eq!(compositing_filter_name(VibrantLayerType::Light), "plusD");
        assert_eq!(compositing_filter_name(VibrantLayerType::Dark), "plusL");
    }
}
