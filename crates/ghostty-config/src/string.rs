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

/// Parse literal escapes from the original bytes, copying unescaped bytes verbatim.
/// Like Zig, \xFF denotes U+00FF; only unescaped bytes bypass UTF-8 validation.
pub fn parse_bytes(input: impl AsRef<[u8]>) -> Result<Vec<u8>, StringError> {
    let bytes = input.as_ref();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let cp = parse_escape_sequence(bytes, &mut i).ok_or(StringError::InvalidString)?;
        let ch = char::from_u32(cp).ok_or(StringError::InvalidString)?;
        out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
    }
    Ok(out)
}

/// Plain-text convenience; byte consumers use [`parse_bytes`].
pub fn parse(input: &str) -> Result<String, StringError> {
    String::from_utf8(parse_bytes(input)?).map_err(|_| StringError::InvalidString)
}

/// Creates an iterator that requires no allocation to extract codepoints
/// from the string literal, parsing escape sequences as it goes.
#[must_use]
pub fn codepoint_iterator(bytes: &str) -> CodepointIterator<'_> {
    codepoint_iterator_bytes(bytes.as_bytes())
}

pub fn codepoint_iterator_bytes(bytes: &[u8]) -> CodepointIterator<'_> {
    CodepointIterator { bytes, i: 0 }
}

pub struct CodepointIterator<'a> {
    bytes: &'a [u8],
    i: usize,
}

impl CodepointIterator<'_> {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<u32>, StringError> {
        if self.i >= self.bytes.len() {
            return Ok(None);
        }
        if self.bytes[self.i] == b'\\' {
            // An escape sequence
            return parse_escape_sequence(self.bytes, &mut self.i)
                .map(Some)
                .ok_or(StringError::InvalidString);
        }

        // Not an escape, parse as UTF-8
        let len = match self.bytes[self.i] {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(StringError::InvalidString),
        };
        let raw = self
            .bytes
            .get(self.i..self.i + len)
            .ok_or(StringError::InvalidString)?;
        let ch = std::str::from_utf8(raw)
            .map_err(|_| StringError::InvalidString)?
            .chars()
            .next()
            .ok_or(StringError::InvalidString)?;
        self.i += len;
        Ok(Some(u32::from(ch)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    fn parse_bytes_copies_original_bytes() {
        use ghostty_os::osbytes::EncodedOs;

        use super::{parse, parse_bytes};

        // Raw non-UTF-8 bytes pass through as the same bytes.
        let enc = EncodedOs::from_bytes(b"a\xffb\\n").into_encoded();
        assert_eq!(
            parse_bytes(EncodedOs::from_encoded(enc).to_bytes()).unwrap(),
            b"a\xffb\n"
        );
        // A literal U+FDD0 (typed or via an escape) is that character.
        let lit = EncodedOs::from_plain_text("\u{fdd0}").into_encoded();
        assert_eq!(
            parse_bytes(EncodedOs::from_encoded(lit).to_bytes()).unwrap(),
            "\u{fdd0}".as_bytes()
        );
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
