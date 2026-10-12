//! Byte-preserving conversions between OS strings and Rust `String`s.
//!
//! Zig treats paths, arguments and environment values as byte slices. Rust
//! Configuration and CLI parsers receive original bytes. Byte-valued fields
//! use [`EncodedOs`] for storage; legacy string-based OS plumbing and config
//! output formatting use its reversible encoding, a bijection on byte strings:
//!
//! * the escape character is the Unicode noncharacter `U+FDD0`;
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

/// The raw bytes of an OS string. Lossless on every platform: Unix returns
/// the bytes, Windows returns WTF-8 (unpaired UTF-16 surrogates are encoded
/// as 3-byte sequences, like Zig's Windows argument/environment handling).
#[cfg(unix)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(windows)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    wide_to_wtf8(&s.encode_wide().collect::<Vec<u16>>())
}

#[cfg(not(any(unix, windows)))]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    s.to_str().map_or_else(Vec::new, |s| s.as_bytes().to_vec())
}

/// `std.fs.path.isAbsolutePosix` (Zig 0.15.2).
#[must_use]
pub fn zig_is_absolute_posix(path: &[u8]) -> bool {
    path.first() == Some(&b'/')
}

/// `std.fs.path.isAbsoluteWindows` (Zig 0.15.2): any leading separator (a
/// rooted or UNC path) or `<byte>:` followed by a separator is absolute.
#[must_use]
pub fn zig_is_absolute_windows(path: &[u8]) -> bool {
    match path {
        [] => false,
        [b'/' | b'\\', ..] => true,
        [_, b':', b'/' | b'\\', ..] => true,
        _ => false,
    }
}

/// `std.fs.path.isAbsolute` for the host platform. Rust's
/// `Path::is_absolute` differs on Windows (it requires a prefix, so `\theme`
/// and `/theme` are not absolute there).
#[must_use]
pub fn zig_is_absolute(path: &[u8]) -> bool {
    if cfg!(windows) {
        zig_is_absolute_windows(path)
    } else {
        zig_is_absolute_posix(path)
    }
}

/// Build an OS string from raw bytes (WTF-8 on Windows; bytes that are not
/// well-formed WTF-8 become U+FFFD). Filesystem and process operations must
/// use their checked conversions instead.
#[cfg(unix)]
pub fn os_from_bytes(b: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(b)
}

#[cfg(windows)]
#[allow(clippy::needless_pass_by_value)] // same signature as the Unix variant
pub fn os_from_bytes(b: Vec<u8>) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&wtf8_to_wide(&b))
}

#[cfg(not(any(unix, windows)))]
#[allow(clippy::needless_pass_by_value)]
pub fn os_from_bytes(b: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&b).into_owned())
}

/// Encode UTF-16 code units (possibly with unpaired surrogates) as WTF-8.
/// Platform independent so the Windows logic is testable everywhere.
pub fn wide_to_wtf8(wide: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(wide.len());
    for r in char::decode_utf16(wide.iter().copied()) {
        match r {
            Ok(c) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Err(e) => {
                let u = u32::from(e.unpaired_surrogate());
                out.push(0xE0 | (u >> 12) as u8);
                out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
        }
    }
    out
}

/// Decode WTF-8 into UTF-16 code units. Surrogate code points encoded as
/// 3-byte sequences are kept as lone surrogates; anything not well-formed
/// WTF-8 yields U+FFFD for each offending byte.
pub fn wtf8_to_wide(b: &[u8]) -> Vec<u16> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let cont = |j: usize| {
        b.get(j)
            .filter(|c| (**c & 0xC0) == 0x80)
            .map(|c| u32::from(*c & 0x3F))
    };
    while i < b.len() {
        let lead = b[i];
        let (cp, len) = match lead {
            0x00..=0x7F => (u32::from(lead), 1),
            0xC2..=0xDF => match cont(i + 1) {
                Some(c1) => ((u32::from(lead & 0x1F) << 6) | c1, 2),
                None => (0xFFFD, 1),
            },
            0xE0..=0xEF => match (cont(i + 1), cont(i + 2)) {
                (Some(c1), Some(c2)) if !(lead == 0xE0 && b[i + 1] < 0xA0) => {
                    ((u32::from(lead & 0x0F) << 12) | (c1 << 6) | c2, 3)
                }
                _ => (0xFFFD, 1),
            },
            0xF0..=0xF4 => match (cont(i + 1), cont(i + 2), cont(i + 3)) {
                (Some(c1), Some(c2), Some(c3)) => {
                    let cp = (u32::from(lead & 0x07) << 18) | (c1 << 12) | (c2 << 6) | c3;
                    if (0x1_0000..=0x10_FFFF).contains(&cp) {
                        (cp, 4)
                    } else {
                        (0xFFFD, 1)
                    }
                }
                _ => (0xFFFD, 1),
            },
            _ => (0xFFFD, 1),
        };
        i += len;
        if cp >= 0x1_0000 {
            let c = cp - 0x1_0000;
            out.push(0xD800 | (c >> 10) as u16);
            out.push(0xDC00 | (c & 0x3FF) as u16);
        } else {
            out.push(cp as u16);
        }
    }
    out
}

/// The bytes are not well-formed WTF-8 (Zig: `error.InvalidWtf8` /
/// `error.InvalidUtf8` from `std.unicode.utf8ToUtf16Le*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid wtf-8")]
pub struct InvalidWtf8;

/// Strict [`wtf8_to_wide`]: malformed input is an error instead of U+FFFD.
/// Lone surrogates encoded as 3-byte sequences are accepted (WTF-8). This is
/// the conversion used by Windows filesystem paths; process launch requires UTF-8.
pub fn try_wtf8_to_wide(b: &[u8]) -> Result<Vec<u16>, InvalidWtf8> {
    let mut result = Vec::with_capacity(b.len());
    let mut rest = b;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                result.extend(text.encode_utf16());
                break;
            }
            Err(error) => {
                let (valid, tail) = rest.split_at(error.valid_up_to());
                // valid_up_to is guaranteed to be a valid UTF-8 prefix.
                result.extend(
                    std::str::from_utf8(valid)
                        .map_err(|_| InvalidWtf8)?
                        .encode_utf16(),
                );
                // Zig's WTF-8 validation accepts both lone and paired surrogate encodings.
                if tail.len() < 3
                    || tail[0] != 0xed
                    || !(0xa0..=0xbf).contains(&tail[1])
                    || !(0x80..=0xbf).contains(&tail[2])
                {
                    return Err(InvalidWtf8);
                }
                result.push(0xd000 | (u16::from(tail[1] & 0x3f) << 6) | u16::from(tail[2] & 0x3f));
                rest = &tail[3..];
            }
        }
    }
    Ok(result)
}

/// Strict UTF-8 conversion used by Zig's Windows Command (including its environment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid utf-8")]
pub struct InvalidUtf8;

pub fn try_utf8_to_wide(bytes: &[u8]) -> Result<Vec<u16>, InvalidUtf8> {
    Ok(std::str::from_utf8(bytes)
        .map_err(|_| InvalidUtf8)?
        .encode_utf16()
        .collect())
}

/// Zig's Windows filesystem conversion has a different error and accepts WTF-8.
pub fn windows_path_wide(bytes: &[u8]) -> std::io::Result<Vec<u16>> {
    try_wtf8_to_wide(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "BadPathName"))
}

/// OS string to reversible `String` (see the module docs). Lossless on every
/// platform (Windows goes through WTF-8).
pub fn os_to_config_string(s: &OsStr) -> String {
    bytes_to_config_string(&os_bytes(s))
}

/// Encode a string that is already valid UTF-8 (plain text, e.g. a URL). The
/// only thing that needs escaping is a literal escape character, so decoding
/// stays a bijection on every platform.
pub fn valid_str_to_config_string(s: &str) -> String {
    bytes_to_config_string(s.as_bytes())
}

/// Decode a config string for a UI-text boundary that requires UTF-8 (window
/// titles, CSS font names, toolkit namespaces): the original bytes, with
/// anything that is not valid UTF-8 replaced by U+FFFD. A literal `U+FDD0`
/// in the text is restored (not shown as the doubled escape).
pub fn config_str_to_text(s: &str) -> String {
    String::from_utf8_lossy(&config_str_to_bytes(s)).into_owned()
}

/// Inverse of [`os_to_config_string`].
pub fn config_str_to_os(s: &str) -> OsString {
    os_from_bytes(config_str_to_bytes(s))
}

/// An OS string (path, environment value, home directory, ...) carried as a
/// `String` through the reversible escape encoding of this module.
///
/// This is deliberately a distinct type: it cannot be used as text or passed
/// to filesystem/exec APIs without an explicit [`EncodedOs::to_os`] /
/// [`EncodedOs::to_path`] decode, and plain text (URLs, regex matches, ...)
/// never becomes an `EncodedOs` except through [`EncodedOs::from_plain_text`],
/// which escapes instead of decoding.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EncodedOs(String);

impl EncodedOs {
    /// Empty value.
    #[must_use]
    pub const fn new() -> Self {
        EncodedOs(String::new())
    }

    /// Encode an OS string.
    #[must_use]
    pub fn from_os(s: &OsStr) -> Self {
        EncodedOs(os_to_config_string(s))
    }

    /// Encode a path.
    #[must_use]
    pub fn from_path(p: &std::path::Path) -> Self {
        Self::from_os(p.as_os_str())
    }

    /// `std.fs.path.isAbsolute` on the decoded bytes.
    #[must_use]
    pub fn is_zig_absolute(&self) -> bool {
        zig_is_absolute(&self.to_bytes())
    }

    /// Encode raw bytes.
    #[must_use]
    pub fn from_bytes(b: &[u8]) -> Self {
        EncodedOs(bytes_to_config_string(b))
    }

    /// Adopt a string already in the internal escape encoding. Config and CLI
    /// input must use `from_bytes`; plain Rust text must use `from_plain_text`.
    #[must_use]
    pub fn from_encoded(s: impl Into<String>) -> Self {
        EncodedOs(s.into())
    }

    /// Wrap plain text (not previously encoded): a literal escape character
    /// is escaped so decoding returns exactly the same text.
    #[must_use]
    pub fn from_plain_text(s: &str) -> Self {
        EncodedOs(valid_str_to_config_string(s))
    }

    /// Internal encoded text for storage/output formatting. Decode before
    /// writing config output or passing it to OS APIs and field parsers.
    #[must_use]
    pub fn as_encoded(&self) -> &str {
        &self.0
    }

    /// Consume into the encoded string.
    #[must_use]
    pub fn into_encoded(self) -> String {
        self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the decoded value ends with the given plain suffix.
    #[must_use]
    pub fn ends_with_plain(&self, suffix: &str) -> bool {
        self.0.ends_with(suffix)
    }

    /// Decode to the raw bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        config_str_to_bytes(&self.0)
    }

    /// Render bytes for a UI or log boundary which requires Unicode text.
    pub fn to_text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.to_bytes()).into_owned()
    }

    /// Decode to an OS string.
    #[must_use]
    pub fn to_os(&self) -> OsString {
        config_str_to_os(&self.0)
    }

    /// Decode to UTF-16 code units (WTF-16: lone surrogates round-trip).
    /// This is the Windows API form; it is platform independent so the
    /// Windows logic is testable everywhere.
    #[must_use]
    pub fn to_wide(&self) -> Vec<u16> {
        wtf8_to_wide(&self.to_bytes())
    }

    /// Strict [`EncodedOs::to_wide`] for WTF-8 data: malformed WTF-8 is
    /// an error (Zig's `utf8ToUtf16Le*` fail) instead of becoming U+FFFD.
    pub fn try_to_wide(&self) -> Result<Vec<u16>, InvalidWtf8> {
        try_wtf8_to_wide(&self.to_bytes())
    }

    /// Windows process inputs use strict UTF-8, unlike filesystem paths.
    pub fn try_to_utf16(&self) -> Result<Vec<u16>, InvalidUtf8> {
        try_utf8_to_wide(&self.to_bytes())
    }

    pub fn try_to_utf16_nul(&self) -> Result<Vec<u16>, InvalidUtf8> {
        let mut wide = self.try_to_utf16()?;
        wide.push(0);
        Ok(wide)
    }

    pub fn try_to_utf8_os(&self) -> Result<OsString, InvalidUtf8> {
        let bytes = self.to_bytes();
        Ok(OsString::from(
            std::str::from_utf8(&bytes).map_err(|_| InvalidUtf8)?,
        ))
    }

    /// [`EncodedOs::try_to_wide`] plus a trailing NUL.
    pub fn try_to_wide_nul(&self) -> Result<Vec<u16>, InvalidWtf8> {
        let mut w = self.try_to_wide()?;
        w.push(0);
        Ok(w)
    }

    /// Strict [`EncodedOs::to_os`]: on Windows malformed WTF-8 is an error;
    /// elsewhere every byte string is a valid OS string.
    pub fn try_to_os(&self) -> Result<OsString, InvalidWtf8> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            Ok(OsString::from_wide(&self.try_to_wide()?))
        }
        #[cfg(not(windows))]
        {
            Ok(self.to_os())
        }
    }

    /// [`EncodedOs::to_wide`] plus a trailing NUL.
    #[must_use]
    pub fn to_wide_nul(&self) -> Vec<u16> {
        let mut w = self.to_wide();
        w.push(0);
        w
    }

    /// Encode UTF-16 code units (possibly with lone surrogates).
    #[must_use]
    pub fn from_wide(w: &[u16]) -> Self {
        Self::from_bytes(&wide_to_wtf8(w))
    }

    /// Decode to a filesystem path, rejecting malformed Windows WTF-8 as BadPathName.
    pub fn to_path(&self) -> std::io::Result<std::path::PathBuf> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            Ok(OsString::from_wide(&windows_path_wide(&self.to_bytes())?).into())
        }
        #[cfg(not(windows))]
        {
            Ok(self.to_os().into())
        }
    }

    /// [`EncodedOs::to_path`] as a syscall sees it: Zig hands paths to the
    /// OS as NUL-terminated C strings, so an interior NUL ends the path
    /// (Rust's `std::fs` would refuse the whole path instead).
    pub fn to_syscall_path(&self) -> std::io::Result<std::path::PathBuf> {
        let bytes = self.to_bytes();
        match bytes.iter().position(|&b| b == 0) {
            Some(end) => EncodedOs::from_bytes(&bytes[..end]).to_path(),
            None => self.to_path(),
        }
    }

    /// Whether the decoded value starts with the given plain prefix.
    #[must_use]
    pub fn starts_with_plain(&self, prefix: &str) -> bool {
        // Valid UTF-8 prefixes (without the escape char) encode to themselves.
        self.0.starts_with(prefix)
    }

    /// Join plain-text components like `std.fs.path.join` (empty components
    /// skipped, exactly one separator in between). Separators are ASCII and
    /// never part of an escape, so joining the encoded text is exact.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn join_plain(&self, parts: &[&str]) -> Self {
        let esc: Vec<String> = parts
            .iter()
            .map(|p| valid_str_to_config_string(p))
            .collect();
        let mut all: Vec<&str> = Vec::with_capacity(parts.len() + 1);
        all.push(&self.0);
        all.extend(esc.iter().map(String::as_str));
        EncodedOs(crate::path_join(&all))
    }

    /// Join another encoded path (same rules as [`EncodedOs::join_plain`]).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn join(&self, other: &EncodedOs) -> Self {
        EncodedOs(crate::path_join(&[&self.0, &other.0]))
    }
}

/// Compares the *encoded* form with a literal (for plain ASCII/UTF-8 text
/// without the escape character the two are identical).
impl PartialEq<str> for EncodedOs {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for EncodedOs {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Plain UTF-8 constructors: string literals never carry escape-encoded bytes.
impl From<&str> for EncodedOs {
    fn from(s: &str) -> Self {
        Self::from_plain_text(s)
    }
}
impl From<String> for EncodedOs {
    fn from(s: String) -> Self {
        Self::from_plain_text(&s)
    }
}

impl From<&OsStr> for EncodedOs {
    fn from(s: &OsStr) -> Self {
        Self::from_os(s)
    }
}

impl From<OsString> for EncodedOs {
    fn from(s: OsString) -> Self {
        Self::from_os(&s)
    }
}

impl From<&std::path::Path> for EncodedOs {
    fn from(p: &std::path::Path) -> Self {
        Self::from_path(p)
    }
}

impl From<std::path::PathBuf> for EncodedOs {
    fn from(p: std::path::PathBuf) -> Self {
        Self::from_path(&p)
    }
}

/// Path to reversible `String` (see the module docs).
pub fn path_to_config_string(p: &std::path::Path) -> String {
    os_to_config_string(p.as_os_str())
}

/// Inverse of [`path_to_config_string`].
pub fn config_str_to_path(s: &str) -> std::io::Result<std::path::PathBuf> {
    EncodedOs::from_encoded(s).to_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn syscall_path_ends_at_the_first_nul() {
        let e = EncodedOs::from_bytes(b"/a/b\0c");
        assert_eq!(
            e.to_syscall_path().unwrap(),
            std::path::PathBuf::from("/a/b")
        );
        let e = EncodedOs::from_bytes(b"/a/\xffb");
        assert_eq!(e.to_syscall_path().unwrap(), e.to_path().unwrap());
        assert_eq!(
            EncodedOs::from_bytes(b"\0x").to_syscall_path().unwrap(),
            std::path::PathBuf::new()
        );
    }

    #[test]
    fn windows_paths_fail_with_bad_path_name_instead_of_replacement() {
        let error = windows_path_wide(b"C:\\bad\xff").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "BadPathName");
        assert_eq!(
            windows_path_wide("C:\\bad\u{fffd}".as_bytes())
                .unwrap()
                .last(),
            Some(&0xfffd)
        );
        assert_eq!(windows_path_wide(b"\xed\xa0\x80").unwrap(), [0xd800]);
    }

    #[test]
    fn wtf8_accepts_separately_encoded_surrogate_pairs_like_zig() {
        assert_eq!(
            try_wtf8_to_wide(b"\xed\xa0\xbd\xed\xb8\x80"),
            Ok(vec![0xd83d, 0xde00])
        );
        assert_eq!(
            windows_path_wide(b"x\xed\xa0\xbd\xed\xb8\x80y").unwrap(),
            [0x78, 0xd83d, 0xde00, 0x79]
        );
        assert_eq!(try_wtf8_to_wide(b"\xed\xa0\xbd\xff"), Err(InvalidWtf8));
    }

    #[test]
    fn windows_command_inputs_require_strict_utf8() {
        for bytes in [
            b"\xff".as_slice(),
            b"\xed\xa0\x80",
            b"\xed\xa0\xbd\xed\xb8\x80",
        ] {
            let value = EncodedOs::from_bytes(bytes);
            assert_eq!(value.try_to_utf16(), Err(InvalidUtf8));
            assert_eq!(value.try_to_utf16_nul(), Err(InvalidUtf8));
            assert_eq!(value.try_to_utf8_os(), Err(InvalidUtf8));
        }
        assert_eq!(
            try_utf8_to_wide("😀\u{fdd0}".as_bytes()).unwrap(),
            [0xd83d, 0xde00, 0xfdd0]
        );
    }

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
    fn valid_str_encoder_escapes_literal_escape_char() {
        // Shared logic of the non-Unix encoder: U+FDD0 U+00FF must not be
        // mistaken for the escape of byte 0xFF.
        let s = "a\u{fdd0}\u{ff}";
        let e = valid_str_to_config_string(s);
        assert_eq!(e, "a\u{fdd0}\u{fdd0}\u{ff}");
        assert_eq!(config_str_to_bytes(&e), s.as_bytes());
        assert_eq!(valid_str_to_config_string("plain"), "plain");
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

    #[test]
    fn wtf8_round_trips_unpaired_surrogates() {
        // "a", lone high, "b", valid pair (U+1F600), lone low.
        let wide = [0x61u16, 0xD800, 0x62, 0xD83D, 0xDE00, 0xDC00];
        let b = wide_to_wtf8(&wide);
        assert_eq!(&b[..1], b"a");
        assert_eq!(&b[1..4], &[0xED, 0xA0, 0x80]);
        assert_eq!(wtf8_to_wide(&b), wide);
        // Through the reversible string encoding too.
        let s = bytes_to_config_string(&b);
        assert_eq!(wtf8_to_wide(&config_str_to_bytes(&s)), wide);
    }

    #[test]
    fn config_str_to_text_restores_literal_escape_char() {
        assert_eq!(
            config_str_to_text(&valid_str_to_config_string("a\u{fdd0}b")),
            "a\u{fdd0}b"
        );
        assert_eq!(
            config_str_to_text(&bytes_to_config_string(b"a\xffb")),
            "a\u{fffd}b"
        );
    }

    #[test]
    fn strict_wtf8_rejects_malformed_input() {
        assert_eq!(try_wtf8_to_wide(b"a\xffb"), Err(InvalidWtf8));
        assert_eq!(try_wtf8_to_wide(b"\xe0\x80\x80"), Err(InvalidWtf8));
        assert_eq!(try_wtf8_to_wide(b"\xc3"), Err(InvalidWtf8));
        // A real U+FFFD and lone surrogates are well-formed.
        assert_eq!(try_wtf8_to_wide("\u{FFFD}".as_bytes()), Ok(vec![0xFFFD]));
        assert_eq!(try_wtf8_to_wide(b"\xed\xa0\x80"), Ok(vec![0xD800]));
        assert_eq!(
            try_wtf8_to_wide("\u{e9}\u{1F600}".as_bytes()),
            Ok(vec![0xE9, 0xD83D, 0xDE00])
        );
        let bad = EncodedOs::from_bytes(b"C:\\\xff");
        assert_eq!(bad.try_to_wide(), Err(InvalidWtf8));
        assert_eq!(bad.try_to_wide_nul(), Err(InvalidWtf8));
        // The lossy form still substitutes (non-launch uses).
        assert_eq!(bad.to_wide().last(), Some(&0xFFFD));
        let ok = EncodedOs::from_plain_text("C:\\x");
        assert_eq!(ok.try_to_wide_nul().unwrap().last(), Some(&0));
    }

    #[test]
    fn wtf8_invalid_bytes_become_replacement() {
        assert_eq!(wtf8_to_wide(b"a\xffb"), vec![0x61, 0xFFFD, 0x62]);
        assert_eq!(wtf8_to_wide(b"\xe0\x80\x80"), vec![0xFFFD; 3]);
        assert_eq!(
            wtf8_to_wide("\u{e9}\u{1F600}".as_bytes()),
            vec![0xE9, 0xD83D, 0xDE00]
        );
    }

    #[test]
    fn encoded_os_plain_text_is_not_decoded() {
        let plain = "http://x/\u{fdd0}\u{ff}";
        let e = EncodedOs::from_plain_text(plain);
        assert_eq!(e.to_bytes(), plain.as_bytes());
        // Adopting the same text as "already encoded" would decode to 0xFF.
        assert_ne!(EncodedOs::from_encoded(plain).to_bytes(), plain.as_bytes());
    }

    #[test]
    fn encoded_os_wide_round_trips_lone_surrogates() {
        let wide = [0x43u16, 0x3A, 0x5C, 0xD800, 0x61, 0xD83D, 0xDE00, 0xDFFF];
        let e = EncodedOs::from_wide(&wide);
        assert_eq!(e.to_wide(), wide);
        assert_eq!(*e.to_wide_nul().last().unwrap(), 0);
        // A literal escape char in a Windows name is still preserved.
        let w2: Vec<u16> = "a\u{fdd0}\u{ff}".encode_utf16().collect();
        assert_eq!(EncodedOs::from_wide(&w2).to_wide(), w2);
    }

    #[cfg(unix)]
    #[test]
    fn encoded_os_unix_round_trip() {
        let o = os_from_bytes(vec![b'/', 0xff, b'x']);
        let e = EncodedOs::from_os(&o);
        assert_eq!(e.to_os(), o);
        assert_eq!(e.to_path().unwrap(), std::path::PathBuf::from(&o));
        assert_eq!(e.join_plain(&["a", "", "b"]).to_bytes(), b"/\xffx/a/b");
    }

    #[cfg(unix)]
    #[test]
    fn os_roundtrip() {
        let o = os_from_bytes(vec![0xff, b'/', 0x80]);
        let s = os_to_config_string(&o);
        assert_eq!(config_str_to_os(&s), o);
    }
}

#[cfg(test)]
mod zig_absolute_tests {
    use super::*;

    #[test]
    fn windows_is_absolute_matches_zig_0_15_2() {
        for (p, want) in [
            ("", false),
            ("/", true),
            ("//server/file", true),
            ("\\\\server", true),
            ("\\theme", true),
            ("/theme", true),
            ("c", false),
            ("c:", false),
            ("c:\\", true),
            ("c:/", true),
            ("1:/x", true),
            ("C:cwd/another", false),
            ("directory\\directory", false),
        ] {
            assert_eq!(zig_is_absolute_windows(p.as_bytes()), want, "{p}");
        }
    }

    #[test]
    fn posix_is_absolute_matches_zig_0_15_2() {
        assert!(!zig_is_absolute_posix(b""));
        assert!(zig_is_absolute_posix(b"/home/foo"));
        assert!(!zig_is_absolute_posix(b"bar/"));
        assert!(!zig_is_absolute_posix(b"\\theme"));
    }
}
