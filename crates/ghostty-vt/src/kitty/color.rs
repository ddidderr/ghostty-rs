//! Kitty color protocol (ZIG/src/terminal/kitty/color.zig).

use std::{borrow::Cow, fmt};

use crate::{color::Rgb, osc::Terminator};

/// OSC 21 payload.
///
/// The request list is stored encoded (fixed size records) in the OSC
/// parser's own storage, so parsing it never touches the global allocator
/// when the parser was given a caller-provided allocator (C API).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OSC<'a> {
    /// list of requests
    pub list: List<'a>,

    /// We must reply with the same string terminator (ST) as used in the
    /// request.
    pub terminator: Terminator,
}

impl OSC<'_> {
    /// Detach from the parser storage.
    #[must_use]
    pub fn into_owned(self) -> OSC<'static> {
        OSC {
            list: self.list.into_owned(),
            terminator: self.terminator,
        }
    }
}

/// Size of one encoded [`Request`] record.
pub const RECORD_LEN: usize = 8;

/// A list of kitty color [`Request`]s: borrowed (or owned) encoded records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct List<'a>(Cow<'a, [u8]>);

impl<'a> List<'a> {
    /// View records encoded with [`Request::encode`] (`bytes.len()` must be a
    /// multiple of [`RECORD_LEN`]).
    #[must_use]
    pub fn from_encoded(bytes: &'a [u8]) -> List<'a> {
        debug_assert_eq!(bytes.len() % RECORD_LEN, 0);
        List(Cow::Borrowed(bytes))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len() / RECORD_LEN
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn get(&self, i: usize) -> Option<Request> {
        Request::decode(self.0.get(i * RECORD_LEN..(i + 1) * RECORD_LEN)?)
    }

    /// Iterate the requests (by value).
    pub fn iter(&self) -> impl Iterator<Item = Request> + '_ {
        self.0
            .as_chunks::<RECORD_LEN>()
            .0
            .iter()
            .filter_map(|rec| Request::decode(rec))
    }

    /// Copy into a `Vec` (allocates; for tests and diagnostics).
    #[must_use]
    pub fn to_vec(&self) -> Vec<Request> {
        self.iter().collect()
    }

    #[must_use]
    pub fn into_owned(self) -> List<'static> {
        List(Cow::Owned(self.0.into_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// All specials in discriminant order.
    const ALL: [Special; 8] = [
        Special::Foreground,
        Special::Background,
        Special::SelectionForeground,
        Special::SelectionBackground,
        Special::Cursor,
        Special::CursorText,
        Special::VisualBell,
        Special::SecondTransparentBackground,
    ];

    fn to_u8(self) -> u8 {
        Special::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    fn from_u8(v: u8) -> Option<Special> {
        Special::ALL.get(usize::from(v)).copied()
    }

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
        // Zig parseUnsigned(u8, key, 10).
        ghostty_zigfmt::parse_unsigned::<u8>(key, 10)
            .ok()
            .map(Kind::Palette)
    }
}

impl Request {
    /// Encode as a fixed size record: `[tag, kind class, kind value, r, g, b,
    /// 0, 0]`.
    #[must_use]
    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let (tag, key, color) = match *self {
            Request::Query(k) => (0, k, Rgb::default()),
            Request::Set { key, color } => (1, key, color),
            Request::Reset(k) => (2, k, Rgb::default()),
        };
        let (class, value) = match key {
            Kind::Palette(p) => (0, p),
            Kind::Special(s) => (1, s.to_u8()),
        };
        [tag, class, value, color.r, color.g, color.b, 0, 0]
    }

    /// Inverse of [`Request::encode`].
    #[must_use]
    pub fn decode(rec: &[u8]) -> Option<Request> {
        let [tag, class, value, r, g, b, ..] = *rec else {
            return None;
        };
        let key = match class {
            0 => Kind::Palette(value),
            1 => Kind::Special(Special::from_u8(value)?),
            _ => return None,
        };
        Some(match tag {
            0 => Request::Query(key),
            1 => Request::Set {
                key,
                color: Rgb { r, g, b },
            },
            2 => Request::Reset(key),
            _ => return None,
        })
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
    fn zig_parse_unsigned_palette_key() {
        assert!(matches!(Kind::parse(b"1_0"), Some(Kind::Palette(10))));
        assert!(Kind::parse(b"+5").is_none());
        assert!(Kind::parse(b"-0").is_none());
        assert!(Kind::parse(b"0x10").is_none());
        assert!(Kind::parse(b"256").is_none());
        assert!(Kind::parse(b"1_").is_none());
    }

    #[test]
    fn request_records_round_trip() {
        let mut reqs = vec![];
        for s in Special::ALL {
            let k = Kind::Special(s);
            reqs.push(Request::Query(k));
            reqs.push(Request::Reset(k));
            reqs.push(Request::Set {
                key: k,
                color: Rgb { r: 1, g: 2, b: 3 },
            });
        }
        reqs.push(Request::Set {
            key: Kind::Palette(255),
            color: Rgb { r: 255, g: 0, b: 9 },
        });
        reqs.push(Request::Query(Kind::Palette(0)));
        let bytes: Vec<u8> = reqs.iter().flat_map(Request::encode).collect();
        let list = List::from_encoded(&bytes);
        assert_eq!(list.len(), reqs.len());
        assert_eq!(list.to_vec(), reqs);
        assert_eq!(list.get(reqs.len()), None);
        assert_eq!(list.into_owned().to_vec(), reqs);
        assert_eq!(Request::decode(&[9, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(Request::decode(&[0, 1, 200, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn osc_kitty_color_protocol_kind_string() {
        assert_eq!("foreground", Kind::Special(Special::Foreground).to_string());
        assert_eq!("42", Kind::Palette(42).to_string());
    }
}
