//! Command line argument iteration.
//!
//! The macOS NSProcessInfo-based iterator is skipped; on Linux this is the
//! standard process argument iterator (as `os_str`s so that non-UTF-8 args
//! don't panic).

/// Returns an iterator over the command line arguments.
pub fn iterator() -> std::env::ArgsOs {
    std::env::args_os()
}

/// Collect the process arguments as `String`s without ever panicking on
/// non-UTF-8 bytes (invalid sequences become U+FFFD). The CLI/config layers
/// are string based; `+action` dispatch only depends on the ASCII prefix.
pub fn collect_lossy() -> Vec<String> {
    lossy(iterator())
}

fn lossy(it: impl Iterator<Item = std::ffi::OsString>) -> Vec<String> {
    it.map(|a| a.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let mut iter = iterator();
        assert!(!iter.next().unwrap().is_empty());
    }

    #[test]
    fn lossy_does_not_panic_on_non_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let args = vec![
            std::ffi::OsString::from("ghostty"),
            std::ffi::OsString::from_vec(vec![b'+', b'x', 0xff]),
        ];
        let out = lossy(args.into_iter());
        assert_eq!(out, vec!["ghostty".to_owned(), "+x\u{fffd}".to_owned()]);
    }
}
