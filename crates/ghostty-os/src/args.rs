//! Command line argument iteration.
//!
//! The macOS NSProcessInfo-based iterator is skipped; on Linux this is the
//! standard process argument iterator (as `os_str`s so that non-UTF-8 args
//! don't panic).

/// Returns an iterator over the command line arguments.
///
/// If [`set_argv`] was called (the embedded `ghostty_init`, Zig:
/// `std.os.argv = argv[0..argc]`) those arguments are returned instead of
/// the process arguments.
#[cfg(not(target_os = "macos"))]
pub fn iterator() -> std::vec::IntoIter<std::ffi::OsString> {
    match ARGV_OVERRIDE.lock().ok().and_then(|g| g.clone()) {
        Some(v) => v.into_iter(),
        None => std::env::args_os().collect::<Vec<_>>().into_iter(),
    }
}

static ARGV_OVERRIDE: std::sync::Mutex<Option<Vec<std::ffi::OsString>>> =
    std::sync::Mutex::new(None);

/// Replace the process arguments seen by [`iterator`] (Zig's assignment to
/// `std.os.argv` in `ghostty_init`). Has no effect on macOS, where the
/// arguments always come from `NSProcessInfo` (as in Zig).
pub fn set_argv(args: Vec<std::ffi::OsString>) {
    if let Ok(mut g) = ARGV_OVERRIDE.lock() {
        *g = Some(args);
    }
}

/// Returns an iterator over the command line arguments.
///
/// On macOS this reads `NSProcessInfo.arguments` instead of libc
/// argc/argv, which are not valid for `NSApplicationMain`-based
/// applications (Zig: `IteratorMacOS`).
#[cfg(target_os = "macos")]
pub fn iterator() -> std::vec::IntoIter<std::ffi::OsString> {
    use objc2_foundation::NSProcessInfo;

    NSProcessInfo::processInfo()
        .arguments()
        .iter()
        .map(|s| std::ffi::OsString::from(s.to_string()))
        .collect::<Vec<_>>()
        .into_iter()
}

/// Collect the process arguments as raw OS strings (no conversion, never
/// panics on non-UTF-8 bytes).
pub fn collect_os() -> Vec<std::ffi::OsString> {
    iterator().collect()
}

/// Collect the process arguments as raw byte vectors (Zig `[]const u8`).
pub fn collect_bytes() -> Vec<Vec<u8>> {
    iterator().map(|a| crate::osbytes::os_bytes(&a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let mut iter = iterator();
        assert!(!iter.next().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn config_strings_keep_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let a = std::ffi::OsString::from_vec(vec![b'+', b'x', 0xff]);
        let s = crate::osbytes::os_to_config_string(&a);
        assert_eq!(
            crate::osbytes::config_str_to_bytes(&s),
            vec![b'+', b'x', 0xff]
        );
    }
}
