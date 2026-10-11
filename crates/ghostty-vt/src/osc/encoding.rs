//! Specialized encodings used in some OSC protocols
//! (ZIG/src/terminal/osc/encoding.zig).

/// Kitty defines "Escape code safe UTF-8" as valid UTF-8 with the
/// additional requirement of not containing any C0 escape codes
/// (0x00-0x1f), DEL (0x7f) and C1 escape codes (0x80-0x9f).
///
/// Used by OSC 66 (text sizing) and OSC 99 (Kitty notifications).
///
/// See: https://sw.kovidgoyal.net/kitty/desktop-notifications/#safe-utf8
pub fn is_safe_utf8(s: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(s) else {
        return false;
    };
    !s.chars()
        .any(|c| matches!(c as u32, 0x00..=0x1f | 0x7f | 0x80..=0x9f))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn is_safe_utf8_cases() {
        assert!(is_safe_utf8(b"Hello world!"));
        assert!(is_safe_utf8("安全的ユニコード☀️".as_bytes()));
        assert!(!is_safe_utf8(b"No linebreaks\nallowed"));
        assert!(!is_safe_utf8(b"\x07no bells"));
        assert!(!is_safe_utf8(b"\x1b]9;no OSCs\x1b\\\x1b[m"));
        assert!(!is_safe_utf8(b"\x9f8-bit escapes are clever, but no"));
    }
}
