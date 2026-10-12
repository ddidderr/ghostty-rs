//! `Ghostty.Surface` (`Ghostty.Surface.swift`): the data model wrapping a
//! `ghostty_surface_t`.
//!
//! Main-thread only (`@MainActor` in Swift). The typed `Input.KeyEvent` /
//! `Input.Mouse*Event` overloads of the Swift class belong to the
//! `Ghostty.Input` port (MA2); this wrapper exposes the C-level forms they
//! lower to.
#![allow(unsafe_code)]

use std::ffi::c_void;

use dispatch2::DispatchQueue;

use crate::ghostty::capi::{
    ghostty_binding_flags_e,
    ghostty_input_key_s,
    ghostty_surface_binding_action,
    ghostty_surface_free,
    ghostty_surface_key,
    ghostty_surface_key_is_binding,
    ghostty_surface_mouse_button,
    ghostty_surface_mouse_captured,
    ghostty_surface_mouse_pos,
    ghostty_surface_mouse_scroll,
    ghostty_surface_t,
    ghostty_surface_text,
};

/// Wraps a `ghostty_surface_t`; frees it on drop (on the main queue).
pub struct Surface {
    surface: ghostty_surface_t,
}

impl Surface {
    /// `init(cSurface:)`. Takes ownership of the surface.
    #[must_use]
    pub fn new(c_surface: ghostty_surface_t) -> Self {
        Self { surface: c_surface }
    }

    /// `unsafeCValue`: the underlying C value. Unsafe because the value is
    /// freed when this `Surface` is dropped.
    #[must_use]
    pub fn unsafe_c_value(&self) -> ghostty_surface_t {
        self.surface
    }

    /// Send text to the terminal as if it was typed. This doesn't send the key
    /// events so keyboard shortcuts and other encodings do not take effect.
    pub fn send_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        // SAFETY: valid surface; the pointer/length describe `text`.
        unsafe { ghostty_surface_text(self.surface, text.as_ptr().cast(), text.len()) };
    }

    /// Send a key event to the terminal (`sendKeyEvent`, C form).
    /// TODO(MA2): the typed `Input.KeyEvent` overload (`withCValue`).
    pub fn send_key_event(&self, event: ghostty_input_key_s) -> bool {
        // SAFETY: valid surface; `event.text` is valid for the call (caller).
        unsafe { ghostty_surface_key(self.surface, event) }
    }

    /// Check if a key event matches a keybinding; returns the binding flags
    /// (`Input.BindingFlags.rawValue`) if it does (`keyIsBinding`, C form).
    /// TODO(MA2): the typed `Input.KeyEvent` overload.
    #[must_use]
    pub fn key_is_binding(&self, event: ghostty_input_key_s) -> Option<u32> {
        let mut flags: ghostty_binding_flags_e = 0;
        // SAFETY: valid surface and out pointer.
        if unsafe { ghostty_surface_key_is_binding(self.surface, event, &raw mut flags) } {
            Some(flags as u32)
        } else {
            None
        }
    }

    /// Whether the terminal has captured mouse input (mouse reporting).
    #[must_use]
    pub fn mouse_captured(&self) -> bool {
        // SAFETY: valid surface.
        unsafe { ghostty_surface_mouse_captured(self.surface) }
    }

    /// `sendMouseButton` with the already lowered C values
    /// (`event.action.cMouseState`, `event.button.cMouseButton`, `event.mods.cMods`).
    /// TODO(MA2): typed `Input.MouseButtonEvent` overload.
    pub fn send_mouse_button(&self, state: i32, button: i32, mods: i32) {
        // SAFETY: valid surface.
        unsafe { ghostty_surface_mouse_button(self.surface, state, button, mods) };
    }

    /// `sendMousePos` (`event.x`, `event.y`, `event.mods.cMods`).
    /// TODO(MA2): typed `Input.MousePosEvent` overload.
    pub fn send_mouse_pos(&self, x: f64, y: f64, mods: i32) {
        // SAFETY: valid surface.
        unsafe { ghostty_surface_mouse_pos(self.surface, x, y, mods) };
    }

    /// `sendMouseScroll` (`event.x`, `event.y`, `event.mods.cScrollMods`).
    /// TODO(MA2): typed `Input.MouseScrollEvent` overload.
    pub fn send_mouse_scroll(&self, x: f64, y: f64, scroll_mods: i32) {
        // SAFETY: valid surface.
        unsafe { ghostty_surface_mouse_scroll(self.surface, x, y, scroll_mods) };
    }

    /// Perform a keybinding action, e.g. `goto_tab:4`. Returns true if the
    /// action was performed; invalid actions return false.
    #[must_use]
    pub fn perform(&self, action: &str) -> bool {
        if action.is_empty() {
            return false;
        }
        // SAFETY: valid surface; pointer/length describe `action`.
        unsafe {
            ghostty_surface_binding_action(self.surface, action.as_ptr().cast(), action.len())
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // deinit is not guaranteed to happen on the main thread and our API
        // calls into libghostty must happen there, so the free is scheduled
        // on the main queue (Swift: `Task.detached { @MainActor in ... }`).
        let surface = self.surface as usize;
        DispatchQueue::main().exec_async(move || {
            // SAFETY: the surface was owned by this wrapper, which is gone.
            unsafe { ghostty_surface_free(surface as *mut c_void) };
        });
    }
}
