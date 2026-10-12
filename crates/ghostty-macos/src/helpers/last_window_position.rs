//! `Helpers/LastWindowPosition.swift`: manages the persistence and
//! restoration of window positions across app launches. The frame math is in
//! `last_window_position_logic`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};

use super::{
    extensions::user_defaults,
    geometry::Rect,
    last_window_position_logic::{POSITION_KEY, encode, restored_frame},
};

pub struct LastWindowPosition;

impl LastWindowPosition {
    /// `LastWindowPosition.shared`.
    #[must_use]
    pub fn shared() -> Self {
        Self
    }

    /// Saves the window frame. Returns false if it wasn't saved.
    pub fn save(&self, window: Option<&NSWindow>) -> bool {
        // We should only save the frame if the window is visible.
        // This avoids overriding the previously saved one
        // with the wrong one when window decorations change while creating,
        // e.g. adding a toolbar affects the window's frame.
        let Some(window) = window else { return false };
        if !window.isVisible() {
            return false;
        }
        let f = window.frame();
        let rect = encode(&Rect::new(
            f.origin.x,
            f.origin.y,
            f.size.width,
            f.size.height,
        ));
        let numbers: Vec<Retained<NSNumber>> = rect.iter().map(|v| NSNumber::new_f64(*v)).collect();
        let array = NSArray::from_retained_slice(&numbers);
        // SAFETY: an NSArray of NSNumbers is a valid property-list value.
        #[allow(unsafe_code)]
        unsafe {
            user_defaults::ghostty()
                .setObject_forKey(Some(&array), &NSString::from_str(POSITION_KEY));
        }
        true
    }

    /// Restores a previously saved window frame (or parts of it) onto the
    /// given window.
    ///
    /// - `restore_origin`: whether to restore the saved position. Pass `false`
    ///   when the config specifies an explicit `window-position-x`/`window-position-y`.
    /// - `restore_size`: whether to restore the saved size. Pass `false` when
    ///   the config specifies an explicit `window-width`/`window-height`.
    ///
    /// Returns `true` if the frame was modified, `false` if there was nothing
    /// to restore.
    pub fn restore(&self, window: &NSWindow, restore_origin: bool, restore_size: bool) -> bool {
        if !(restore_origin || restore_size) {
            return false;
        }

        let defaults = user_defaults::ghostty();
        let Some(array) = defaults.arrayForKey(&NSString::from_str(POSITION_KEY)) else {
            return false;
        };
        // `as? [Double]`: every element must be a number.
        let mut values = Vec::new();
        for o in &array {
            let Some(n) = o.downcast_ref::<NSNumber>() else {
                return false;
            };
            values.push(n.doubleValue());
        }
        if values.len() < 2 {
            return false;
        }

        let screen: Option<Retained<NSScreen>> = window
            .screen()
            .or_else(|| NSScreen::mainScreen(super::main_thread()));
        let Some(screen) = screen else { return false };
        let to_rect = |r: NSRect| Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height);

        let Some(new_frame) = restored_frame(
            &values,
            to_rect(window.frame()),
            to_rect(screen.visibleFrame()),
            restore_origin,
            restore_size,
        ) else {
            return false;
        };

        window.setFrame_display(
            NSRect::new(
                NSPoint::new(new_frame.origin.x, new_frame.origin.y),
                NSSize::new(new_frame.size.width, new_frame.size.height),
            ),
            true,
        );
        true
    }
}
