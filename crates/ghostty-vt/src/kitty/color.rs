//! Kitty color protocol (ZIG/src/terminal/kitty/color.zig).

use std::fmt;

use crate::{color::Rgb, osc::Terminator};

/// OSC 21 payload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OSC {
    /// list of requests
    pub list: Vec<Request>,

    /// We must reply with the same string terminator (ST) as used in the
    /// request.
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Query(Kind),
    Set { key: Kind, color: Rgb },
    Reset(Kind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Special {
    Foreground,
    Background,
    SelectionForeground,
    SelectionBackground,
    Cursor,
    CursorText,
    VisualBell,
    SecondTransparentBackground,
}

impl Special {
    /// The tag name (Zig `@tagName`).
    pub fn name(self) -> &'static str {
        match self {
            Special::Foreground => "foreground",
            Special::Background => "background",
            Special::SelectionForeground => "selection_foreground",
            Special::SelectionBackground => "selection_background",
            Special::Cursor => "cursor",
            Special::CursorText => "cursor_text",
            Special::VisualBell => "visual_bell",
            Special::SecondTransparentBackground => "second_transparent_background",
        }
    }

    /// Zig `std.meta.stringToEnum`.
    pub fn from_name(s: &[u8]) -> Option<Special> {
        Some(match s {
            b"foreground" => Special::Foreground,
            b"background" => Special::Background,
            b"selection_foreground" => Special::SelectionForeground,
            b"selection_background" => Special::SelectionBackground,
            b"cursor" => Special::Cursor,
            b"cursor_text" => Special::CursorText,
            b"visual_bell" => Special::VisualBell,
            b"second_transparent_background" => Special::SecondTransparentBackground,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Palette(u8),
    Special(Special),
}

impl Kind {
    /// Number of distinct kinds (255 + number of specials).
    pub const MAX: usize = u8::MAX as usize + 8;

    pub fn parse(key: &[u8]) -> Option<Kind> {
        if let Some(s) = Special::from_name(key) {
            return Some(Kind::Special(s));
        }
        // Zig parseUnsigned(u8, key, 10): optional '+', digits only.
        let digits = match key.first()? {
            b'+' => &key[1..],
            _ => key,
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let s = std::str::from_utf8(digits).ok()?;
        s.parse::<u8>().ok().map(Kind::Palette)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Palette(p) => write!(f, "{p}"),
            Kind::Special(s) => f.write_str(s.name()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn osc_kitty_color_protocol_kind_string() {
        assert_eq!("foreground", Kind::Special(Special::Foreground).to_string());
        assert_eq!("42", Kind::Palette(42).to_string());
    }
}
