use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use crate::flatpak::is_flatpak;

/// PATH-style delimiter (`std.fs.path.delimiter`).
#[cfg(windows)]
const DELIMITER: char = ';';
#[cfg(not(windows))]
const DELIMITER: char = ':';

/// The comparison form of an environment key on Windows: its WTF-16 code
/// units, uppercased per unit. Never lossy: distinct lone surrogates stay
/// distinct.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_key(key: &OsStr) -> Vec<u16> {
    crate::osbytes::wtf8_to_wide(&crate::osbytes::os_bytes(key))
        .into_iter()
        .map(crate::windows_nls::upcase_w)
        .collect()
}

/// A byte-preserving environment map (Zig `std.process.EnvMap`): keys and
/// values are arbitrary OS strings, never required to be UTF-8.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvMap(HashMap<OsString, OsString>);

impl EnvMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// On Windows environment variable names are case-insensitive (Zig's
    /// `EnvMap` uses a case-insensitive hash context there), so lookups find
    /// the existing key regardless of ASCII case. Elsewhere this is the key
    /// itself.
    #[cfg(windows)]
    fn canonical_key<'a>(&'a self, key: &'a OsStr) -> Cow<'a, OsStr> {
        let want = windows_key(key);
        self.0
            .keys()
            .find(|k| windows_key(k) == want)
            .map_or(Cow::Borrowed(key), |k| Cow::Borrowed(k.as_os_str()))
    }

    #[cfg(not(windows))]
    #[allow(clippy::unused_self)]
    fn canonical_key<'a>(&'a self, key: &'a OsStr) -> Cow<'a, OsStr> {
        Cow::Borrowed(key)
    }

    pub fn insert(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Option<OsString> {
        let key = self.canonical_key(key.as_ref()).into_owned();
        self.0.insert(key, value.as_ref().to_owned())
    }

    pub fn get(&self, key: impl AsRef<OsStr>) -> Option<&OsStr> {
        self.0
            .get(&*self.canonical_key(key.as_ref()))
            .map(OsString::as_os_str)
    }

    pub fn remove(&mut self, key: impl AsRef<OsStr>) -> Option<OsString> {
        let key = self.canonical_key(key.as_ref()).into_owned();
        self.0.remove(&key)
    }

    pub fn contains_key(&self, key: impl AsRef<OsStr>) -> bool {
        self.0.contains_key(&*self.canonical_key(key.as_ref()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.0.iter().map(|(k, v)| (k.as_os_str(), v.as_os_str()))
    }
}

impl<K: AsRef<OsStr>> std::ops::Index<K> for EnvMap {
    type Output = OsStr;

    fn index(&self, key: K) -> &OsStr {
        self.get(key).expect("no entry found for key")
    }
}

impl<K: Into<OsString>, V: Into<OsString>> FromIterator<(K, V)> for EnvMap {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl IntoIterator for EnvMap {
    type Item = (OsString, OsString);
    type IntoIter = std::collections::hash_map::IntoIter<OsString, OsString>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Get the environment map. Inside Flatpak this is empty (the sandbox
/// environment is not meaningful for host commands). Bytes are preserved.
pub fn get_env_map() -> EnvMap {
    if is_flatpak() {
        return EnvMap::new();
    }
    std::env::vars_os().collect()
}

/// Filter out Snap-specific environment variables and path components
/// (port of `Surface.filterSnapPaths`): remove any `SNAP_*` variable and
/// drop `:`-separated components that start with one of the snap
/// directories. Variables we set ourselves (`TERMINFO`, `GHOSTTY*`) are
/// left alone.
#[cfg(windows)]
pub fn filter_snap_paths(_env: &mut EnvMap) {
    // Snap only exists on Linux.
}

/// See the Windows no-op above.
#[cfg(unix)]
pub fn filter_snap_paths(env: &mut EnvMap) {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    const SNAP_VARS: [&str; 5] = [
        "SNAP",
        "SNAP_USER_COMMON",
        "SNAP_USER_DATA",
        "SNAP_DATA",
        "SNAP_COMMON",
    ];

    let mut to_remove: Vec<OsString> = Vec::new();
    let mut to_update: Vec<(OsString, OsString)> = Vec::new();

    for (key, value) in env.iter() {
        let kb = key.as_bytes();
        if kb == b"TERMINFO" || kb.starts_with(b"GHOSTTY") {
            continue;
        }
        if kb.starts_with(b"SNAP_") {
            to_remove.push(key.to_owned());
            continue;
        }

        let mut filtered: Vec<&[u8]> = Vec::new();
        let mut modified = false;
        for path in value.as_bytes().split(|&b| b == b':') {
            let mut include = true;
            for k in SNAP_VARS {
                if let Some(snap_path) = env.get(k) {
                    let snap_path = snap_path.as_bytes();
                    if snap_path.is_empty() {
                        continue;
                    }
                    if path.starts_with(snap_path) {
                        include = false;
                        modified = true;
                        break;
                    }
                }
            }
            if include {
                filtered.push(path);
            }
        }

        if modified {
            if filtered.is_empty() {
                to_remove.push(key.to_owned());
            } else {
                to_update.push((key.to_owned(), OsString::from_vec(filtered.join(&b':'))));
            }
        }
    }

    for (k, v) in to_update {
        env.insert(k, v);
    }
    for k in to_remove {
        env.remove(k);
    }
}

/// Append a value to an environment variable such as PATH.
pub fn append_env(current: &str, value: &str) -> String {
    // If there is no prior value, we return it as-is
    if current.is_empty() {
        return value.to_owned();
    }

    // Otherwise we must prefix.
    append_env_always(current, value)
}

/// Always append value to environment, even when it is empty.
/// This is useful because some env vars (like MANPATH) want there
/// to be an empty prefix to preserve existing values.
pub fn append_env_always(current: &str, value: &str) -> String {
    format!("{current}{DELIMITER}{value}")
}

/// Byte-preserving [`append_env_always`].
pub fn append_env_always_os(current: &OsStr, value: &OsStr) -> OsString {
    let mut out = current.to_owned();
    out.push(DELIMITER.to_string());
    out.push(value);
    out
}

/// Byte-preserving [`prepend_env`].
pub fn prepend_env_os(current: &OsStr, value: &OsStr) -> OsString {
    if current.is_empty() {
        return value.to_owned();
    }
    let mut out = value.to_owned();
    out.push(DELIMITER.to_string());
    out.push(current);
    out
}

/// Byte-preserving [`append_env`].
pub fn append_env_os(current: &OsStr, value: &OsStr) -> OsString {
    if current.is_empty() {
        return value.to_owned();
    }
    let mut out = current.to_owned();
    out.push(DELIMITER.to_string());
    out.push(value);
    out
}

/// Prepend a value to an environment variable such as PATH.
pub fn prepend_env(current: &str, value: &str) -> String {
    // If there is no prior value, we return it as-is
    if current.is_empty() {
        return value.to_owned();
    }

    format!("{value}{DELIMITER}{current}")
}

/// Gets the raw value of an environment variable, or None if not found.
/// A present value that is not valid UTF-8 is returned as-is (never treated
/// as absent).
pub fn getenv_os(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

/// Gets the raw value of an environment variable. Returns None if not found
/// or the value is empty.
pub fn getenv_not_empty_os(key: &str) -> Option<OsString> {
    getenv_os(key).filter(|v| !v.is_empty())
}

/// Gets the value of an environment variable, or None if not found. A
/// non-UTF-8 value is still present: its invalid bytes are carried through
/// the reversible [`crate::osbytes`] encoding.
pub fn getenv(key: &str) -> Option<crate::osbytes::EncodedOs> {
    getenv_os(key).map(|v| crate::osbytes::EncodedOs::from_os(&v))
}

/// Gets the value of an environment variable. Returns None if not found or
/// the value is empty.
pub fn getenv_not_empty(key: &str) -> Option<crate::osbytes::EncodedOs> {
    getenv(key).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    #[cfg(unix)]
    #[test]
    fn env_map_preserves_non_utf8() {
        let mut env = EnvMap::new();
        let v = OsString::from_vec(vec![b'a', 0xff, b'b']);
        env.insert("K", &v);
        assert_eq!(env.get("K"), Some(v.as_os_str()));
    }

    #[cfg(unix)]
    #[test]
    fn filter_snap_paths_filters() {
        let mut env = EnvMap::new();
        env.insert("SNAP", "/snap/ghostty/1");
        env.insert("SNAP_NAME", "ghostty");
        env.insert("GHOSTTY_X", "/snap/ghostty/1/a");
        env.insert("TERMINFO", "/snap/ghostty/1/ti");
        env.insert("PATH", "/snap/ghostty/1/bin:/usr/bin:/snap/ghostty/1/x");
        env.insert("ONLY", "/snap/ghostty/1/lib");
        env.insert("KEEP", "/usr/lib");
        filter_snap_paths(&mut env);
        assert!(!env.contains_key("SNAP"));
        assert!(!env.contains_key("SNAP_NAME"));
        assert_eq!(env["GHOSTTY_X"], *"/snap/ghostty/1/a");
        assert_eq!(env["TERMINFO"], *"/snap/ghostty/1/ti");
        assert_eq!(env["PATH"], *"/usr/bin");
        assert!(!env.contains_key("ONLY"));
        assert_eq!(env["KEEP"], *"/usr/lib");
    }

    #[test]
    fn append_env_empty() {
        assert_eq!(append_env("", "foo"), "foo");
    }

    #[test]
    fn append_env_existing() {
        assert_eq!(append_env("a:b", "foo"), format!("a:b{DELIMITER}foo"));
    }

    #[cfg(unix)]
    #[test]
    fn getenv_non_utf8_is_present() {
        let v = OsString::from_vec(vec![b'a', 0xff]);
        temp_env::with_var("GHOSTTY_RS_T3", Some(&v), || {
            assert_eq!(getenv_os("GHOSTTY_RS_T3").unwrap(), v);
            assert_eq!(getenv_not_empty_os("GHOSTTY_RS_T3").unwrap(), v);
            let s = getenv("GHOSTTY_RS_T3").unwrap();
            assert_eq!(
                crate::osbytes::config_str_to_bytes(s.as_encoded()),
                vec![b'a', 0xff]
            );
            assert!(getenv_not_empty("GHOSTTY_RS_T3").is_some());
        });
    }

    #[cfg(unix)]
    #[test]
    fn os_variants_preserve_bytes() {
        let cur = OsString::from_vec(vec![0xff, b'a']);
        let out = prepend_env_os(&cur, OsStr::new("x"));
        assert_eq!(out.into_vec(), vec![b'x', b':', 0xff, b'a']);
        let out = append_env_os(&cur, OsStr::new("x"));
        assert_eq!(out.into_vec(), vec![0xff, b'a', b':', b'x']);
        assert_eq!(append_env_os(OsStr::new(""), OsStr::new("x")), "x");
    }

    #[test]
    fn prepend_env_empty() {
        assert_eq!(prepend_env("", "foo"), "foo");
    }

    #[test]
    fn prepend_env_existing() {
        assert_eq!(prepend_env("a:b", "foo"), format!("foo{DELIMITER}a:b"));
    }

    #[test]
    fn append_env_always_empty_value() {
        assert_eq!(append_env_always("a", ""), format!("a{DELIMITER}"));
        assert_eq!(append_env_always("", "x"), format!("{DELIMITER}x"));
    }

    #[test]
    fn getenv_not_empty_filters() {
        temp_env::with_vars(
            [("GHOSTTY_RS_T1", Some("")), ("GHOSTTY_RS_T2", Some("v"))],
            || {
                assert_eq!(getenv("GHOSTTY_RS_T1").unwrap(), "");
                assert_eq!(getenv_not_empty("GHOSTTY_RS_T1"), None);
                assert_eq!(getenv_not_empty("GHOSTTY_RS_T2").unwrap(), "v");
                assert_eq!(getenv_not_empty("GHOSTTY_RS_NOPE"), None);
            },
        );
    }
}

#[cfg(test)]
mod windows_key_tests {
    #[cfg(unix)]
    use super::*;

    fn key(units: &[u16]) -> Vec<u16> {
        units
            .iter()
            .copied()
            .map(crate::windows_nls::upcase_w)
            .collect()
    }

    #[test]
    fn keys_use_the_windows_table_not_unicode_mappings() {
        // U+1F80 and U+1F88 are the same key under Windows/Zig's table even
        // though Unicode's uppercase of U+1F80 is a two-unit sequence.
        assert_eq!(key(&[0x1F80]), key(&[0x1F88]));
        // The Georgian letters gained uppercase after the table froze.
        assert_ne!(key(&[0x10D0]), key(&[0x1C90]));
        // U+00DF stays distinct from "SS" (and from U+1E9E).
        assert_ne!(key(&[0xDF]), key(&[0x53, 0x53]));
    }

    #[test]
    fn non_ascii_case_keys_are_equal() {
        assert_eq!(key(&[0xE9, 0x61]), key(&[0xC9, 0x41]));
    }

    #[test]
    fn distinct_lone_surrogate_keys_stay_distinct() {
        assert_ne!(key(&[0x41, 0xD800]), key(&[0x41, 0xD801]));
        assert_ne!(key(&[0xD800]), key(&[0xFFFD]));
    }

    #[cfg(unix)]
    #[test]
    fn windows_key_of_os_str_is_wtf16() {
        use std::os::unix::ffi::OsStrExt;
        // Unix OsStr "Aé" in UTF-8.
        assert_eq!(
            windows_key(OsStr::from_bytes("a\u{e9}".as_bytes())),
            vec![0x41, 0xC9]
        );
    }
}
