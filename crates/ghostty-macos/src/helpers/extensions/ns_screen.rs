//! `Helpers/Extensions/NSScreen+Extension.swift`. The math is in
//! `ns_screen_logic`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSScreen};
use objc2_core_foundation::{CFRetained, CFUUID};
use objc2_foundation::{NSNumber, NSPoint, NSSize, NSString, NSUUID};

use super::{ns_screen_logic, user_defaults, uuid::uuid_from_cfuuid};
use crate::helpers::geometry::{Rect, Size};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Returns a +1 retained CFUUID or null.
    fn CGDisplayCreateUUIDFromDisplayID(display_id: u32) -> *const CFUUID;
}

fn to_rect(r: objc2_foundation::NSRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

pub trait NSScreenExt {
    /// The unique CoreGraphics display ID for this screen.
    fn display_id(&self) -> Option<u32>;
    /// The stable UUID for this display, suitable for tracking across
    /// reconnects and NSScreen garbage collection.
    fn display_uuid(&self) -> Option<Retained<NSUUID>>;
    /// Returns true if the given screen has a visible dock. This isn't
    /// point-in-time visible, this is true if the dock is always visible AND
    /// present on this screen.
    fn has_dock(&self) -> bool;
    /// Returns true if the screen has a visible notch (i.e., a non-zero safe
    /// area inset at the top).
    fn has_notch(&self) -> bool;
    /// Converts top-left offset coordinates to bottom-left origin coordinates
    /// for window positioning.
    fn origin_from_top_left_offset(&self, x: f64, y: f64, window_size: NSSize) -> NSPoint;
}

impl NSScreenExt for NSScreen {
    fn display_id(&self) -> Option<u32> {
        let description = self.deviceDescription();
        let key = NSString::from_str("NSScreenNumber");
        let value = description.objectForKey(&key)?;
        value
            .downcast_ref::<NSNumber>()
            .map(NSNumber::unsignedIntValue)
    }

    #[allow(unsafe_code)]
    fn display_uuid(&self) -> Option<Retained<NSUUID>> {
        let display_id = self.display_id()?;
        // SAFETY: plain integer argument; result is +1 retained or null.
        let raw = unsafe { CGDisplayCreateUUIDFromDisplayID(display_id) };
        let raw = std::ptr::NonNull::new(raw.cast_mut())?;
        // SAFETY: adopting the +1 reference.
        let cfuuid: CFRetained<CFUUID> = unsafe { CFRetained::from_raw(raw) };
        uuid_from_cfuuid(&cfuuid)
    }

    fn has_dock(&self) -> bool {
        // If the dock autohides then we don't have a dock ever.
        let domain =
            user_defaults::ghostty().persistentDomainForName(&NSString::from_str("com.apple.dock"));
        let autohide = domain
            .and_then(|d| d.objectForKey(&NSString::from_str("autohide")))
            .and_then(|v| v.downcast_ref::<NSNumber>().map(NSNumber::boolValue));
        if autohide == Some(true) {
            return false;
        }

        // There is no public API to directly ask about dock visibility, so we
        // compare visibleFrame and frame (see ns_screen_logic).
        let mtm = crate::helpers::main_thread();
        let menu_height = NSApplication::sharedApplication(mtm)
            .mainMenu()
            .map_or(0.0, |m| m.menuBarHeight());
        ns_screen_logic::has_visible_dock(
            to_rect(self.visibleFrame()),
            to_rect(self.frame()),
            menu_height,
            self.safeAreaInsets().top,
        )
    }

    fn has_notch(&self) -> bool {
        // We assume that a top safe area means notch, since we don't
        // currently know any other situation this is true.
        ns_screen_logic::has_notch(self.safeAreaInsets().top)
    }

    fn origin_from_top_left_offset(&self, x: f64, y: f64, window_size: NSSize) -> NSPoint {
        let p = ns_screen_logic::origin_from_top_left_offset(
            to_rect(self.visibleFrame()),
            x,
            y,
            Size {
                width: window_size.width,
                height: window_size.height,
            },
        );
        NSPoint::new(p.x, p.y)
    }
}
