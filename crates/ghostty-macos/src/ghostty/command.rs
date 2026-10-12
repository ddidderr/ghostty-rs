//! `Ghostty.Command` (`Ghostty.Command.swift`): `ghostty_command_s`.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char};

use super::capi::ghostty_command_s;

/// A command is fully self-contained (owned strings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The title of the command.
    pub title: String,

    /// Human-friendly description of what this command will do.
    pub description: String,

    /// The full action that must be performed to invoke this command.
    pub action: String,

    /// Only the key portion of the action so you can compare action types, e.g. `goto_split`
    /// instead of `goto_split:left`.
    pub action_key: String,
}

impl Command {
    /// Unsupported action keys, because they either don't make sense in the context of our
    /// target platform or they just aren't implemented yet.
    pub const UNSUPPORTED_ACTION_KEYS: [&'static str; 3] = [
        "toggle_tab_overview",
        "toggle_window_decorations",
        "show_gtk_inspector",
    ];

    /// True if this can be performed on this target.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        !Self::UNSUPPORTED_ACTION_KEYS.contains(&self.action_key.as_str())
    }

    /// `init(cValue:)`.
    ///
    /// # Safety
    /// All four pointers of `c` must be NUL terminated strings (or null,
    /// which gives an empty string).
    #[must_use]
    pub unsafe fn from_c(c: &ghostty_command_s) -> Self {
        Self {
            title: unsafe { cstr(c.title) },
            description: unsafe { cstr(c.description) },
            action: unsafe { cstr(c.action) },
            action_key: unsafe { cstr(c.action_key) },
        }
    }
}

/// `String(cString:)`.
unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: per the contract of `Command::from_c`.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_c_and_supported() {
        let c = ghostty_command_s {
            action_key: c"goto_split".as_ptr(),
            action: c"goto_split:left".as_ptr(),
            title: c"Focus Left".as_ptr(),
            description: c"desc".as_ptr(),
        };
        // SAFETY: static C strings.
        let cmd = unsafe { Command::from_c(&c) };
        assert_eq!(cmd.title, "Focus Left");
        assert_eq!(cmd.action, "goto_split:left");
        assert!(cmd.is_supported());
        let c2 = ghostty_command_s {
            action_key: c"toggle_tab_overview".as_ptr(),
            ..c
        };
        // SAFETY: as above.
        assert!(!unsafe { Command::from_c(&c2) }.is_supported());
    }
}
