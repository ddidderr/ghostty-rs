//! `Ghostty.Action` (`Ghostty.Action.swift`): Swift-side value types for the
//! payloads of `ghostty_action_s`.
#![allow(unsafe_code, clippy::wildcard_imports)]

use std::ffi::{CStr, c_int};

use super::capi::{
    consts::*,
    ghostty_action_color_change_s,
    ghostty_action_key_table_s,
    ghostty_action_move_tab_s,
    ghostty_action_open_url_s,
    ghostty_action_progress_report_s,
    ghostty_action_scrollbar_s,
    ghostty_action_start_search_s,
};
use crate::helpers::color::Color;

/// `Ghostty.Action.ColorChange`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorChange {
    pub kind: ColorChangeKind,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChangeKind {
    Foreground,
    Background,
    Cursor,
    Palette { index: u8 },
}

impl ColorChange {
    #[must_use]
    pub fn from_c(c: &ghostty_action_color_change_s) -> Self {
        let kind = match c.kind {
            GHOSTTY_ACTION_COLOR_KIND_FOREGROUND => ColorChangeKind::Foreground,
            GHOSTTY_ACTION_COLOR_KIND_BACKGROUND => ColorChangeKind::Background,
            GHOSTTY_ACTION_COLOR_KIND_CURSOR => ColorChangeKind::Cursor,
            other => ColorChangeKind::Palette { index: other as u8 },
        };
        Self {
            kind,
            color: Color::from_rgb8(c.r, c.g, c.b),
        }
    }
}

/// `Ghostty.Action.MoveTab`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveTab {
    pub amount: isize,
}

impl MoveTab {
    #[must_use]
    pub fn from_c(c: &ghostty_action_move_tab_s) -> Self {
        Self { amount: c.amount }
    }
}

/// `Ghostty.Action.OpenURL`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenUrl {
    pub kind: OpenUrlKind,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenUrlKind {
    Unknown,
    Text,
    Html,
}

impl OpenUrlKind {
    #[must_use]
    pub fn from_c(c: c_int) -> Self {
        match c {
            GHOSTTY_ACTION_OPEN_URL_KIND_TEXT => Self::Text,
            GHOSTTY_ACTION_OPEN_URL_KIND_HTML => Self::Html,
            _ => Self::Unknown,
        }
    }
}

impl OpenUrl {
    /// `init(c:)`.
    ///
    /// # Safety
    /// A non-null `c.url` must point to `c.len` readable bytes.
    #[must_use]
    pub unsafe fn from_c(c: &ghostty_action_open_url_s) -> Self {
        let url = if c.url.is_null() {
            String::new()
        } else {
            // SAFETY: per the contract.
            let data = unsafe { std::slice::from_raw_parts(c.url.cast::<u8>(), c.len) };
            String::from_utf8(data.to_vec()).unwrap_or_default()
        };
        Self {
            kind: OpenUrlKind::from_c(c.kind),
            url,
        }
    }
}

/// `Ghostty.Action.ProgressReport`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressReport {
    pub state: ProgressReportState,
    pub progress: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressReportState {
    Remove,
    Set,
    Error,
    Indeterminate,
    Pause,
}

impl ProgressReportState {
    #[must_use]
    pub fn from_c(c: c_int) -> Self {
        match c {
            GHOSTTY_PROGRESS_STATE_SET => Self::Set,
            GHOSTTY_PROGRESS_STATE_ERROR => Self::Error,
            GHOSTTY_PROGRESS_STATE_INDETERMINATE => Self::Indeterminate,
            GHOSTTY_PROGRESS_STATE_PAUSE => Self::Pause,
            // GHOSTTY_PROGRESS_STATE_REMOVE and anything unknown.
            _ => Self::Remove,
        }
    }
}

impl ProgressReport {
    #[must_use]
    pub fn from_c(c: &ghostty_action_progress_report_s) -> Self {
        Self {
            state: ProgressReportState::from_c(c.state),
            progress: u8::try_from(c.progress).ok(),
        }
    }
}

/// `Ghostty.Action.Scrollbar`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

impl Scrollbar {
    #[must_use]
    pub fn from_c(c: &ghostty_action_scrollbar_s) -> Self {
        Self {
            total: c.total,
            offset: c.offset,
            len: c.len,
        }
    }
}

/// `Ghostty.Action.StartSearch`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartSearch {
    pub needle: Option<String>,
}

impl StartSearch {
    /// `init(c:)`.
    ///
    /// # Safety
    /// A non-null `c.needle` must be NUL terminated.
    #[must_use]
    pub unsafe fn from_c(c: &ghostty_action_start_search_s) -> Self {
        let needle = if c.needle.is_null() {
            None
        } else {
            // SAFETY: per the contract.
            Some(
                unsafe { CStr::from_ptr(c.needle) }
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        Self { needle }
    }
}

/// `Ghostty.Action.PromptTitle`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptTitle {
    Surface,
    Tab,
}

impl PromptTitle {
    #[must_use]
    pub fn from_c(c: c_int) -> Self {
        if c == GHOSTTY_PROMPT_TITLE_TAB {
            Self::Tab
        } else {
            Self::Surface
        }
    }
}

/// `Ghostty.Action.KeyTable`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyTable {
    Activate { name: String },
    Deactivate,
    DeactivateAll,
}

impl KeyTable {
    /// `init?(c:)`.
    ///
    /// # Safety
    /// For the activate tag, `c.value.activate.name` must point to `len` bytes.
    #[must_use]
    pub unsafe fn from_c(c: &ghostty_action_key_table_s) -> Option<Self> {
        match c.tag {
            GHOSTTY_KEY_TABLE_ACTIVATE => {
                // SAFETY: the tag selects the `activate` member; pointer per the contract.
                let (ptr, len) = unsafe { (c.value.activate.name, c.value.activate.len) };
                let name = if ptr.is_null() {
                    String::new()
                } else {
                    // SAFETY: per the contract.
                    let data = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
                    String::from_utf8(data.to_vec()).unwrap_or_default()
                };
                Some(Self::Activate { name })
            }
            GHOSTTY_KEY_TABLE_DEACTIVATE => Some(Self::Deactivate),
            GHOSTTY_KEY_TABLE_DEACTIVATE_ALL => Some(Self::DeactivateAll),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::capi::{ghostty_action_key_table_activate_s, ghostty_action_key_table_u};

    #[test]
    fn color_change_kinds() {
        let c = |kind| ghostty_action_color_change_s {
            kind,
            r: 255,
            g: 0,
            b: 51,
        };
        assert_eq!(
            ColorChange::from_c(&c(GHOSTTY_ACTION_COLOR_KIND_FOREGROUND)).kind,
            ColorChangeKind::Foreground
        );
        assert_eq!(
            ColorChange::from_c(&c(GHOSTTY_ACTION_COLOR_KIND_CURSOR)).kind,
            ColorChangeKind::Cursor
        );
        let p = ColorChange::from_c(&c(7));
        assert_eq!(p.kind, ColorChangeKind::Palette { index: 7 });
        assert_eq!(p.color, Color::new(1.0, 0.0, 0.2));
    }

    #[test]
    fn progress_report() {
        let r = ProgressReport::from_c(&ghostty_action_progress_report_s {
            state: GHOSTTY_PROGRESS_STATE_SET,
            progress: 40,
        });
        assert_eq!(
            r,
            ProgressReport {
                state: ProgressReportState::Set,
                progress: Some(40)
            }
        );
        let r = ProgressReport::from_c(&ghostty_action_progress_report_s {
            state: GHOSTTY_PROGRESS_STATE_INDETERMINATE,
            progress: -1,
        });
        assert_eq!(r.progress, None);
        assert_eq!(ProgressReportState::from_c(99), ProgressReportState::Remove);
    }

    #[test]
    fn open_url_and_search() {
        let url = "file:///tmp/a";
        let c = ghostty_action_open_url_s {
            kind: GHOSTTY_ACTION_OPEN_URL_KIND_TEXT,
            url: url.as_ptr().cast(),
            len: url.len(),
        };
        // SAFETY: `url` outlives the call.
        let o = unsafe { OpenUrl::from_c(&c) };
        assert_eq!(
            o,
            OpenUrl {
                kind: OpenUrlKind::Text,
                url: url.into()
            }
        );
        let s = ghostty_action_start_search_s {
            needle: c"abc".as_ptr(),
        };
        // SAFETY: static C string.
        assert_eq!(
            unsafe { StartSearch::from_c(&s) }.needle.as_deref(),
            Some("abc")
        );
        let s = ghostty_action_start_search_s {
            needle: std::ptr::null(),
        };
        // SAFETY: null handled.
        assert_eq!(unsafe { StartSearch::from_c(&s) }.needle, None);
    }

    #[test]
    fn key_table() {
        let name = "vim";
        let c = ghostty_action_key_table_s {
            tag: GHOSTTY_KEY_TABLE_ACTIVATE,
            value: ghostty_action_key_table_u {
                activate: ghostty_action_key_table_activate_s {
                    name: name.as_ptr().cast(),
                    len: 3,
                },
            },
        };
        // SAFETY: `name` outlives the call.
        assert_eq!(
            unsafe { KeyTable::from_c(&c) },
            Some(KeyTable::Activate { name: "vim".into() })
        );
        let c = ghostty_action_key_table_s {
            tag: GHOSTTY_KEY_TABLE_DEACTIVATE_ALL,
            ..c
        };
        // SAFETY: no pointer is read.
        assert_eq!(
            unsafe { KeyTable::from_c(&c) },
            Some(KeyTable::DeactivateAll)
        );
        assert_eq!(
            PromptTitle::from_c(GHOSTTY_PROMPT_TITLE_TAB),
            PromptTitle::Tab
        );
    }
}
