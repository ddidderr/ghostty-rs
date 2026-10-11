//! Kitty keyboard protocol key mapping data (ZIG/src/input/kitty.zig).

use crate::key::Key;

/// A single entry in the kitty keymap data. There are only ~100 entries
/// so the recommendation is to just use a linear search to find the entry
/// for a given key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub key: Key,
    pub code: u32,
    pub final_: u8,
    pub modifier: bool,
}

const fn e(key: Key, code: u32, final_: u8, modifier: bool) -> Entry {
    Entry {
        key,
        code,
        final_,
        modifier,
    }
}

/// The raw data for how to map keys to Kitty data. Based on the information:
/// https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions
/// And the exact table is ported from Foot:
/// https://codeberg.org/dnkl/foot/src/branch/master/kitty-keymap.h
///
/// Note that we currently don't support all the same keysyms as Kitty,
/// but we can add them as we add support.
///
/// These are stored in order of the above Kitty link, so it is easy to
/// find the entry for a given key.
pub const ENTRIES: &[Entry] = &[
    e(Key::Escape, 27, b'u', false),
    e(Key::Enter, 13, b'u', false),
    e(Key::Tab, 9, b'u', false),
    e(Key::Backspace, 127, b'u', false),
    e(Key::Insert, 2, b'~', false),
    e(Key::Delete, 3, b'~', false),
    e(Key::ArrowLeft, 1, b'D', false),
    e(Key::ArrowRight, 1, b'C', false),
    e(Key::ArrowUp, 1, b'A', false),
    e(Key::ArrowDown, 1, b'B', false),
    e(Key::PageUp, 5, b'~', false),
    e(Key::PageDown, 6, b'~', false),
    e(Key::Home, 1, b'H', false),
    e(Key::End, 1, b'F', false),
    e(Key::CapsLock, 57358, b'u', true),
    e(Key::ScrollLock, 57359, b'u', false),
    e(Key::NumLock, 57360, b'u', true),
    e(Key::PrintScreen, 57361, b'u', false),
    e(Key::Pause, 57362, b'u', false),
    e(Key::F1, 1, b'P', false),
    e(Key::F2, 1, b'Q', false),
    e(Key::F3, 13, b'~', false),
    e(Key::F4, 1, b'S', false),
    e(Key::F5, 15, b'~', false),
    e(Key::F6, 17, b'~', false),
    e(Key::F7, 18, b'~', false),
    e(Key::F8, 19, b'~', false),
    e(Key::F9, 20, b'~', false),
    e(Key::F10, 21, b'~', false),
    e(Key::F11, 23, b'~', false),
    e(Key::F12, 24, b'~', false),
    e(Key::F13, 57376, b'u', false),
    e(Key::F14, 57377, b'u', false),
    e(Key::F15, 57378, b'u', false),
    e(Key::F16, 57379, b'u', false),
    e(Key::F17, 57380, b'u', false),
    e(Key::F18, 57381, b'u', false),
    e(Key::F19, 57382, b'u', false),
    e(Key::F20, 57383, b'u', false),
    e(Key::F21, 57384, b'u', false),
    e(Key::F22, 57385, b'u', false),
    e(Key::F23, 57386, b'u', false),
    e(Key::F24, 57387, b'u', false),
    e(Key::F25, 57388, b'u', false),
    e(Key::Numpad0, 57399, b'u', false),
    e(Key::Numpad1, 57400, b'u', false),
    e(Key::Numpad2, 57401, b'u', false),
    e(Key::Numpad3, 57402, b'u', false),
    e(Key::Numpad4, 57403, b'u', false),
    e(Key::Numpad5, 57404, b'u', false),
    e(Key::Numpad6, 57405, b'u', false),
    e(Key::Numpad7, 57406, b'u', false),
    e(Key::Numpad8, 57407, b'u', false),
    e(Key::Numpad9, 57408, b'u', false),
    e(Key::NumpadDecimal, 57409, b'u', false),
    e(Key::NumpadDivide, 57410, b'u', false),
    e(Key::NumpadMultiply, 57411, b'u', false),
    e(Key::NumpadSubtract, 57412, b'u', false),
    e(Key::NumpadAdd, 57413, b'u', false),
    e(Key::NumpadEnter, 57414, b'u', false),
    e(Key::NumpadEqual, 57415, b'u', false),
    e(Key::NumpadSeparator, 57416, b'u', false),
    e(Key::NumpadLeft, 57417, b'u', false),
    e(Key::NumpadRight, 57418, b'u', false),
    e(Key::NumpadUp, 57419, b'u', false),
    e(Key::NumpadDown, 57420, b'u', false),
    e(Key::NumpadPageUp, 57421, b'u', false),
    e(Key::NumpadPageDown, 57422, b'u', false),
    e(Key::NumpadHome, 57423, b'u', false),
    e(Key::NumpadEnd, 57424, b'u', false),
    e(Key::NumpadInsert, 57425, b'u', false),
    e(Key::NumpadDelete, 57426, b'u', false),
    e(Key::NumpadBegin, 57427, b'u', false),
    e(Key::ShiftLeft, 57441, b'u', true),
    e(Key::ShiftRight, 57447, b'u', true),
    e(Key::ControlLeft, 57442, b'u', true),
    e(Key::ControlRight, 57448, b'u', true),
    e(Key::MetaLeft, 57444, b'u', true),
    e(Key::MetaRight, 57450, b'u', true),
    e(Key::AltLeft, 57443, b'u', true),
    e(Key::AltRight, 57449, b'u', true),
];

/// The full list of entries for the current platform.
pub fn entries() -> &'static [Entry] {
    ENTRIES
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn entries_table() {
        // To force evaluation of the table
        assert_ne!(entries().len(), 0);
    }
}
