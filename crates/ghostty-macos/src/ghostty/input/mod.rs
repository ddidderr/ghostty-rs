//! The top of `Ghostty.Input.swift`: keyboard shortcuts from triggers, the
//! `NSEvent.ModifierFlags` <-> `ghostty_input_mods_e` conversions, the
//! `keyToEquivalent` map and `Ghostty.Input.BindingFlags`.
//!
//! TODO(MA2): the rest of `Ghostty.Input.swift` (`KeyEvent`, `MouseButtonEvent`,
//! `MousePosEvent`, `MouseScrollEvent`, `MouseState`, `MouseButton`,
//! `ScrollMods`, `Momentum`, `Mods`, the `Key` enum and the `AppEnum`
//! conformances) belongs to the Surface View port and is added to this module
//! by that agent.
//!
//! Modifier flags are handled as the raw `NSEvent.ModifierFlags.rawValue`
//! (`u64`, `NSUInteger`) so this code is portable and unit tested on every
//! target; the AppKit wrappers pass `flags.bits()`.
#![allow(unsafe_code, clippy::wildcard_imports)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use bitflags::bitflags;

use super::capi::{consts::*, ghostty_input_key_e, ghostty_input_mods_e, ghostty_input_trigger_s};
use crate::helpers::keyboard_shortcut::{EventModifiers, KeyEquivalent, KeyboardShortcut};

// MARK: NSEvent.ModifierFlags raw values

/// `NSEvent.ModifierFlags.capsLock`
pub const NS_FLAG_CAPS_LOCK: u64 = 1 << 16;
/// `NSEvent.ModifierFlags.shift`
pub const NS_FLAG_SHIFT: u64 = 1 << 17;
/// `NSEvent.ModifierFlags.control`
pub const NS_FLAG_CONTROL: u64 = 1 << 18;
/// `NSEvent.ModifierFlags.option`
pub const NS_FLAG_OPTION: u64 = 1 << 19;
/// `NSEvent.ModifierFlags.command`
pub const NS_FLAG_COMMAND: u64 = 1 << 20;

/// `NX_DEVICERSHIFTKEYMASK`
pub const NX_DEVICERSHIFTKEYMASK: u64 = 0x0000_0004;
/// `NX_DEVICERCTLKEYMASK`
pub const NX_DEVICERCTLKEYMASK: u64 = 0x0000_2000;
/// `NX_DEVICERALTKEYMASK`
pub const NX_DEVICERALTKEYMASK: u64 = 0x0000_0040;
/// `NX_DEVICERCMDKEYMASK`
pub const NX_DEVICERCMDKEYMASK: u64 = 0x0000_0010;

// MARK: Keyboard Shortcuts

/// Return the key equivalent for the given trigger.
///
/// Returns nil if the trigger doesn't have an equivalent KeyboardShortcut. This is possible
/// because Ghostty input triggers are a superset of what can be represented by a macOS
/// KeyboardShortcut. For example, macOS doesn't have any way to represent function keys
/// (F1, F2, ...) with a KeyboardShortcut. This doesn't represent a practical issue because input
/// handling for Ghostty is handled at a lower level (usually). This function should generally only
/// be used for things like NSMenu that only support keyboard shortcuts anyways.
#[must_use]
pub fn keyboard_shortcut(trigger: &ghostty_input_trigger_s) -> Option<KeyboardShortcut> {
    let key = match trigger.tag {
        GHOSTTY_TRIGGER_PHYSICAL => {
            // Only functional keys can be converted to a KeyboardShortcut. Other physical
            // mappings cannot because KeyboardShortcut in Swift is inherently layout-dependent.
            // SAFETY: the tag selects the `physical` member (all members are 32-bit).
            key_to_equivalent(unsafe { trigger.key.physical })?
        }

        GHOSTTY_TRIGGER_UNICODE => {
            // SAFETY: the tag selects the `unicode` member.
            let scalar = char::from_u32(unsafe { trigger.key.unicode })?;
            KeyEquivalent::new(scalar)
        }

        // catch_all matches any key, so it can't be represented as a KeyboardShortcut
        _ => return None,
    };

    Some(KeyboardShortcut::new(
        key,
        event_modifiers_from_ns_flags(event_modifier_flags(trigger.mods)),
    ))
}

// MARK: Mods

/// Returns the event modifier flags (raw `NSEvent.ModifierFlags`) set for the Ghostty mods enum.
#[must_use]
pub fn event_modifier_flags(mods: ghostty_input_mods_e) -> u64 {
    let mut flags = 0;
    if mods & GHOSTTY_MODS_SHIFT != 0 {
        flags |= NS_FLAG_SHIFT;
    }
    if mods & GHOSTTY_MODS_CTRL != 0 {
        flags |= NS_FLAG_CONTROL;
    }
    if mods & GHOSTTY_MODS_ALT != 0 {
        flags |= NS_FLAG_OPTION;
    }
    if mods & GHOSTTY_MODS_SUPER != 0 {
        flags |= NS_FLAG_COMMAND;
    }
    flags
}

/// Translate event modifier flags (raw `NSEvent.ModifierFlags`) to a ghostty mods enum.
#[must_use]
pub fn ghostty_mods(flags: u64) -> ghostty_input_mods_e {
    let mut mods = GHOSTTY_MODS_NONE;

    if flags & NS_FLAG_SHIFT != 0 {
        mods |= GHOSTTY_MODS_SHIFT;
    }
    if flags & NS_FLAG_CONTROL != 0 {
        mods |= GHOSTTY_MODS_CTRL;
    }
    if flags & NS_FLAG_OPTION != 0 {
        mods |= GHOSTTY_MODS_ALT;
    }
    if flags & NS_FLAG_COMMAND != 0 {
        mods |= GHOSTTY_MODS_SUPER;
    }
    if flags & NS_FLAG_CAPS_LOCK != 0 {
        mods |= GHOSTTY_MODS_CAPS;
    }

    // Handle sided input. We can't tell that both are pressed in the
    // Ghostty structure but that's okay -- we don't use that information.
    if flags & NX_DEVICERSHIFTKEYMASK != 0 {
        mods |= GHOSTTY_MODS_SHIFT_RIGHT;
    }
    if flags & NX_DEVICERCTLKEYMASK != 0 {
        mods |= GHOSTTY_MODS_CTRL_RIGHT;
    }
    if flags & NX_DEVICERALTKEYMASK != 0 {
        mods |= GHOSTTY_MODS_ALT_RIGHT;
    }
    if flags & NX_DEVICERCMDKEYMASK != 0 {
        mods |= GHOSTTY_MODS_SUPER_RIGHT;
    }

    mods
}

/// `EventModifiers(nsFlags:)` (`EventModifiers+Extension.swift`) on raw flags.
#[must_use]
pub fn event_modifiers_from_ns_flags(flags: u64) -> EventModifiers {
    let mut result = EventModifiers::empty();
    if flags & NS_FLAG_SHIFT != 0 {
        result.insert(EventModifiers::SHIFT);
    }
    if flags & NS_FLAG_CONTROL != 0 {
        result.insert(EventModifiers::CONTROL);
    }
    if flags & NS_FLAG_OPTION != 0 {
        result.insert(EventModifiers::OPTION);
    }
    if flags & NS_FLAG_COMMAND != 0 {
        result.insert(EventModifiers::COMMAND);
    }
    if flags & NS_FLAG_CAPS_LOCK != 0 {
        result.insert(EventModifiers::CAPS_LOCK);
    }
    result
}

/// `NSEvent.ModifierFlags(swiftUIFlags:)` (`EventModifiers+Extension.swift`), as raw flags.
#[must_use]
pub fn ns_flags_from_event_modifiers(flags: EventModifiers) -> u64 {
    let mut result = 0;
    if flags.contains(EventModifiers::SHIFT) {
        result |= NS_FLAG_SHIFT;
    }
    if flags.contains(EventModifiers::CONTROL) {
        result |= NS_FLAG_CONTROL;
    }
    if flags.contains(EventModifiers::OPTION) {
        result |= NS_FLAG_OPTION;
    }
    if flags.contains(EventModifiers::COMMAND) {
        result |= NS_FLAG_COMMAND;
    }
    if flags.contains(EventModifiers::CAPS_LOCK) {
        result |= NS_FLAG_CAPS_LOCK;
    }
    result
}

/// `Ghostty.keyToEquivalent`: a map from the Ghostty key enum to the keyEquivalent string for
/// shortcuts. Note that not all ghostty key enum values are represented here because not all of
/// them can be mapped to a KeyEquivalent.
#[must_use]
pub fn key_to_equivalent(key: ghostty_input_key_e) -> Option<KeyEquivalent> {
    Some(match key {
        // Function keys
        GHOSTTY_KEY_ARROW_UP => KeyEquivalent::UP_ARROW,
        GHOSTTY_KEY_ARROW_DOWN => KeyEquivalent::DOWN_ARROW,
        GHOSTTY_KEY_ARROW_LEFT => KeyEquivalent::LEFT_ARROW,
        GHOSTTY_KEY_ARROW_RIGHT => KeyEquivalent::RIGHT_ARROW,
        GHOSTTY_KEY_HOME => KeyEquivalent::HOME,
        GHOSTTY_KEY_END => KeyEquivalent::END,
        GHOSTTY_KEY_DELETE => KeyEquivalent::DELETE,
        GHOSTTY_KEY_PAGE_UP => KeyEquivalent::PAGE_UP,
        GHOSTTY_KEY_PAGE_DOWN => KeyEquivalent::PAGE_DOWN,
        GHOSTTY_KEY_ESCAPE => KeyEquivalent::ESCAPE,
        GHOSTTY_KEY_ENTER => KeyEquivalent::RETURN,
        GHOSTTY_KEY_TAB => KeyEquivalent::TAB,
        GHOSTTY_KEY_BACKSPACE => KeyEquivalent::DELETE,
        GHOSTTY_KEY_SPACE => KeyEquivalent::SPACE,
        _ => return None,
    })
}

// MARK: Ghostty.Input.BindingFlags

bitflags! {
    /// `ghostty_binding_flags_e`
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct BindingFlags: u32 {
        const CONSUMED = GHOSTTY_BINDING_FLAGS_CONSUMED as u32;
        const ALL = GHOSTTY_BINDING_FLAGS_ALL as u32;
        const GLOBAL = GHOSTTY_BINDING_FLAGS_GLOBAL as u32;
        const PERFORMABLE = GHOSTTY_BINDING_FLAGS_PERFORMABLE as u32;
    }
}

impl BindingFlags {
    /// `init(cFlags:)`.
    #[must_use]
    pub const fn from_c_flags(c: c_int) -> Self {
        Self::from_bits_retain(c as u32)
    }

    /// `cFlags`.
    #[must_use]
    pub const fn c_flags(self) -> c_int {
        self.bits() as c_int
    }
}

use std::ffi::c_int;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::capi::ghostty_input_trigger_key_u;

    fn unicode(c: char, mods: c_int) -> ghostty_input_trigger_s {
        ghostty_input_trigger_s {
            tag: GHOSTTY_TRIGGER_UNICODE,
            key: ghostty_input_trigger_key_u { unicode: c as u32 },
            mods,
        }
    }

    #[test]
    fn unicode_trigger_becomes_shortcut() {
        let s = keyboard_shortcut(&unicode('q', GHOSTTY_MODS_SUPER)).unwrap();
        assert_eq!(
            s,
            KeyboardShortcut::new(KeyEquivalent::new('q'), EventModifiers::COMMAND)
        );
        let s = keyboard_shortcut(&unicode(',', GHOSTTY_MODS_SUPER | GHOSTTY_MODS_SHIFT)).unwrap();
        assert_eq!(s.modifiers, EventModifiers::COMMAND | EventModifiers::SHIFT);
    }

    #[test]
    fn physical_and_catch_all_triggers() {
        let t = ghostty_input_trigger_s {
            tag: GHOSTTY_TRIGGER_PHYSICAL,
            key: ghostty_input_trigger_key_u {
                physical: GHOSTTY_KEY_ARROW_LEFT,
            },
            mods: GHOSTTY_MODS_ALT,
        };
        let s = keyboard_shortcut(&t).unwrap();
        assert_eq!(s.key, KeyEquivalent::LEFT_ARROW);
        assert_eq!(s.modifiers, EventModifiers::OPTION);

        // Not a functional key.
        let t = ghostty_input_trigger_s {
            key: ghostty_input_trigger_key_u {
                physical: GHOSTTY_KEY_A,
            },
            ..t
        };
        assert!(keyboard_shortcut(&t).is_none());
        let t = ghostty_input_trigger_s {
            tag: GHOSTTY_TRIGGER_CATCH_ALL,
            ..t
        };
        assert!(keyboard_shortcut(&t).is_none());
    }

    #[test]
    fn invalid_unicode_scalar_is_none() {
        let t = unicode('a', 0);
        let t = ghostty_input_trigger_s {
            key: ghostty_input_trigger_key_u { unicode: 0xD800 },
            ..t
        };
        assert!(keyboard_shortcut(&t).is_none());
    }

    #[test]
    fn mods_round_trip_and_sides() {
        let flags = NS_FLAG_SHIFT | NS_FLAG_COMMAND | NS_FLAG_CAPS_LOCK | NX_DEVICERCMDKEYMASK;
        let m = ghostty_mods(flags);
        assert_eq!(
            m,
            GHOSTTY_MODS_SHIFT | GHOSTTY_MODS_SUPER | GHOSTTY_MODS_CAPS | GHOSTTY_MODS_SUPER_RIGHT
        );
        assert_eq!(event_modifier_flags(m), NS_FLAG_SHIFT | NS_FLAG_COMMAND);
        assert_eq!(
            ghostty_mods(NX_DEVICERCTLKEYMASK | NX_DEVICERALTKEYMASK | NX_DEVICERSHIFTKEYMASK),
            GHOSTTY_MODS_CTRL_RIGHT | GHOSTTY_MODS_ALT_RIGHT | GHOSTTY_MODS_SHIFT_RIGHT
        );
        assert_eq!(ghostty_mods(0), GHOSTTY_MODS_NONE);
    }

    #[test]
    fn event_modifiers_conversions() {
        let m = EventModifiers::SHIFT | EventModifiers::OPTION | EventModifiers::CAPS_LOCK;
        assert_eq!(
            event_modifiers_from_ns_flags(ns_flags_from_event_modifiers(m)),
            m
        );
    }

    #[test]
    fn binding_flags() {
        let f = BindingFlags::from_c_flags(
            GHOSTTY_BINDING_FLAGS_CONSUMED | GHOSTTY_BINDING_FLAGS_GLOBAL,
        );
        assert!(f.contains(BindingFlags::CONSUMED | BindingFlags::GLOBAL));
        assert!(!f.contains(BindingFlags::PERFORMABLE));
        assert_eq!(f.c_flags(), 5);
    }
}
