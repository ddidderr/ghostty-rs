//! String-literal escape parsing (port of `config/string.zig`).

/// Errors from string parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringError {
    InvalidString,
}

impl std::fmt::Display for StringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InvalidString")
    }
}

impl std::error::Error for StringError {}

impl From<StringError> for crate::cli_args::ArgError {
    fn from(e: StringError) -> Self {
        crate::cli_args::ArgError::Other(e.to_string())
    }
}

/// Parse a Zig escape sequence starting at `bytes[*i]` (which must be a
/// backslash). On success advances `*i` past the sequence and returns the
/// codepoint.
fn parse_escape_sequence(bytes: &[u8], i: &mut usize) -> Option<u32> {
    let mut idx = *i;
    debug_assert_eq!(bytes[idx], b'\\');
    idx += 1;
    let c = *bytes.get(idx)?;
    idx += 1;
    let cp = match c {
        b'n' => u32::from(b'\n'),
        b'r' => u32::from(b'\r'),
        b'\\' => u32::from(b'\\'),
        b't' => u32::from(b'\t'),
        b'\'' => u32::from(b'\''),
        b'"' => u32::from(b'"'),
        b'x' => {
            let h = bytes.get(idx..idx + 2)?;
            let h = std::str::from_utf8(h).ok()?;
            if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            idx += 2;
            u32::from_str_radix(h, 16).ok()?
        }
        b'u' => {
            if *bytes.get(idx)? != b'{' {
                return None;
            }
            idx += 1;
            let start = idx;
            let mut v: u32 = 0;
            loop {
                let b = *bytes.get(idx)?;
                idx += 1;
                if b == b'}' {
                    if idx - 1 == start {
                        return None;
                    }
                    break;
                }
                let d = (b as char).to_digit(16)?;
                v = v.checked_mul(16)?.checked_add(d)?;
                if v > 0x10ffff {
                    return None;
                }
            }
            v
        }
        _ => return None,
    };
    *i = idx;
    Some(cp)
}

/// Parse a string literal into the original bytes (Zig `parse`'s output).
/// `input` is a config string in the reversible OS-byte encoding; the result
/// is decoded, so it can be written to the pty / OS directly.
pub fn parse_bytes(input: &str) -> Result<Vec<u8>, StringError> {
    parse(input).map(|s| ghostty_os::osbytes::config_str_to_bytes(&s))
}

/// Parse a string literal into a string. The string can contain
/// any valid Zig string literal escape sequences.
///
/// The input and output are in the reversible OS-byte encoding (see
/// `ghostty_os::osbytes`): bytes are copied through unchanged, and a
/// code point produced by an escape is encoded as plain text, so a
/// literal `U+FDD0` from `\u{fdd0}` stays distinct from the escape
/// character. Use [`parse_bytes`] for the raw bytes.
pub fn parse(bytes: &str) -> Result<String, StringError> {
    let b = bytes.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            // Copy a whole UTF-8 character.
            let ch = bytes[i..]
                .chars()
                .next()
                .ok_or(StringError::InvalidString)?;
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let cp = parse_escape_sequence(b, &mut i).ok_or(StringError::InvalidString)?;
        // Surrogates cannot be encoded.
        let ch = char::from_u32(cp).ok_or(StringError::InvalidString)?;
        if ch == '\u{FDD0}' {
            out.push_str("\u{FDD0}\u{FDD0}");
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

/// Creates an iterator that requires no allocation to extract codepoints
/// from the string literal, parsing escape sequences as it goes.
#[must_use]
pub fn codepoint_iterator(bytes: &str) -> CodepointIterator<'_> {
    CodepointIterator { bytes, i: 0 }
}

pub struct CodepointIterator<'a> {
    bytes: &'a str,
    i: usize,
}

impl CodepointIterator<'_> {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<u32>, StringError> {
        if self.i >= self.bytes.len() {
            return Ok(None);
        }
        if self.bytes.as_bytes()[self.i] == b'\\' {
            // An escape sequence
            return parse_escape_sequence(self.bytes.as_bytes(), &mut self.i)
                .map(Some)
                .ok_or(StringError::InvalidString);
        }

        // Not an escape, parse as UTF-8
        let ch = self.bytes[self.i..]
            .chars()
            .next()
            .ok_or(StringError::InvalidString)?;
        self.i += ch.len_utf8();
        Ok(Some(u32::from(ch)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    fn parse_bytes_decodes_the_reversible_encoding() {
        use ghostty_os::osbytes::EncodedOs;

        use super::{parse, parse_bytes};

        // Raw non-UTF-8 bytes pass through as the same bytes.
        let enc = EncodedOs::from_bytes(b"a\xffb\\n").into_encoded();
        assert_eq!(parse_bytes(&enc).unwrap(), b"a\xffb\n");
        // A literal U+FDD0 (typed or via an escape) is that character.
        let lit = EncodedOs::from_plain_text("\u{fdd0}").into_encoded();
        assert_eq!(parse_bytes(&lit).unwrap(), "\u{fdd0}".as_bytes());
        assert_eq!(parse_bytes("\\u{fdd0}").unwrap(), "\u{fdd0}".as_bytes());
        // An escape producing U+FF is the UTF-8 encoding, not byte FF.
        assert_eq!(parse_bytes("\\xff").unwrap(), "\u{ff}".as_bytes());
        assert!(parse("\\u{d800}").is_err());
    }

    use super::*;

    #[test]
    fn parse_empty() {
        assert_eq!(parse("").unwrap(), "");
    }

    #[test]
    fn parse_no_escapes() {
        assert_eq!(parse("hello world").unwrap(), "hello world");
    }

    #[test]
    fn parse_escapes() {
        assert_eq!(parse("hello\\nworld").unwrap(), "hello\nworld");
        assert_eq!(
            parse("hello\\u{1F601}world").unwrap(),
            "hello\u{1F601}world"
        );
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse("\\q"), Err(StringError::InvalidString));
        assert_eq!(parse("\\"), Err(StringError::InvalidString));
        assert_eq!(parse("\\u{}"), Err(StringError::InvalidString));
        assert_eq!(parse("\\u{110000}"), Err(StringError::InvalidString));
        assert_eq!(parse("\\x4"), Err(StringError::InvalidString));
    }

    #[test]
    fn codepoint_iterator_empty() {
        let mut it = codepoint_iterator("");
        assert_eq!(it.next().unwrap(), None);
    }

    #[test]
    fn codepoint_iterator_ascii_no_escapes() {
        let mut it = codepoint_iterator("abc");
        assert_eq!(it.next().unwrap(), Some('a' as u32));
        assert_eq!(it.next().unwrap(), Some('b' as u32));
        assert_eq!(it.next().unwrap(), Some('c' as u32));
        assert_eq!(it.next().unwrap(), None);
    }

    #[test]
    fn codepoint_iterator_multibyte_utf8() {
        // │ is U+2502 (3 bytes in UTF-8)
        let mut it = codepoint_iterator("a│b");
        assert_eq!(it.next().unwrap(), Some('a' as u32));
        assert_eq!(it.next().unwrap(), Some('│' as u32));
        assert_eq!(it.next().unwrap(), Some('b' as u32));
        assert_eq!(it.next().unwrap(), None);
    }

    #[test]
    fn codepoint_iterator_escape_sequences() {
        let mut it = codepoint_iterator("a\\tb\\n\\\\");
        assert_eq!(it.next().unwrap(), Some('a' as u32));
        assert_eq!(it.next().unwrap(), Some('\t' as u32));
        assert_eq!(it.next().unwrap(), Some('b' as u32));
        assert_eq!(it.next().unwrap(), Some('\n' as u32));
        assert_eq!(it.next().unwrap(), Some('\\' as u32));
        assert_eq!(it.next().unwrap(), None);
    }

    #[test]
    fn codepoint_iterator_unicode_escape() {
        let mut it = codepoint_iterator("\\u{2502}x");
        assert_eq!(it.next().unwrap(), Some('│' as u32));
        assert_eq!(it.next().unwrap(), Some('x' as u32));
        assert_eq!(it.next().unwrap(), None);
    }

    #[test]
    fn codepoint_iterator_emoji_unicode_escape() {
        let mut it = codepoint_iterator("\\u{1F601}");
        assert_eq!(it.next().unwrap(), Some(0x1F601));
        assert_eq!(it.next().unwrap(), None);
    }
}
