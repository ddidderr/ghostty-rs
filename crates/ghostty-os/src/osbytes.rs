//! Byte-preserving conversions between OS strings and Rust `String`s.
//!
//! Zig treats paths, arguments and environment values as byte slices. Rust
//! `String` cannot hold invalid UTF-8, so the String based layers (the config
//! system) use a reversible encoding that is a true bijection on byte
//! strings:
//!
//! * the escape character is the Unicode noncharacter `U+FDD0` (never used
//!   in real text);
//! * every byte that is not part of a valid UTF-8 sequence (always
//!   `0x80..=0xFF`) is encoded as `U+FDD0` followed by the char `U+00bb`
//!   (`bb` = the byte), e.g. `0xFF` becomes `U+FDD0 U+00FF`;
//! * a literal `U+FDD0` in valid input is escaped as `U+FDD0 U+FDD0`.
//!
//! Hence every valid UTF-8 string without `U+FDD0` is encoded unchanged, and
//! `config_str_to_bytes(bytes_to_config_string(x)) == x` for all byte strings.

use std::ffi::{OsStr, OsString};

const ESC: char = '\u{FDD0}';

/// Encode arbitrary bytes into a `String`, escaping invalid UTF-8 bytes.
pub fn bytes_to_config_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            out.push(c);
            if c == ESC {
                out.push(ESC);
            }
        }
        for &b in chunk.invalid() {
            out.push(ESC);
            out.push(char::from(b));
        }
    }
    out
}

/// Inverse of [`bytes_to_config_string`].
pub fn config_str_to_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ESC {
            match chars.peek().copied() {
                Some(ESC) => {
                    chars.next();
                }
                Some(n) if (0x80..=0xFF).contains(&(n as u32)) => {
                    chars.next();
                    out.push(n as u32 as u8);
                    continue;
                }
                // Not produced by the encoder: keep the char as-is.
                _ => {}
            }
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    out
}

/// The raw bytes of an OS string (Unix; lossless). On other platforms the
/// string is converted via its UTF-8 (WTF-8) form.
#[cfg(unix)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// Build an OS string from raw bytes.
#[cfg(unix)]
pub fn os_from_bytes(b: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(b)
}

#[cfg(not(unix))]
pub fn os_from_bytes(b: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&b).into_owned())
}

/// OS string to reversible `String` (see the module docs).
pub fn os_to_config_string(s: &OsStr) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        bytes_to_config_string(s.as_bytes())
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned()
    }
}

/// Inverse of [`os_to_config_string`].
pub fn config_str_to_os(s: &str) -> OsString {
    os_from_bytes(config_str_to_bytes(s))
}

/// Path to reversible `String` (see the module docs).
pub fn path_to_config_string(p: &std::path::Path) -> String {
    os_to_config_string(p.as_os_str())
}

/// Inverse of [`path_to_config_string`].
pub fn config_str_to_path(s: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(config_str_to_os(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_invalid_bytes() {
        let raw = vec![b'a', 0xff, b'b', 0xc3, 0xa9, 0xc3];
        let s = bytes_to_config_string(&raw);
        assert_eq!(s, "a\u{fdd0}\u{ff}b\u{e9}\u{fdd0}\u{c3}");
        assert_eq!(config_str_to_bytes(&s), raw);
    }

    #[test]
    fn valid_utf8_unchanged() {
        let s = "h\u{e9}llo \u{1f600}";
        assert_eq!(bytes_to_config_string(s.as_bytes()), s);
        assert_eq!(config_str_to_bytes(s), s.as_bytes());
        // Nerd Font icons, including the old escape range, are untouched.
        let icons = "\u{f500}\u{f780}\u{f7ff}\u{fd46}\u{e0b0}";
        assert_eq!(bytes_to_config_string(icons.as_bytes()), icons);
        assert_eq!(config_str_to_bytes(icons), icons.as_bytes());
    }

    #[test]
    fn literal_escape_char_is_doubled() {
        let s = "a\u{fdd0}\u{ff}";
        let e = bytes_to_config_string(s.as_bytes());
        assert_eq!(e, "a\u{fdd0}\u{fdd0}\u{ff}");
        assert_eq!(config_str_to_bytes(&e), s.as_bytes());
    }

    /// Tiny xorshift generator (no external rand dependency).
    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn property_bytes_round_trip() {
        // Fragments that stress the boundaries: icon ranges, the escape
        // char, its doubled form, and truncated/invalid sequences.
        let frags: [&[u8]; 12] = [
            "\u{f780}".as_bytes(),
            "\u{f7ff}".as_bytes(),
            "\u{f500}".as_bytes(),
            "\u{fd46}".as_bytes(),
            "\u{fdd0}".as_bytes(),
            "\u{fdd0}\u{fdd0}".as_bytes(),
            "\u{fdd0}\u{ff}".as_bytes(),
            "\u{1f600}".as_bytes(),
            b"\xef\xb7",
            b"\xff",
            b"\xc3",
            b"abc",
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..5000 {
            let mut x = Vec::new();
            for _ in 0..(rng(&mut seed) % 12) {
                match rng(&mut seed) % 3 {
                    0 => x.push(rng(&mut seed) as u8),
                    _ => x.extend_from_slice(frags[(rng(&mut seed) % 12) as usize]),
                }
            }
            let enc = bytes_to_config_string(&x);
            assert_eq!(config_str_to_bytes(&enc), x, "bytes {x:?}");
            if let Ok(valid) = std::str::from_utf8(&x)
                && !valid.contains('\u{fdd0}')
            {
                assert_eq!(enc, valid);
            }
        }
    }

    #[test]
    fn property_valid_strings_round_trip() {
        // Every string (valid UTF-8) decodes back to itself after encoding.
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        for _ in 0..5000 {
            let mut s = String::new();
            for _ in 0..(rng(&mut seed) % 10) {
                let cp = match rng(&mut seed) % 4 {
                    0 => 0xF500 + (rng(&mut seed) % 0x847) as u32,
                    1 => 0xFDD0,
                    2 => 0x80 + (rng(&mut seed) % 0x80) as u32,
                    _ => 0x20 + (rng(&mut seed) % 0x5f) as u32,
                };
                s.push(char::from_u32(cp).unwrap());
            }
            let enc = bytes_to_config_string(s.as_bytes());
            assert_eq!(config_str_to_bytes(&enc), s.as_bytes(), "{s:?}");
            if !s.contains('\u{fdd0}') {
                assert_eq!(enc, s);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn os_roundtrip() {
        let o = os_from_bytes(vec![0xff, b'/', 0x80]);
        let s = os_to_config_string(&o);
        assert_eq!(config_str_to_os(&s), o);
    }
}
