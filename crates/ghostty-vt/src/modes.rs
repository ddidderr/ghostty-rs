//! Terminal modes (ZIG/src/terminal/modes.zig): an enum of supported modes,
//! a packed bit storage for mode values, and a state struct that handles
//! save/restore.
//!
//! In Zig the enum, packed struct and lookup are generated at comptime from
//! one table. Here the single [`ENTRIES`] table drives everything and the
//! `Mode` discriminant doubles as the bit index in [`ModePacked`].

/// A struct that maintains the state of all the settable modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModeState {
    /// The values of the current modes.
    pub values: ModePacked,
    /// The saved values. We only allow saving each mode once.
    /// This is in line with other terminals that implement XTSAVE
    /// and XTRESTORE. We can improve this in the future if it becomes
    /// a real-world issue but we need to be aware of a DoS vector.
    pub saved: ModePacked,
    /// The default values for the modes. This is used to reset
    /// the modes to their default values during reset.
    pub default: ModePacked,
}

impl ModeState {
    /// Reset the modes to their default values. This also clears the
    /// saved state.
    pub fn reset(&mut self) {
        self.values = self.default;
        self.saved = ModePacked::default();
    }

    /// Set a mode to a value.
    pub fn set(&mut self, mode: Mode, value: bool) {
        self.values.set(mode, value);
    }

    /// Get the value of a mode.
    pub fn get(&self, mode: Mode) -> bool {
        self.values.get(mode)
    }

    /// Save the state of the given mode. This can then be restored
    /// with restore. This will only be accurate if the previous
    /// mode was saved exactly once and not restored. Otherwise this
    /// will just keep restoring the last stored value in memory.
    pub fn save(&mut self, mode: Mode) {
        let v = self.values.get(mode);
        self.saved.set(mode, v);
    }

    /// See save. This will return the restored value.
    pub fn restore(&mut self, mode: Mode) -> bool {
        let v = self.saved.get(mode);
        self.values.set(mode, v);
        v
    }
}

/// Packed storage of all the settable modes (one bit per mode). This
/// shouldn't be used directly but rather through the [`ModeState`] struct.
/// Defaults match each entry's default value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModePacked(u64);

impl Default for ModePacked {
    fn default() -> Self {
        let mut bits = 0u64;
        for e in ENTRIES {
            if e.default {
                bits |= 1 << (e.mode as u64);
            }
        }
        Self(bits)
    }
}

impl ModePacked {
    pub fn get(&self, mode: Mode) -> bool {
        self.0 & (1 << (mode as u64)) != 0
    }

    pub fn set(&mut self, mode: Mode, value: bool) {
        if value {
            self.0 |= 1 << (mode as u64);
        } else {
            self.0 &= !(1 << (mode as u64));
        }
    }

    /// Raw bits.
    pub fn bits(&self) -> u64 {
        self.0
    }
}

/// The available modes. See [`ENTRIES`] for numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Mode {
    DisableKeyboard,
    Insert,
    SendReceiveMode,
    Linefeed,
    CursorKeys,
    M132Column,
    SlowScroll,
    ReverseColors,
    Origin,
    Wraparound,
    Autorepeat,
    MouseEventX10,
    CursorBlinking,
    CursorVisible,
    EnableMode3,
    ReverseWrap,
    AltScreenLegacy,
    KeypadKeys,
    EnableLeftAndRightMargin,
    MouseEventNormal,
    MouseEventButton,
    MouseEventAny,
    FocusEvent,
    MouseFormatUtf8,
    MouseFormatSgr,
    MouseAlternateScroll,
    MouseFormatUrxvt,
    MouseFormatSgrPixels,
    IgnoreKeypadWithNumlock,
    AltEscPrefix,
    AltSendsEscape,
    ReverseWrapExtended,
    AltScreen,
    SaveCursor,
    AltScreenSaveCursorClearEnter,
    BracketedPaste,
    SynchronizedOutput,
    GraphemeCluster,
    ReportColorScheme,
    InBandSizeReports,
}

impl Mode {
    /// The entry for this mode.
    pub fn entry(self) -> &'static Entry {
        &ENTRIES[self as usize]
    }

    /// The numeric value of the mode.
    pub fn value(self) -> u16 {
        self.entry().value
    }

    /// True if this is an ANSI mode, false if its a DEC mode (?-prefixed).
    pub fn ansi(self) -> bool {
        self.entry().ansi
    }

    /// The snake_case name of the mode (as in the Zig tag name).
    pub fn name(self) -> &'static str {
        self.entry().name
    }
}

/// Look up a mode from its numeric value and ANSI-ness.
pub fn mode_from_int(v: u16, ansi: bool) -> Option<Mode> {
    ENTRIES
        .iter()
        .find(|e| !e.disabled && e.value == v && e.ansi == ansi)
        .map(|e| e.mode)
}

/// A single entry of a possible mode we support.
#[derive(Debug)]
pub struct Entry {
    pub mode: Mode,
    pub name: &'static str,
    pub value: u16,
    pub default: bool,
    /// True if this is an ANSI mode, false if its a DEC mode (?-prefixed).
    pub ansi: bool,
    /// If true, this mode is disabled and Ghostty will not allow it to be
    /// set or queried. The mode enum still has it, allowing Ghostty developers
    /// to develop a mode without exposing it to real users.
    pub disabled: bool,
}

#[allow(non_snake_case)]
const fn Entry(mode: Mode, name: &'static str, value: u16, default: bool, ansi: bool) -> Entry {
    Entry {
        mode,
        name,
        value,
        default,
        ansi,
        disabled: false,
    }
}

/// The full list of available entries, in `Mode` declaration order.
pub const ENTRIES: &[Entry; 40] = &[
    Entry(Mode::DisableKeyboard, "disable_keyboard", 2, false, true),
    Entry(Mode::Insert, "insert", 4, false, true),
    Entry(Mode::SendReceiveMode, "send_receive_mode", 12, true, true),
    Entry(Mode::Linefeed, "linefeed", 20, false, true),
    Entry(Mode::CursorKeys, "cursor_keys", 1, false, false),
    Entry(Mode::M132Column, "132_column", 3, false, false),
    Entry(Mode::SlowScroll, "slow_scroll", 4, false, false),
    Entry(Mode::ReverseColors, "reverse_colors", 5, false, false),
    Entry(Mode::Origin, "origin", 6, false, false),
    Entry(Mode::Wraparound, "wraparound", 7, true, false),
    Entry(Mode::Autorepeat, "autorepeat", 8, false, false),
    Entry(Mode::MouseEventX10, "mouse_event_x10", 9, false, false),
    Entry(Mode::CursorBlinking, "cursor_blinking", 12, false, false),
    Entry(Mode::CursorVisible, "cursor_visible", 25, true, false),
    Entry(Mode::EnableMode3, "enable_mode_3", 40, false, false),
    Entry(Mode::ReverseWrap, "reverse_wrap", 45, false, false),
    Entry(Mode::AltScreenLegacy, "alt_screen_legacy", 47, false, false),
    Entry(Mode::KeypadKeys, "keypad_keys", 66, false, false),
    Entry(
        Mode::EnableLeftAndRightMargin,
        "enable_left_and_right_margin",
        69,
        false,
        false,
    ),
    Entry(
        Mode::MouseEventNormal,
        "mouse_event_normal",
        1000,
        false,
        false,
    ),
    Entry(
        Mode::MouseEventButton,
        "mouse_event_button",
        1002,
        false,
        false,
    ),
    Entry(Mode::MouseEventAny, "mouse_event_any", 1003, false, false),
    Entry(Mode::FocusEvent, "focus_event", 1004, false, false),
    Entry(
        Mode::MouseFormatUtf8,
        "mouse_format_utf8",
        1005,
        false,
        false,
    ),
    Entry(Mode::MouseFormatSgr, "mouse_format_sgr", 1006, false, false),
    Entry(
        Mode::MouseAlternateScroll,
        "mouse_alternate_scroll",
        1007,
        true,
        false,
    ),
    Entry(
        Mode::MouseFormatUrxvt,
        "mouse_format_urxvt",
        1015,
        false,
        false,
    ),
    Entry(
        Mode::MouseFormatSgrPixels,
        "mouse_format_sgr_pixels",
        1016,
        false,
        false,
    ),
    Entry(
        Mode::IgnoreKeypadWithNumlock,
        "ignore_keypad_with_numlock",
        1035,
        true,
        false,
    ),
    Entry(Mode::AltEscPrefix, "alt_esc_prefix", 1036, true, false),
    Entry(Mode::AltSendsEscape, "alt_sends_escape", 1039, false, false),
    Entry(
        Mode::ReverseWrapExtended,
        "reverse_wrap_extended",
        1045,
        false,
        false,
    ),
    Entry(Mode::AltScreen, "alt_screen", 1047, false, false),
    Entry(Mode::SaveCursor, "save_cursor", 1048, false, false),
    Entry(
        Mode::AltScreenSaveCursorClearEnter,
        "alt_screen_save_cursor_clear_enter",
        1049,
        false,
        false,
    ),
    Entry(Mode::BracketedPaste, "bracketed_paste", 2004, false, false),
    Entry(
        Mode::SynchronizedOutput,
        "synchronized_output",
        2026,
        false,
        false,
    ),
    Entry(
        Mode::GraphemeCluster,
        "grapheme_cluster",
        2027,
        false,
        false,
    ),
    Entry(
        Mode::ReportColorScheme,
        "report_color_scheme",
        2031,
        false,
        false,
    ),
    Entry(
        Mode::InBandSizeReports,
        "in_band_size_reports",
        2048,
        false,
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_in_order() {
        for (i, e) in ENTRIES.iter().enumerate() {
            assert_eq!(e.mode as usize, i);
        }
        assert!(ENTRIES.len() <= 64);
    }

    #[test]
    fn mode_from_int_lookup() {
        assert_eq!(mode_from_int(4, true), Some(Mode::Insert));
        assert_eq!(mode_from_int(9, true), None);
        assert_eq!(mode_from_int(9, false), Some(Mode::MouseEventX10));
        assert_eq!(mode_from_int(14, true), None);
    }

    #[test]
    fn mode_state() {
        let mut state = ModeState::default();
        // Normal set/get
        assert!(!state.get(Mode::CursorKeys));
        state.set(Mode::CursorKeys, true);
        assert!(state.get(Mode::CursorKeys));

        // Save/restore
        state.save(Mode::CursorKeys);
        state.set(Mode::CursorKeys, false);
        assert!(!state.get(Mode::CursorKeys));
        assert!(state.restore(Mode::CursorKeys));
        assert!(state.get(Mode::CursorKeys));
    }

    #[test]
    fn defaults_and_reset() {
        let mut state = ModeState::default();
        assert!(state.get(Mode::Wraparound));
        assert!(state.get(Mode::CursorVisible));
        assert!(state.get(Mode::SendReceiveMode));
        assert!(!state.get(Mode::Origin));
        state.set(Mode::Wraparound, false);
        state.reset();
        assert!(state.get(Mode::Wraparound));
    }

    #[test]
    fn mode_tag_order() {
        // Mirrors Zig ModeTag "order": value in low bits, ansi in the top bit.
        assert_eq!(Mode::CursorKeys.value(), 1);
        assert!(!Mode::CursorKeys.ansi());
    }

    #[test]
    fn packed_size() {
        // Zig asserts @sizeOf(ModePacked) == 8.
        assert_eq!(std::mem::size_of::<ModePacked>(), 8);
    }
}
