//! Rust stand-ins for the SwiftUI value types the Swift app uses to carry key
//! equivalents (`KeyEquivalent`, `EventModifiers`, `KeyboardShortcut`) plus
//! `Helpers/Extensions/KeyboardShortcut+Extension.swift` and
//! `EventModifiers+Extension.swift` (the `NSEvent.ModifierFlags` conversions
//! live in `helpers::extensions` because they need AppKit types).
//!
//! The values match SwiftUI's (`KeyEquivalent.upArrow` is `NSUpArrowFunctionKey`
//! etc.) so a menu item's `keyEquivalent` is the same string as in Swift.

use std::fmt;

use bitflags::bitflags;

/// `SwiftUI.KeyEquivalent`: a single character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEquivalent {
    pub character: char,
}

impl KeyEquivalent {
    pub const UP_ARROW: Self = Self::new('\u{F700}');
    pub const DOWN_ARROW: Self = Self::new('\u{F701}');
    pub const LEFT_ARROW: Self = Self::new('\u{F702}');
    pub const RIGHT_ARROW: Self = Self::new('\u{F703}');
    pub const ESCAPE: Self = Self::new('\u{1B}');
    pub const RETURN: Self = Self::new('\r');
    pub const TAB: Self = Self::new('\t');
    pub const SPACE: Self = Self::new(' ');
    pub const DELETE: Self = Self::new('\u{7F}');
    pub const DELETE_FORWARD: Self = Self::new('\u{F728}');
    pub const HOME: Self = Self::new('\u{F729}');
    pub const END: Self = Self::new('\u{F72B}');
    pub const PAGE_UP: Self = Self::new('\u{F72C}');
    pub const PAGE_DOWN: Self = Self::new('\u{F72D}');
    pub const CLEAR: Self = Self::new('\u{F739}');

    #[must_use]
    pub const fn new(character: char) -> Self {
        Self { character }
    }
}

bitflags! {
    /// `SwiftUI.EventModifiers` (same raw values).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct EventModifiers: u32 {
        const CAPS_LOCK = 1 << 1;
        const SHIFT = 1 << 0;
        const CONTROL = 1 << 2;
        const OPTION = 1 << 3;
        const COMMAND = 1 << 4;
        const NUMERIC_PAD = 1 << 5;
        const FUNCTION = 1 << 6;
    }
}

/// `SwiftUI.KeyboardShortcut`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyboardShortcut {
    pub key: KeyEquivalent,
    pub modifiers: EventModifiers,
}

impl KeyboardShortcut {
    #[must_use]
    pub const fn new(key: KeyEquivalent, modifiers: EventModifiers) -> Self {
        Self { key, modifiers }
    }

    /// `KeyboardShortcut.keyList`: each modifier symbol then the key.
    #[must_use]
    pub fn key_list(&self) -> Vec<String> {
        let mut result = Vec::new();
        if self.modifiers.contains(EventModifiers::CONTROL) {
            result.push("⌃".to_string());
        }
        if self.modifiers.contains(EventModifiers::OPTION) {
            result.push("⌥".to_string());
        }
        if self.modifiers.contains(EventModifiers::SHIFT) {
            result.push("⇧".to_string());
        }
        if self.modifiers.contains(EventModifiers::COMMAND) {
            result.push("⌘".to_string());
        }

        let key_string = match self.key {
            KeyEquivalent::RETURN => "⏎".to_string(),
            KeyEquivalent::ESCAPE => "⎋".to_string(),
            KeyEquivalent::DELETE => "⌫".to_string(),
            KeyEquivalent::SPACE => "␣".to_string(),
            KeyEquivalent::TAB => "⇥".to_string(),
            KeyEquivalent::UP_ARROW => "▲".to_string(),
            KeyEquivalent::DOWN_ARROW => "▼".to_string(),
            KeyEquivalent::LEFT_ARROW => "◀".to_string(),
            KeyEquivalent::RIGHT_ARROW => "▶".to_string(),
            KeyEquivalent::PAGE_UP => "↑".to_string(),
            KeyEquivalent::PAGE_DOWN => "↓".to_string(),
            KeyEquivalent::HOME => "⤒".to_string(),
            KeyEquivalent::END => "⤓".to_string(),
            other => other.character.to_uppercase().collect(),
        };
        result.push(key_string);
        result
    }
}

impl fmt::Display for KeyboardShortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key_list().join(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_lists_modifiers_in_swift_order() {
        let s = KeyboardShortcut::new(
            KeyEquivalent::new('t'),
            EventModifiers::COMMAND | EventModifiers::SHIFT | EventModifiers::CONTROL,
        );
        assert_eq!(s.to_string(), "⌃⇧⌘T");
    }

    #[test]
    fn special_keys_use_symbols() {
        let s = KeyboardShortcut::new(KeyEquivalent::UP_ARROW, EventModifiers::OPTION);
        assert_eq!(s.key_list(), vec!["⌥", "▲"]);
        let s = KeyboardShortcut::new(KeyEquivalent::DELETE, EventModifiers::empty());
        assert_eq!(s.to_string(), "⌫");
    }
}
