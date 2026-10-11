use std::collections::HashMap;

use crate::flatpak::is_flatpak;

/// PATH-style delimiter (`std.fs.path.delimiter` on POSIX).
const DELIMITER: char = ':';

/// Get the environment map. Inside Flatpak this is empty (the sandbox
/// environment is not meaningful for host commands).
pub fn get_env_map() -> HashMap<String, String> {
    if is_flatpak() {
        return HashMap::new();
    }
    std::env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect()
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

/// Prepend a value to an environment variable such as PATH.
pub fn prepend_env(current: &str, value: &str) -> String {
    // If there is no prior value, we return it as-is
    if current.is_empty() {
        return value.to_owned();
    }

    format!("{value}{DELIMITER}{current}")
}

/// Gets the value of an environment variable, or None if not found
/// (or not valid unicode).
pub fn getenv(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Gets the value of an environment variable. Returns None if not found or
/// the value is empty.
pub fn getenv_not_empty(key: &str) -> Option<String> {
    getenv(key).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_env_empty() {
        assert_eq!(append_env("", "foo"), "foo");
    }

    #[test]
    fn append_env_existing() {
        assert_eq!(append_env("a:b", "foo"), "a:b:foo");
    }

    #[test]
    fn prepend_env_empty() {
        assert_eq!(prepend_env("", "foo"), "foo");
    }

    #[test]
    fn prepend_env_existing() {
        assert_eq!(prepend_env("a:b", "foo"), "foo:a:b");
    }

    #[test]
    fn append_env_always_empty_value() {
        assert_eq!(append_env_always("a", ""), "a:");
        assert_eq!(append_env_always("", "x"), ":x");
    }

    #[test]
    fn getenv_not_empty_filters() {
        temp_env::with_vars(
            [("GHOSTTY_RS_T1", Some("")), ("GHOSTTY_RS_T2", Some("v"))],
            || {
                assert_eq!(getenv("GHOSTTY_RS_T1").as_deref(), Some(""));
                assert_eq!(getenv_not_empty("GHOSTTY_RS_T1"), None);
                assert_eq!(getenv_not_empty("GHOSTTY_RS_T2").as_deref(), Some("v"));
                assert_eq!(getenv_not_empty("GHOSTTY_RS_NOPE"), None);
            },
        );
    }
}
