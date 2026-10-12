//! SurfaceMouse represents mouse helper functionality for the core surface
//! (port of `surface_mouse.zig`).
//!
//! It's currently small in scope; its purpose is to isolate mouse logic that
//! has gotten a bit complex (e.g. pointer shape handling for key events).

use ghostty_input::{Key, Mods};
use ghostty_terminal::terminal::MouseEvents;
use ghostty_vt::mouse_shape::MouseShape;

/// See the module docs.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceMouse {
    /// For processing key events; the key that was physically pressed on the
    /// keyboard.
    pub physical_key: Key,

    /// The mouse event tracking mode, if any.
    pub mouse_event: MouseEvents,

    /// The current terminal's mouse shape.
    pub mouse_shape: MouseShape,

    /// The last mods state when the last mouse button (whatever it was) was
    /// pressed or release.
    pub mods: Mods,

    /// True if the mouse position is currently over a link.
    pub over_link: bool,

    /// True if the mouse pointer is currently hidden.
    pub hidden: bool,
}

impl SurfaceMouse {
    /// Translates key state to mouse shape (cursor) state, based on a state
    /// machine.
    ///
    /// There are 4 current states:
    ///
    /// * text: starting state, displays a text bar.
    /// * default: default state when in a mouse tracking mode. (e.g. vim,
    ///   etc). Displays an arrow pointer.
    /// * pointer: default state when over a link. Displays a pointing finger.
    /// * crosshair: any above state can transition to this when the
    ///   rectangle select keys are pressed (ctrl/super+alt).
    ///
    /// Additionally, default can transition back to text if one of the shift
    /// keys are pressed during mouse tracking mode.
    ///
    /// Any secondary state transitions back to its default state when the
    /// appropriate keys are released.
    ///
    /// `None` is returned when the mouse shape does not need changing.
    #[must_use]
    pub fn key_to_mouse_shape(&self) -> Option<MouseShape> {
        // Filter for appropriate key events
        if !eligible_mouse_shape_key_event(self.physical_key) {
            return None;
        }

        // Exceptions: link hover or hidden state overrides any other shape
        // processing and does not change state.
        if self.over_link || self.hidden {
            return None;
        }

        if self.mouse_event == MouseEvents::None {
            // Default terminal mode, should be text (text bar)
            if is_rectangle_select_state(self.mods) {
                // Crosshair (rectangle select)
                Some(MouseShape::Crosshair)
            } else {
                Some(MouseShape::Text)
            }
        } else {
            // In mouse tracking mode, should be default (arrow pointer)
            if is_mouse_mode_override_state(self.mods) && is_rectangle_select_state(self.mods) {
                // Crosshair (rectangle select), only set if we are also
                // overriding (e.g. shift+ctrl+alt)
                Some(MouseShape::Crosshair)
            } else if is_mouse_mode_override_state(self.mods) {
                // Normal override state
                Some(MouseShape::Text)
            } else {
                Some(MouseShape::Default)
            }
        }
    }
}

fn eligible_mouse_shape_key_event(physical_key: Key) -> bool {
    physical_key.ctrl_or_super()
        || physical_key.left_or_right_shift()
        || physical_key.left_or_right_alt()
}

fn is_mouse_mode_override_state(mods: Mods) -> bool {
    mods.shift
}

/// Returns true if our modifiers put us in a state where dragging should
/// cause a rectangle select.
#[must_use]
pub fn is_rectangle_select_state(mods: Mods) -> bool {
    is_rectangle_select_state_for(mods, ghostty_input::platform::is_darwin())
}

/// [`is_rectangle_select_state`] for an explicit platform (Zig picks with
/// `builtin.target.os.tag.isDarwin()` at comptime): on Darwin option alone
/// selects a rectangle, elsewhere it is ctrl+alt.
#[must_use]
pub fn is_rectangle_select_state_for(mods: Mods, darwin: bool) -> bool {
    if darwin {
        mods.alt
    } else {
        mods.ctrl_or_super() && mods.alt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_select_state_per_platform() {
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        let ctrl_alt = Mods {
            alt: true,
            ctrl: true,
            ..Mods::default()
        };
        // macOS: alt alone.
        assert!(is_rectangle_select_state_for(alt, true));
        // Elsewhere: ctrl+alt.
        assert!(!is_rectangle_select_state_for(alt, false));
        assert!(is_rectangle_select_state_for(ctrl_alt, false));
    }

    fn m(
        physical_key: Key,
        mouse_event: MouseEvents,
        mouse_shape: MouseShape,
        mods: Mods,
        over_link: bool,
        hidden: bool,
    ) -> SurfaceMouse {
        SurfaceMouse {
            physical_key,
            mouse_event,
            mouse_shape,
            mods,
            over_link,
            hidden,
        }
    }

    fn mods(shift: bool, ctrl: bool, alt: bool, super_: bool) -> Mods {
        Mods {
            shift,
            ctrl,
            alt,
            super_,
            ..Mods::default()
        }
    }

    #[test]
    fn key_to_mouse_shape() {
        let none = Mods::default();

        // No specific key pressed
        let s = m(
            Key::Unidentified,
            MouseEvents::None,
            MouseShape::Progress,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), None);

        // Over a link.
        let s = m(
            Key::ShiftLeft,
            MouseEvents::None,
            MouseShape::Progress,
            none,
            true,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), None);

        // Mouse is currently hidden
        let s = m(
            Key::ShiftLeft,
            MouseEvents::None,
            MouseShape::Progress,
            none,
            true,
            true,
        );
        assert_eq!(s.key_to_mouse_shape(), None);

        // default, no mods (mouse tracking)
        let s = m(
            Key::ShiftLeft,
            MouseEvents::X10,
            MouseShape::Default,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Default));

        // default -> crosshair (mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::X10,
            MouseShape::Default,
            mods(true, true, true, true),
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Crosshair));

        // default -> text (mouse tracking)
        let s = m(
            Key::ShiftLeft,
            MouseEvents::X10,
            MouseShape::Default,
            mods(true, false, false, false),
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Text));

        // crosshair -> text (mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::X10,
            MouseShape::Crosshair,
            mods(true, false, false, false),
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Text));

        // crosshair -> default (mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::X10,
            MouseShape::Crosshair,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Default));

        // text -> crosshair (mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::X10,
            MouseShape::Text,
            mods(true, true, true, true),
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Crosshair));

        // text -> default (mouse tracking)
        let s = m(
            Key::ShiftLeft,
            MouseEvents::X10,
            MouseShape::Text,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Default));

        // text, no mods (no mouse tracking)
        let s = m(
            Key::ShiftLeft,
            MouseEvents::None,
            MouseShape::Text,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Text));

        // text -> crosshair (no mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::None,
            MouseShape::Text,
            mods(false, true, true, true),
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Crosshair));

        // crosshair -> text (no mouse tracking)
        let s = m(
            Key::AltLeft,
            MouseEvents::None,
            MouseShape::Crosshair,
            none,
            false,
            false,
        );
        assert_eq!(s.key_to_mouse_shape(), Some(MouseShape::Text));
    }
}
