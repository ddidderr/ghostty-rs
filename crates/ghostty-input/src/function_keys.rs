//! This is the list of "PC style function keys" that xterm supports for
//! the legacy keyboard protocols. These always take priority since at the
//! time of writing this, even the most modern keyboard protocols still
//! are backwards compatible with regards to these sequences.
//!
//! This is based on a variety of sources cross-referenced but mostly
//! based on foot's keymap.h: https://codeberg.org/dnkl/foot/src/branch/master/keymap.h
//! (ZIG/src/input/function_keys.zig)

use std::sync::LazyLock;

use crate::key::{Key, Mods};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    Any,
    Normal,
    Application,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeypadMode {
    Any,
    Normal,
    Application,
}

/// A bit confusing so I'll document this one: this is the "modify other keys"
/// setting. We only change behavior for "set_other" which is ESC [ > 4; 2 m.
/// So this can be "any" which means we don't care what's going on. Or it
/// can be "set" which means modify keys must be set EXCEPT FOR "other keys"
/// mode, and "set_other" which means modify keys must be set to "other keys"
/// mode.
///
/// See: https://invisible-island.net/xterm/modified-keys.html
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyKeys {
    Any,
    Set,
    SetOther,
}

/// A single entry in the table of keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The exact set of modifiers that must be active for this entry to match.
    /// If mods_empty_is_any is true then empty mods means any set of mods can
    /// match. Otherwise, empty mods means no mods must be active.
    pub mods: Mods,
    pub mods_empty_is_any: bool,

    /// The state required for cursor/keypad mode.
    pub cursor: CursorMode,
    pub keypad: KeypadMode,

    /// Whether or not this entry should be used
    pub modify_other_keys: ModifyKeys,

    /// The sequence to send to the pty if this entry matches.
    pub sequence: String,
}

impl Entry {
    fn new(sequence: impl Into<String>) -> Entry {
        Entry {
            mods: Mods::default(),
            mods_empty_is_any: true,
            cursor: CursorMode::Any,
            keypad: KeypadMode::Any,
            modify_other_keys: ModifyKeys::Any,
            sequence: sequence.into(),
        }
    }
}

const fn m(shift: bool, ctrl: bool, alt: bool, super_: bool) -> Mods {
    Mods {
        shift,
        ctrl,
        alt,
        super_,
        caps_lock: false,
        num_lock: false,
        sides: crate::key_mods::Side {
            shift: crate::key_mods::ModSide::Left,
            ctrl: crate::key_mods::ModSide::Left,
            alt: crate::key_mods::ModSide::Left,
            super_: crate::key_mods::ModSide::Left,
        },
    }
}

/// The list of modifier combinations for modify other key sequences.
/// The mode value is index + 2.
pub const MODIFIERS: &[Mods] = &[
    m(true, false, false, false),
    m(false, false, true, false),
    m(true, false, true, false),
    m(false, true, false, false),
    m(true, true, false, false),
    m(false, true, true, false),
    m(true, true, true, false),
    m(false, false, false, true),
    m(true, false, false, true),
    m(false, false, true, true),
    m(true, false, true, true),
    m(false, true, false, true),
    m(true, true, false, true),
    m(false, true, true, true),
    m(true, true, true, true),
];

fn ent(mods: Mods, modify_other_keys: ModifyKeys, sequence: &str) -> Entry {
    Entry {
        mods,
        modify_other_keys,
        ..Entry::new(sequence)
    }
}

/// Returns the entries for a key. Not every key has any entries. It is
/// expected that you'll just use a for loop through the entry values, there
/// are never more than a couple dozen or so.
pub fn keys(k: Key) -> &'static [Entry] {
    &KEYS[k as usize]
}

/// Iterate over the whole table (all keys, in `Key` order).
pub fn all_keys() -> impl Iterator<Item = &'static [Entry]> {
    KEYS.iter().map(Vec::as_slice)
}

static KEYS: LazyLock<Vec<Vec<Entry>>> = LazyLock::new(build_keys);

fn cat(mut a: Vec<Entry>, b: Vec<Entry>) -> Vec<Entry> {
    a.extend(b);
    a
}

fn one(seq: &str) -> Vec<Entry> {
    vec![Entry::new(seq)]
}

fn build_keys() -> Vec<Vec<Entry>> {
    let mut result: Vec<Vec<Entry>> = vec![Vec::new(); Key::ALL.len()];
    let mut set = |k: Key, v: Vec<Entry>| result[k as usize] = v;

    set(
        Key::ArrowUp,
        cat(pc_style("\x1b[1;{}A"), cursor_key("\x1b[A", "\x1bOA")),
    );
    set(
        Key::ArrowDown,
        cat(pc_style("\x1b[1;{}B"), cursor_key("\x1b[B", "\x1bOB")),
    );
    set(
        Key::ArrowRight,
        cat(pc_style("\x1b[1;{}C"), cursor_key("\x1b[C", "\x1bOC")),
    );
    set(
        Key::ArrowLeft,
        cat(pc_style("\x1b[1;{}D"), cursor_key("\x1b[D", "\x1bOD")),
    );
    set(
        Key::Home,
        cat(pc_style("\x1b[1;{}H"), cursor_key("\x1b[H", "\x1bOH")),
    );
    set(
        Key::End,
        cat(pc_style("\x1b[1;{}F"), cursor_key("\x1b[F", "\x1bOF")),
    );
    set(Key::Insert, cat(pc_style("\x1b[2;{}~"), one("\x1B[2~")));
    set(Key::Delete, cat(pc_style("\x1b[3;{}~"), one("\x1B[3~")));
    set(Key::PageUp, cat(pc_style("\x1b[5;{}~"), one("\x1B[5~")));
    set(Key::PageDown, cat(pc_style("\x1b[6;{}~"), one("\x1B[6~")));

    // Function Keys. todo: f13-f35 but we need to add to input.Key
    set(Key::F1, cat(pc_style("\x1b[1;{}P"), one("\x1BOP")));
    set(Key::F2, cat(pc_style("\x1b[1;{}Q"), one("\x1BOQ")));
    set(Key::F3, cat(pc_style("\x1b[13;{}~"), one("\x1BOR")));
    set(Key::F4, cat(pc_style("\x1b[1;{}S"), one("\x1BOS")));
    set(Key::F5, cat(pc_style("\x1b[15;{}~"), one("\x1B[15~")));
    set(Key::F6, cat(pc_style("\x1b[17;{}~"), one("\x1B[17~")));
    set(Key::F7, cat(pc_style("\x1b[18;{}~"), one("\x1B[18~")));
    set(Key::F8, cat(pc_style("\x1b[19;{}~"), one("\x1B[19~")));
    set(Key::F9, cat(pc_style("\x1b[20;{}~"), one("\x1B[20~")));
    set(Key::F10, cat(pc_style("\x1b[21;{}~"), one("\x1B[21~")));
    set(Key::F11, cat(pc_style("\x1b[23;{}~"), one("\x1B[23~")));
    set(Key::F12, cat(pc_style("\x1b[24;{}~"), one("\x1B[24~")));

    // Keypad keys
    set(Key::Numpad0, kp_keys("p"));
    set(Key::Numpad1, kp_keys("q"));
    set(Key::Numpad2, kp_keys("r"));
    set(Key::Numpad3, kp_keys("s"));
    set(Key::Numpad4, kp_keys("t"));
    set(Key::Numpad5, kp_keys("u"));
    set(Key::Numpad6, kp_keys("v"));
    set(Key::Numpad7, kp_keys("w"));
    set(Key::Numpad8, kp_keys("x"));
    set(Key::Numpad9, kp_keys("y"));
    set(Key::NumpadDecimal, kp_keys("n"));
    set(Key::NumpadDivide, kp_keys("o"));
    set(Key::NumpadMultiply, kp_keys("j"));
    set(Key::NumpadSubtract, kp_keys("m"));
    set(Key::NumpadAdd, kp_keys("k"));
    set(Key::NumpadEnter, cat(kp_keys("M"), one("\r")));
    set(
        Key::NumpadUp,
        cat(pc_style("\x1b[1;{}A"), cursor_key("\x1b[A", "\x1bOA")),
    );
    set(
        Key::NumpadDown,
        cat(pc_style("\x1b[1;{}B"), cursor_key("\x1b[B", "\x1bOB")),
    );
    set(
        Key::NumpadRight,
        cat(pc_style("\x1b[1;{}C"), cursor_key("\x1b[C", "\x1bOC")),
    );
    set(
        Key::NumpadLeft,
        cat(pc_style("\x1b[1;{}D"), cursor_key("\x1b[D", "\x1bOD")),
    );
    set(
        Key::NumpadBegin,
        cat(pc_style("\x1b[1;{}E"), cursor_key("\x1b[E", "\x1bOE")),
    );
    set(
        Key::NumpadHome,
        cat(pc_style("\x1b[1;{}H"), cursor_key("\x1b[H", "\x1bOH")),
    );
    set(
        Key::NumpadEnd,
        cat(pc_style("\x1b[1;{}F"), cursor_key("\x1b[F", "\x1bOF")),
    );
    set(
        Key::NumpadInsert,
        cat(pc_style("\x1b[2;{}~"), one("\x1B[2~")),
    );
    set(
        Key::NumpadDelete,
        cat(pc_style("\x1b[3;{}~"), one("\x1B[3~")),
    );
    set(
        Key::NumpadPageUp,
        cat(pc_style("\x1b[5;{}~"), one("\x1B[5~")),
    );
    set(
        Key::NumpadPageDown,
        cat(pc_style("\x1b[6;{}~"), one("\x1B[6~")),
    );

    set(
        Key::Backspace,
        vec![
            // Modify Keys Normal / Other
            ent(m(true, false, false, false), ModifyKeys::Set, "\x7f"),
            ent(m(false, false, true, false), ModifyKeys::Set, "\x1b\x7f"),
            ent(m(true, false, true, false), ModifyKeys::Set, "\x1b\x7f"),
            ent(m(true, true, false, false), ModifyKeys::Set, "\x08"),
            ent(m(false, true, true, false), ModifyKeys::Set, "\x1b\x08"),
            ent(m(false, false, false, true), ModifyKeys::Set, "\x7f"),
            ent(m(true, false, false, true), ModifyKeys::Set, "\x7f"),
            ent(m(false, false, true, true), ModifyKeys::Set, "\x1b\x7f"),
            ent(m(true, false, true, true), ModifyKeys::Set, "\x1b\x7f"),
            ent(m(false, true, false, true), ModifyKeys::Set, "\x08"),
            ent(m(true, true, false, true), ModifyKeys::Set, "\x08"),
            ent(m(false, true, true, true), ModifyKeys::Set, "\x1b\x08"),
            ent(m(true, true, true, true), ModifyKeys::Set, "\x1b\x08"),
            ent(
                m(true, false, false, false),
                ModifyKeys::SetOther,
                "\x1b[27;2;127~",
            ),
            ent(
                m(false, false, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;3;127~",
            ),
            ent(
                m(true, false, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;4;127~",
            ),
            ent(
                m(true, true, false, false),
                ModifyKeys::SetOther,
                "\x1b[27;6;127~",
            ),
            ent(
                m(false, true, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;7;127~",
            ),
            ent(
                m(true, true, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;8;127~",
            ),
            ent(
                m(false, false, false, true),
                ModifyKeys::SetOther,
                "\x1b[27;9;127~",
            ),
            ent(
                m(true, false, false, true),
                ModifyKeys::SetOther,
                "\x1b[27;10;127~",
            ),
            ent(
                m(false, false, true, true),
                ModifyKeys::SetOther,
                "\x1b[27;11;127~",
            ),
            ent(
                m(true, false, true, true),
                ModifyKeys::SetOther,
                "\x1b[27;12;127~",
            ),
            ent(
                m(false, true, false, true),
                ModifyKeys::SetOther,
                "\x1b[27;13;127~",
            ),
            ent(
                m(true, true, false, true),
                ModifyKeys::SetOther,
                "\x1b[27;14;127~",
            ),
            ent(
                m(false, true, true, true),
                ModifyKeys::SetOther,
                "\x1b[27;15;127~",
            ),
            ent(
                m(true, true, true, true),
                ModifyKeys::SetOther,
                "\x1b[27;16;127~",
            ),
            ent(m(false, true, false, false), ModifyKeys::Any, "\x08"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\x7f"),
            ent(m(false, true, false, false), ModifyKeys::Any, "\x08"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\x7f"),
        ],
    );

    set(
        Key::Tab,
        vec![
            ent(m(true, false, false, false), ModifyKeys::Set, "\x1b[Z"),
            ent(m(false, false, true, false), ModifyKeys::Set, "\x1b\t"),
            ent(
                m(true, false, false, false),
                ModifyKeys::SetOther,
                "\x1b[27;2;9~",
            ),
            ent(
                m(false, false, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;3;9~",
            ),
            ent(m(true, false, true, false), ModifyKeys::Any, "\x1b[27;4;9~"),
            ent(
                m(false, true, false, false),
                ModifyKeys::Any,
                "\x1b[27;5;9~",
            ),
            ent(m(true, true, false, false), ModifyKeys::Any, "\x1b[27;6;9~"),
            ent(m(false, true, true, false), ModifyKeys::Any, "\x1b[27;7;9~"),
            ent(m(true, true, true, false), ModifyKeys::Any, "\x1b[27;8;9~"),
            ent(
                m(false, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;9;9~",
            ),
            ent(
                m(true, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;10;9~",
            ),
            ent(
                m(false, false, true, true),
                ModifyKeys::Any,
                "\x1b[27;11;9~",
            ),
            ent(m(true, false, true, true), ModifyKeys::Any, "\x1b[27;12;9~"),
            ent(
                m(false, true, false, true),
                ModifyKeys::Any,
                "\x1b[27;13;9~",
            ),
            ent(m(true, true, false, true), ModifyKeys::Any, "\x1b[27;14;9~"),
            ent(m(false, true, true, true), ModifyKeys::Any, "\x1b[27;15;9~"),
            ent(m(true, true, true, true), ModifyKeys::Any, "\x1b[27;16;9~"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\t"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\t"),
        ],
    );

    set(
        Key::Enter,
        vec![
            ent(
                m(true, false, false, false),
                ModifyKeys::Any,
                "\x1b[27;2;13~",
            ),
            ent(m(false, false, true, false), ModifyKeys::Set, "\x1b\r"),
            ent(
                m(false, false, true, false),
                ModifyKeys::SetOther,
                "\x1b[27;3;13~",
            ),
            ent(
                m(true, false, true, false),
                ModifyKeys::Any,
                "\x1b[27;4;13~",
            ),
            ent(
                m(false, true, false, false),
                ModifyKeys::Any,
                "\x1b[27;5;13~",
            ),
            ent(
                m(true, true, false, false),
                ModifyKeys::Any,
                "\x1b[27;6;13~",
            ),
            ent(
                m(false, true, true, false),
                ModifyKeys::Any,
                "\x1b[27;7;13~",
            ),
            ent(m(true, true, true, false), ModifyKeys::Any, "\x1b[27;8;13~"),
            ent(
                m(false, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;9;13~",
            ),
            ent(
                m(true, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;10;13~",
            ),
            ent(
                m(false, false, true, true),
                ModifyKeys::Any,
                "\x1b[27;11;13~",
            ),
            ent(
                m(true, false, true, true),
                ModifyKeys::Any,
                "\x1b[27;12;13~",
            ),
            ent(
                m(false, true, false, true),
                ModifyKeys::Any,
                "\x1b[27;13;13~",
            ),
            ent(
                m(true, true, false, true),
                ModifyKeys::Any,
                "\x1b[27;14;13~",
            ),
            ent(
                m(false, true, true, true),
                ModifyKeys::Any,
                "\x1b[27;15;13~",
            ),
            ent(m(true, true, true, true), ModifyKeys::Any, "\x1b[27;16;13~"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\r"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\r"),
        ],
    );

    set(
        Key::Escape,
        vec![
            ent(
                m(true, false, false, false),
                ModifyKeys::Any,
                "\x1b[27;2;27~",
            ),
            ent(m(false, false, true, false), ModifyKeys::Any, "\x1b\x1b"),
            ent(
                m(true, false, true, false),
                ModifyKeys::Any,
                "\x1b[27;4;27~",
            ),
            ent(
                m(false, true, false, false),
                ModifyKeys::Any,
                "\x1b[27;5;27~",
            ),
            ent(
                m(true, true, false, false),
                ModifyKeys::Any,
                "\x1b[27;6;27~",
            ),
            ent(
                m(false, true, true, false),
                ModifyKeys::Any,
                "\x1b[27;7;27~",
            ),
            ent(m(true, true, true, false), ModifyKeys::Any, "\x1b[27;8;27~"),
            ent(
                m(false, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;9;27~",
            ),
            ent(
                m(true, false, false, true),
                ModifyKeys::Any,
                "\x1b[27;10;27~",
            ),
            ent(
                m(false, false, true, true),
                ModifyKeys::Any,
                "\x1b[27;11;27~",
            ),
            ent(
                m(true, false, true, true),
                ModifyKeys::Any,
                "\x1b[27;12;27~",
            ),
            ent(
                m(false, true, false, true),
                ModifyKeys::Any,
                "\x1b[27;13;27~",
            ),
            ent(
                m(true, true, false, true),
                ModifyKeys::Any,
                "\x1b[27;14;27~",
            ),
            ent(
                m(false, true, true, true),
                ModifyKeys::Any,
                "\x1b[27;15;27~",
            ),
            ent(m(true, true, true, true), ModifyKeys::Any, "\x1b[27;16;27~"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\x1b"),
            ent(m(false, false, false, false), ModifyKeys::Any, "\x1b"),
        ],
    );

    result
}

/// Returns the default keypad application mode entry.
fn kp_default(suffix: &str) -> Vec<Entry> {
    vec![Entry {
        mods_empty_is_any: false,
        keypad: KeypadMode::Application,
        ..Entry::new(format!("\x1bO{suffix}"))
    }]
}

/// Returns the entries for a keypad key. The suffix is the final character
/// of the sent sequence, such as "r" for kp_2.
fn kp_keys(suffix: &str) -> Vec<Entry> {
    let mut pc = pc_style(&format!("\x1bO{{}}{suffix}"));
    for entry in &mut pc {
        entry.keypad = KeypadMode::Application;
    }
    cat(kp_default(suffix), pc)
}

/// Returns entries that are dependent on cursor key settings.
fn cursor_key(normal: &str, application: &str) -> Vec<Entry> {
    vec![
        Entry {
            cursor: CursorMode::Normal,
            ..Entry::new(normal)
        },
        Entry {
            cursor: CursorMode::Application,
            ..Entry::new(application)
        },
    ]
}

/// Constructs a set of pcStyle function keys using the given format. The
/// format should have exactly one "hole" (`{}`) for the mods code.
/// Example: "\x1b[11;{}~" for F1.
fn pc_style(fmt: &str) -> Vec<Entry> {
    MODIFIERS
        .iter()
        .zip(2..)
        .map(|(mods, code)| Entry {
            mods: *mods,
            ..Entry::new(fmt.replace("{}", &code.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_table() {
        // All key sequences must fit into a termio array.
        // TODO(integration): termio's `Message.WriteReq.Small.Max` isn't
        // available here; use its documented limit (38 bytes).
        const MAX: usize = 38;
        let mut n = 0;
        for entries in all_keys() {
            for entry in entries {
                assert!(entry.sequence.len() <= MAX, "{:?}", entry.sequence);
                n += 1;
            }
        }
        assert!(n > 0);
    }
}
