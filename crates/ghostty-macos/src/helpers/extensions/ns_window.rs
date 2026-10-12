//! `Helpers/Extensions/NSWindow+Extension.swift`. Frame math is in
//! `ns_window_logic`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, clippy::borrow_as_ptr, clippy::ref_as_ptr)]

use objc2::{msg_send, rc::Retained, runtime::AnyObject, sel};
use objc2_app_kit::{NSScreen, NSView, NSWindow, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{ns_view::NSViewExt, ns_window_logic};
use crate::helpers::{geometry::Rect, objc_exception_catcher};

fn to_rect(r: NSRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

fn to_ns_rect(r: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(r.origin.x, r.origin.y),
        NSSize::new(r.size.width, r.size.height),
    )
}

pub trait NSWindowExt {
    /// Get the CGWindowID type for the window (used for low level
    /// CoreGraphics APIs). `None` if the window has no window device.
    fn cg_window_id(&self) -> Option<u32>;
    /// Adjusts the window frame if necessary to ensure the window remains
    /// visible on screen. This constrains both the size (to not exceed the
    /// screen) and the origin (to keep the window on screen).
    fn constrain_to_screen(&self);
    /// True if this is the first window in the tab group.
    fn is_first_window_in_tab_group(&self) -> bool;
    /// Wraps `addTabbedWindow` with an Objective-C exception catcher because
    /// AppKit can throw NSExceptions in visual tab picker flows.
    fn add_tabbed_window_safely(&self, child: &NSWindow, ordered: NSWindowOrderingMode) -> bool;

    // Native tabbing private API usage. :(

    /// The [private] titlebar view. In normal windows `NSTabBar` typically
    /// appears as a subview of `NSTitlebarView` within `NSThemeFrame`. In
    /// fullscreen, the system creates a dedicated fullscreen window and the
    /// view hierarchy changes; in that case, the `titlebarView` is only
    /// accessible via a reference on `NSThemeFrame`.
    fn titlebar_view(&self) -> Option<Retained<NSView>>;
    /// Returns the [private] NSTabBar view, if it exists.
    fn tab_bar_view(&self) -> Option<Retained<NSView>>;
    /// Returns tab button views in visual order from left to right.
    fn tab_buttons_in_visual_order(&self) -> Vec<Retained<NSView>>;
    /// Returns the visual tab index and matching tab button at the given screen point.
    fn tab_button_hit(&self, screen_point: NSPoint) -> Option<(usize, Retained<NSView>)>;
    /// Returns the index of the tab button at the given screen point, if any.
    fn tab_index(&self, screen_point: NSPoint) -> Option<usize>;
}

impl NSWindowExt for NSWindow {
    fn cg_window_id(&self) -> Option<u32> {
        ns_window_logic::cg_window_id(self.windowNumber())
    }

    fn constrain_to_screen(&self) {
        let screen: Option<Retained<NSScreen>> = self
            .screen()
            .or_else(|| NSScreen::mainScreen(crate::helpers::main_thread()));
        let Some(screen) = screen else { return };
        let window_frame = to_rect(self.frame());
        if let Some(new_frame) =
            ns_window_logic::constrained_frame(window_frame, to_rect(screen.visibleFrame()))
        {
            self.setFrame_display(to_ns_rect(new_frame), true);
        }
    }

    fn is_first_window_in_tab_group(&self) -> bool {
        let Some(group) = self.tabGroup() else {
            return true;
        };
        let Some(first) = group.windows().firstObject() else {
            return true;
        };
        std::ptr::eq(&*first, self)
    }

    fn add_tabbed_window_safely(&self, child: &NSWindow, ordered: NSWindowOrderingMode) -> bool {
        match objc_exception_catcher::add_tabbed_window_safely(self, child, ordered) {
            Ok(()) => true,
            Err(error) => {
                log::error!("addTabbedWindow failed: {error}");
                false
            }
        }
    }

    fn titlebar_view(&self) -> Option<Retained<NSView>> {
        let theme_frame_view = self.contentView()?.root_view();
        // SAFETY: `respondsToSelector:` is safe to send to any object.
        let responds: bool =
            unsafe { msg_send![&*theme_frame_view, respondsToSelector: sel!(titlebarView)] };
        if !responds {
            return None;
        }
        // `value(forKey: "titlebarView") as? NSView`
        let value: Option<Retained<AnyObject>> = unsafe {
            msg_send![&*theme_frame_view, valueForKey: &*NSString::from_str("titlebarView")]
        };
        value?.downcast::<NSView>().ok()
    }

    fn tab_bar_view(&self) -> Option<Retained<NSView>> {
        self.titlebar_view()?
            .first_descendant_with_class_name("NSTabBar")
    }

    fn tab_buttons_in_visual_order(&self) -> Vec<Retained<NSView>> {
        let Some(tab_bar_view) = self.tab_bar_view() else {
            return Vec::new();
        };
        let mut buttons = tab_bar_view.descendants_with_class_name("NSTabButton");
        buttons.sort_by(|a, b| a.frame().origin.x.total_cmp(&b.frame().origin.x));
        buttons
    }

    fn tab_button_hit(&self, screen_point: NSPoint) -> Option<(usize, Retained<NSView>)> {
        let tab_bar_view = self.tab_bar_view()?;
        let tab_bar_window = tab_bar_view.window()?;

        // In fullscreen, AppKit can host the titlebar and tab bar in a
        // separate NSToolbarFullScreenWindow. Hit testing has to use that
        // window's base coordinate space or content clicks can be
        // misinterpreted as tab clicks.
        let location_in_tab_bar_window = tab_bar_window.convertPointFromScreen(screen_point);
        let location_in_tab_bar =
            tab_bar_view.convertPoint_fromView(location_in_tab_bar_window, None);
        if !point_in_rect(location_in_tab_bar, tab_bar_view.bounds()) {
            return None;
        }

        for (index, tab_button) in self.tab_buttons_in_visual_order().into_iter().enumerate() {
            let location_in_tab_button =
                tab_button.convertPoint_fromView(location_in_tab_bar_window, None);
            if point_in_rect(location_in_tab_button, tab_button.bounds()) {
                return Some((index, tab_button));
            }
        }

        None
    }

    fn tab_index(&self, screen_point: NSPoint) -> Option<usize> {
        self.tab_button_hit(screen_point).map(|(i, _)| i)
    }
}

/// `CGRect.contains(CGPoint)`.
fn point_in_rect(p: NSPoint, r: NSRect) -> bool {
    p.x >= r.origin.x
        && p.x < r.origin.x + r.size.width
        && p.y >= r.origin.y
        && p.y < r.origin.y + r.size.height
}
