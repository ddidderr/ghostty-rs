//! Command line argument iteration.
//!
//! The macOS NSProcessInfo-based iterator is skipped; on Linux this is the
//! standard process argument iterator (as `os_str`s so that non-UTF-8 args
//! don't panic).

/// Returns an iterator over the command line arguments.
pub fn iterator() -> std::env::ArgsOs {
    std::env::args_os()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let mut iter = iterator();
        assert!(!iter.next().unwrap().is_empty());
    }
}
