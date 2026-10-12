//! `Helpers/Extensions/NSMenuItem+Extension.swift`.

use objc2::rc::Retained;
use objc2_app_kit::{NSImage, NSMenuItem};
use objc2_foundation::NSString;

use super::macos_at_least;

pub trait NSMenuItemExt {
    /// Sets the image property from a symbol if we want images on our menu items.
    fn set_image_if_desired(&self, system_symbol_name: &str);
}

impl NSMenuItemExt for NSMenuItem {
    fn set_image_if_desired(&self, system_symbol_name: &str) {
        // We only set on macOS 26 when icons on menu items became the norm.
        if macos_at_least(26) {
            let title = self.title();
            let image: Option<Retained<NSImage>> =
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(system_symbol_name),
                    Some(&title),
                );
            self.setImage(image.as_deref());
        }
    }
}
