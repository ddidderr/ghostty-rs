//! `Helpers/Extensions/EventModifiers+Extension.swift`: conversions between
//! `SwiftUI.EventModifiers` ([`EventModifiers`]) and `NSEvent.ModifierFlags`.
//!
//! The conversion works on the raw `NSEventModifierFlags` bits (documented,
//! stable values) so it is testable everywhere; the typed
//! `NSEventModifierFlags` conversions are thin wrappers (Apple only).

use crate::helpers::keyboard_shortcut::EventModifiers;

pub const NS_CAPS_LOCK: usize = 1 << 16;
pub const NS_SHIFT: usize = 1 << 17;
pub const NS_CONTROL: usize = 1 << 18;
pub const NS_OPTION: usize = 1 << 19;
pub const NS_COMMAND: usize = 1 << 20;

/// `EventModifiers.init(nsFlags:)` on raw `NSEvent.ModifierFlags` bits.
#[must_use]
pub fn event_modifiers_from_ns_bits(ns: usize) -> EventModifiers {
    let mut result = EventModifiers::empty();
    if ns & NS_SHIFT != 0 {
        result.insert(EventModifiers::SHIFT);
    }
    if ns & NS_CONTROL != 0 {
        result.insert(EventModifiers::CONTROL);
    }
    if ns & NS_OPTION != 0 {
        result.insert(EventModifiers::OPTION);
    }
    if ns & NS_COMMAND != 0 {
        result.insert(EventModifiers::COMMAND);
    }
    if ns & NS_CAPS_LOCK != 0 {
        result.insert(EventModifiers::CAPS_LOCK);
    }
    result
}

/// `NSEvent.ModifierFlags.init(swiftUIFlags:)` as raw bits.
#[must_use]
pub fn ns_bits_from_event_modifiers(flags: EventModifiers) -> usize {
    let mut result = 0;
    if flags.contains(EventModifiers::SHIFT) {
        result |= NS_SHIFT;
    }
    if flags.contains(EventModifiers::CONTROL) {
        result |= NS_CONTROL;
    }
    if flags.contains(EventModifiers::OPTION) {
        result |= NS_OPTION;
    }
    if flags.contains(EventModifiers::COMMAND) {
        result |= NS_COMMAND;
    }
    if flags.contains(EventModifiers::CAPS_LOCK) {
        result |= NS_CAPS_LOCK;
    }
    result
}

/// `EventModifiers(nsFlags:)`.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn event_modifiers_from_ns_flags(flags: objc2_app_kit::NSEventModifierFlags) -> EventModifiers {
    event_modifiers_from_ns_bits(flags.0)
}

/// `NSEvent.ModifierFlags(swiftUIFlags:)`.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn ns_flags_from_event_modifiers(flags: EventModifiers) -> objc2_app_kit::NSEventModifierFlags {
    objc2_app_kit::NSEventModifierFlags(ns_bits_from_event_modifiers(flags))
}

/// Alias of [`ns_flags_from_event_modifiers`] (`NSEvent.ModifierFlags(swiftUIFlags:)`).
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn ns_flags_from_swiftui(flags: EventModifiers) -> objc2_app_kit::NSEventModifierFlags {
    ns_flags_from_event_modifiers(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_known_flags() {
        let all = EventModifiers::SHIFT
            | EventModifiers::CONTROL
            | EventModifiers::OPTION
            | EventModifiers::COMMAND
            | EventModifiers::CAPS_LOCK;
        assert_eq!(
            event_modifiers_from_ns_bits(ns_bits_from_event_modifiers(all)),
            all
        );
        assert_eq!(
            ns_bits_from_event_modifiers(EventModifiers::COMMAND),
            1 << 20
        );
        assert_eq!(event_modifiers_from_ns_bits(1 << 17), EventModifiers::SHIFT);
    }

    #[test]
    fn other_bits_are_ignored() {
        // numeric pad / function / device bits are not translated
        assert_eq!(
            event_modifiers_from_ns_bits((1 << 21) | (1 << 23) | 0x2),
            EventModifiers::empty()
        );
        assert_eq!(ns_bits_from_event_modifiers(EventModifiers::FUNCTION), 0);
    }
}
