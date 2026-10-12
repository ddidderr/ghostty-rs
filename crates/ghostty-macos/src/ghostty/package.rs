//! `GhosttyPackage.swift` (minus the notification names, which are in
//! [`super::notification`]) and `GhosttyPackageMeta.swift`.
#![allow(unsafe_code, clippy::wildcard_imports)]
#![cfg_attr(test, allow(clippy::assert_is_empty))]

use std::ffi::{CStr, c_int};

use super::capi::{
    consts::*,
    ghostty_clipboard_content_s,
    ghostty_info,
    ghostty_string_free,
    ghostty_string_s,
};

// MARK: C Extensions

/// The user notification category identifier
pub const USER_NOTIFICATION_CATEGORY: &str = "com.mitchellh.ghostty.userNotification";

/// The user notification "Show" action
pub const USER_NOTIFICATION_ACTION_SHOW: &str = "com.mitchellh.ghostty.userNotification.Show";

// MARK: Build Info

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// `ghostty_build_mode_e`
    pub mode: c_int,
    pub version: String,
}

/// `Ghostty.info`.
#[must_use]
pub fn info() -> Info {
    let raw = ghostty_info();
    let version = if raw.version.is_null() {
        // NSString(bytes:...) of nothing falls back to "unknown".
        "unknown".to_string()
    } else {
        // SAFETY: `version` points to `version_len` bytes that live for the process.
        let bytes =
            unsafe { std::slice::from_raw_parts(raw.version.cast::<u8>(), raw.version_len) };
        std::str::from_utf8(bytes).map_or_else(|_| "unknown".to_string(), str::to_string)
    };
    Info {
        mode: raw.build_mode,
        version,
    }
}

// MARK: General Helpers

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchSource {
    Cli,
    App,
    ZigRun,
}

impl LaunchSource {
    /// `init?(rawValue:)`.
    #[must_use]
    pub fn from_raw_value(raw: &str) -> Option<Self> {
        match raw {
            "cli" => Some(Self::Cli),
            "app" => Some(Self::App),
            "zig_run" => Some(Self::ZigRun),
            _ => None,
        }
    }

    /// Pure form of [`launch_source`]: `env` is the value of
    /// `GHOSTTY_MAC_LAUNCH_SOURCE`.
    #[must_use]
    pub fn from_env_value(env: Option<&str>) -> Self {
        // We default to the CLI because the app bundle always sets the
        // source. If its unset we assume we're in a CLI environment.
        let Some(value) = env else {
            return Self::Cli;
        };

        // If the env var is set but its unknown then we default back to the app.
        Self::from_raw_value(value).unwrap_or(Self::App)
    }
}

/// Returns the mechanism that launched the app. This is based on an env var so
/// its up to the env var being set in the correct circumstance.
#[must_use]
pub fn launch_source() -> LaunchSource {
    LaunchSource::from_env_value(std::env::var("GHOSTTY_MAC_LAUNCH_SOURCE").ok().as_deref())
}

// MARK: Swift Types for C Types

/// A `ghostty_string_s` that is freed with `ghostty_string_free` when dropped.
pub struct AllocatedString {
    c_string: ghostty_string_s,
}

impl AllocatedString {
    #[must_use]
    pub fn new(c: ghostty_string_s) -> Self {
        Self { c_string: c }
    }

    /// The contents; empty if null or not valid UTF-8.
    #[must_use]
    pub fn string(&self) -> String {
        if self.c_string.ptr.is_null() {
            return String::new();
        }
        // SAFETY: `ptr`/`len` describe the allocation owned by this value.
        let bytes = unsafe {
            std::slice::from_raw_parts(self.c_string.ptr.cast::<u8>(), self.c_string.len)
        };
        String::from_utf8(bytes.to_vec()).unwrap_or_default()
    }
}

impl Drop for AllocatedString {
    fn drop(&mut self) {
        ghostty_string_free(self.c_string);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetFloatWindow {
    On,
    Off,
    Toggle,
}

impl SetFloatWindow {
    /// `from(_ c: ghostty_action_float_window_e)`.
    #[must_use]
    pub fn from_c(c: c_int) -> Option<Self> {
        match c {
            GHOSTTY_FLOAT_WINDOW_ON => Some(Self::On),
            GHOSTTY_FLOAT_WINDOW_OFF => Some(Self::Off),
            GHOSTTY_FLOAT_WINDOW_TOGGLE => Some(Self::Toggle),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetSecureInput {
    On,
    Off,
    Toggle,
}

impl SetSecureInput {
    /// `from(_ c: ghostty_action_secure_input_e)`.
    #[must_use]
    pub fn from_c(c: c_int) -> Option<Self> {
        match c {
            GHOSTTY_SECURE_INPUT_ON => Some(Self::On),
            GHOSTTY_SECURE_INPUT_OFF => Some(Self::Off),
            GHOSTTY_SECURE_INPUT_TOGGLE => Some(Self::Toggle),
            _ => None,
        }
    }
}

/// An enum that is used for the directions that a split focus event can change.
///
/// TODO(MA3): `toSplitTreeFocusDirection` (`SplitTree<ViewType>.FocusDirection`)
/// lives with the SplitTree port in `features::splits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitFocusDirection {
    Previous,
    Next,
    Up,
    Down,
    Left,
    Right,
}

impl SplitFocusDirection {
    /// Initialize from a Ghostty API enum (`ghostty_action_goto_split_e`).
    #[must_use]
    pub fn from_direction(direction: c_int) -> Option<Self> {
        match direction {
            GHOSTTY_GOTO_SPLIT_PREVIOUS => Some(Self::Previous),
            GHOSTTY_GOTO_SPLIT_NEXT => Some(Self::Next),
            GHOSTTY_GOTO_SPLIT_UP => Some(Self::Up),
            GHOSTTY_GOTO_SPLIT_DOWN => Some(Self::Down),
            GHOSTTY_GOTO_SPLIT_LEFT => Some(Self::Left),
            GHOSTTY_GOTO_SPLIT_RIGHT => Some(Self::Right),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_native(self) -> c_int {
        match self {
            Self::Previous => GHOSTTY_GOTO_SPLIT_PREVIOUS,
            Self::Next => GHOSTTY_GOTO_SPLIT_NEXT,
            Self::Up => GHOSTTY_GOTO_SPLIT_UP,
            Self::Down => GHOSTTY_GOTO_SPLIT_DOWN,
            Self::Left => GHOSTTY_GOTO_SPLIT_LEFT,
            Self::Right => GHOSTTY_GOTO_SPLIT_RIGHT,
        }
    }
}

/// Enum used for resizing splits. This is the direction the split divider will move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitResizeDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SplitResizeDirection {
    /// `from(direction: ghostty_action_resize_split_direction_e)`.
    #[must_use]
    pub fn from_direction(direction: c_int) -> Option<Self> {
        match direction {
            GHOSTTY_RESIZE_SPLIT_UP => Some(Self::Up),
            GHOSTTY_RESIZE_SPLIT_DOWN => Some(Self::Down),
            GHOSTTY_RESIZE_SPLIT_LEFT => Some(Self::Left),
            GHOSTTY_RESIZE_SPLIT_RIGHT => Some(Self::Right),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_native(self) -> c_int {
        match self {
            Self::Up => GHOSTTY_RESIZE_SPLIT_UP,
            Self::Down => GHOSTTY_RESIZE_SPLIT_DOWN,
            Self::Left => GHOSTTY_RESIZE_SPLIT_LEFT,
            Self::Right => GHOSTTY_RESIZE_SPLIT_RIGHT,
        }
    }
}

/// Identity of an `NSPasteboard` (its address). Swift's
/// `osc_52_write(OSPasteboard?)` holds the pasteboard object; Ghostty only ever
/// uses the two process-lifetime pasteboards (`NSPasteboard.general` and
/// `NSPasteboard.ghosttySelection`), so the identity is enough here and the
/// AppKit side turns it back into the object. Not dereferenced in this module.
pub type PasteboardHandle = usize;

/// The type of a clipboard request
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    /// A direct paste of clipboard contents
    Paste,

    /// An application is attempting to read from the clipboard using OSC 52
    Osc52Read,

    /// An application is attempting to write to the clipboard using OSC 52
    Osc52Write(Option<PasteboardHandle>),
}

impl ClipboardRequest {
    /// The text to show in the clipboard confirmation prompt for a given request type
    #[must_use]
    pub const fn text(&self) -> &'static str {
        match self {
            Self::Paste => {
                "Pasting this text to the terminal may be dangerous as it looks like some commands may be executed."
            }
            Self::Osc52Read => {
                "An application is attempting to read from the clipboard.\nThe current clipboard contents are shown below."
            }
            Self::Osc52Write(_) => {
                "An application is attempting to write to the clipboard.\nThe content to write is shown below."
            }
        }
    }

    /// `from(request: ghostty_clipboard_request_e)`.
    #[must_use]
    pub fn from_request(request: c_int) -> Option<Self> {
        match request {
            GHOSTTY_CLIPBOARD_REQUEST_PASTE => Some(Self::Paste),
            GHOSTTY_CLIPBOARD_REQUEST_OSC_52_READ => Some(Self::Osc52Read),
            GHOSTTY_CLIPBOARD_REQUEST_OSC_52_WRITE => Some(Self::Osc52Write(None)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardContent {
    pub mime: String,
    pub data: String,
}

impl ClipboardContent {
    /// `from(content: ghostty_clipboard_content_s)`.
    ///
    /// # Safety
    /// Non-null `mime` / `data` must be NUL terminated strings.
    #[must_use]
    pub unsafe fn from_c(content: &ghostty_clipboard_content_s) -> Option<Self> {
        if content.mime.is_null() || content.data.is_null() {
            return None;
        }
        // SAFETY: non-null and NUL terminated per the contract.
        let (mime, data) = unsafe { (CStr::from_ptr(content.mime), CStr::from_ptr(content.data)) };
        Some(Self {
            mime: mime.to_string_lossy().into_owned(),
            data: data.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_source_defaults() {
        assert_eq!(LaunchSource::from_env_value(None), LaunchSource::Cli);
        assert_eq!(LaunchSource::from_env_value(Some("app")), LaunchSource::App);
        assert_eq!(
            LaunchSource::from_env_value(Some("zig_run")),
            LaunchSource::ZigRun
        );
        assert_eq!(LaunchSource::from_env_value(Some("???")), LaunchSource::App);
    }

    #[test]
    fn split_directions_round_trip() {
        for d in [
            SplitFocusDirection::Previous,
            SplitFocusDirection::Next,
            SplitFocusDirection::Up,
            SplitFocusDirection::Down,
            SplitFocusDirection::Left,
            SplitFocusDirection::Right,
        ] {
            assert_eq!(SplitFocusDirection::from_direction(d.to_native()), Some(d));
        }
        assert_eq!(SplitFocusDirection::from_direction(99), None);
        for d in [
            SplitResizeDirection::Up,
            SplitResizeDirection::Down,
            SplitResizeDirection::Left,
            SplitResizeDirection::Right,
        ] {
            assert_eq!(SplitResizeDirection::from_direction(d.to_native()), Some(d));
        }
    }

    #[test]
    fn clipboard_request_from_c() {
        assert_eq!(
            ClipboardRequest::from_request(GHOSTTY_CLIPBOARD_REQUEST_OSC_52_WRITE),
            Some(ClipboardRequest::Osc52Write(None))
        );
        assert_eq!(ClipboardRequest::from_request(42), None);
        assert!(ClipboardRequest::Paste.text().contains("dangerous"));
    }

    #[test]
    fn clipboard_content() {
        let c = ghostty_clipboard_content_s {
            mime: c"text/plain".as_ptr(),
            data: c"hi".as_ptr(),
        };
        // SAFETY: static C strings.
        let content = unsafe { ClipboardContent::from_c(&c) };
        assert_eq!(
            content,
            Some(ClipboardContent {
                mime: "text/plain".into(),
                data: "hi".into()
            })
        );
        let n = ghostty_clipboard_content_s {
            mime: std::ptr::null(),
            data: c"hi".as_ptr(),
        };
        // SAFETY: null is handled.
        assert!(unsafe { ClipboardContent::from_c(&n) }.is_none());
    }

    #[test]
    fn info_has_version() {
        assert!(!info().version.is_empty());
    }
}
