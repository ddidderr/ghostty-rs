//! `Helpers/Extensions/String+Extension.swift`.

use std::path::PathBuf;

/// `String.truncate(length:trailing:)`: counts Characters (extended grapheme
/// clusters in Swift; Unicode scalars here, which is identical for the text
/// this is used on).
#[must_use]
pub fn truncate(s: &str, length: usize, trailing: &str) -> String {
    let trailing_len = trailing.chars().count();
    let count = s.chars().count();
    if length <= trailing_len || s.is_empty() || count <= length {
        return s.to_string();
    }
    let max_length = length - trailing_len;
    let mut out: String = s.chars().take(max_length).collect();
    out.push_str(trailing);
    out
}

/// `truncate(length:)` with the default `"…"` trailing text.
#[must_use]
pub fn truncate_default(s: &str, length: usize) -> String {
    truncate(s, length, "…")
}

/// `abbreviatedPath`: the path with the home directory abbreviated as `~`.
#[must_use]
pub fn abbreviated_path(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// `String.temporaryFile(_:)`: write the string to `<tmp>/<filename>.txt`
/// (errors are ignored like `try?`) and return the URL path.
#[must_use]
pub fn temporary_file(contents: &str, temp_dir: &std::path::Path, filename: &str) -> PathBuf {
    let mut path = temp_dir.join(filename);
    path.set_extension("txt");
    let _ = std::fs::write(&path, contents);
    path
}

/// `String.fourCharCode`: a four-character ASCII string as a big-endian
/// `FourCharCode` (`UInt32`).
#[must_use]
pub fn four_char_code(s: &str) -> u32 {
    debug_assert!(
        s.chars().count() <= 4,
        "FourCharCode string must be at most 4 characters"
    );
    s.bytes()
        .take(4)
        .fold(0u32, |acc, b| (acc << 8) | u32::from(b))
}

/// The home-dependent wrapper (`FileManager.homeDirectoryForCurrentUser`).
#[must_use]
pub fn abbreviated_path_for_current_user(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    abbreviated_path(path, &home)
}

pub trait StringExt {
    fn truncate(&self, length: usize) -> String;
    fn abbreviated_path(&self) -> String;
    fn four_char_code(&self) -> u32;
}

impl StringExt for str {
    fn truncate(&self, length: usize) -> String {
        truncate_default(self, length)
    }
    fn abbreviated_path(&self) -> String {
        abbreviated_path_for_current_user(self)
    }
    fn four_char_code(&self) -> u32 {
        four_char_code(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_cases() {
        assert_eq!(truncate_default("hello", 10), "hello");
        assert_eq!(truncate_default("hello world", 8), "hello w…");
        assert_eq!(truncate_default("hello", 5), "hello");
        assert_eq!(truncate_default("", 3), "");
        // maxLength <= 0 returns self
        assert_eq!(truncate("abcdef", 1, "…"), "abcdef");
        assert_eq!(truncate("abcdef", 3, ".."), "a..");
    }

    #[test]
    fn abbreviates_home() {
        assert_eq!(abbreviated_path("/Users/x/dev", "/Users/x"), "~/dev");
        assert_eq!(abbreviated_path("/opt/x", "/Users/x"), "/opt/x");
    }

    #[test]
    fn four_cc() {
        assert_eq!(four_char_code("why?"), 0x7768_793F);
        assert_eq!(four_char_code("ab"), 0x6162);
        assert_eq!(four_char_code(""), 0);
    }

    #[test]
    fn temp_file_written() {
        let dir = std::env::temp_dir();
        let p = temporary_file("data", &dir, "ghostty-macos-test-temp");
        assert_eq!(p.extension().unwrap(), "txt");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "data");
        let _ = std::fs::remove_file(p);
    }
}
