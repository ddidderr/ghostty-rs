#[cfg(unix)]
use rustix::process::{Resource, Rlimit as RustixRlimit, getrlimit, setrlimit};

/// Resource limit (`rlimit` in Zig). `None` means infinity.
#[cfg(unix)]
pub type Rlimit = RustixRlimit;

/// rlim_t value for infinity in the Zig implementation.
#[cfg(unix)]
const INFINITY: u64 = u64::MAX;

#[cfg(unix)]
fn cur(l: &Rlimit) -> u64 {
    l.current.unwrap_or(INFINITY)
}
#[cfg(unix)]
fn max(l: &Rlimit) -> u64 {
    l.maximum.unwrap_or(INFINITY)
}
#[cfg(unix)]
fn mk(cur: u64, max: Option<u64>) -> Rlimit {
    Rlimit {
        current: if cur == INFINITY { None } else { Some(cur) },
        maximum: max,
    }
}

/// This maximizes the number of file descriptors we can have open. We
/// need to do this because each window consumes at least a handful of fds.
/// This is extracted from the Zig compiler source code.
#[cfg(unix)]
pub fn fix_max_files() -> Option<Rlimit> {
    let old = getrlimit(Resource::Nofile);

    // If we're already at the max, we're done.
    if cur(&old) >= max(&old) {
        log::debug!("file handle limit already maximized value={}", cur(&old));
        return Some(old);
    }

    // Do a binary search for the limit.
    let mut lim = old;
    let mut min_v: u64 = cur(&lim);
    let mut max_v: u64 = 1 << 20;
    // But if there's a defined upper bound, don't search, just set it.
    if max(&lim) != INFINITY {
        min_v = max(&lim);
        max_v = max(&lim);
    }

    loop {
        let c = min_v + (max_v - min_v) / 2;
        lim = mk(c, lim.maximum);
        if setrlimit(Resource::Nofile, lim).is_ok() {
            min_v = c;
        } else {
            max_v = c;
        }
        if min_v + 1 >= max_v {
            break;
        }
    }

    log::debug!("file handle limit raised value={}", cur(&lim));
    Some(old)
}

#[cfg(unix)]
pub fn restore_max_files(lim: Rlimit) {
    let _ = setrlimit(Resource::Nofile, lim);
}

/// Return the recommended path for temporary files.
pub fn alloc_tmp_dir() -> Option<String> {
    if cfg!(windows) {
        // TODO: what is a good fallback path on windows? (same as Zig)
        return std::env::var_os("TMP").map(|v| v.to_string_lossy().into_owned());
    }
    if let Some(v) = std::env::var_os("TMPDIR") {
        return Some(v.to_string_lossy().into_owned());
    }
    if let Some(v) = std::env::var_os("TMP") {
        return Some(v.to_string_lossy().into_owned());
    }
    Some("/tmp".to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn tmp_dir_env_precedence() {
        temp_env::with_vars([("TMPDIR", Some("/a")), ("TMP", Some("/b"))], || {
            assert_eq!(alloc_tmp_dir().as_deref(), Some("/a"));
        });
        temp_env::with_vars([("TMPDIR", None), ("TMP", Some("/b"))], || {
            assert_eq!(alloc_tmp_dir().as_deref(), Some("/b"));
        });
        temp_env::with_vars([("TMPDIR", None::<&str>), ("TMP", None)], || {
            assert_eq!(alloc_tmp_dir().as_deref(), Some("/tmp"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn fix_and_restore_max_files() {
        let old = fix_max_files().unwrap();
        let now = getrlimit(Resource::Nofile);
        assert!(cur(&now) >= cur(&old));
        restore_max_files(old);
        assert_eq!(getrlimit(Resource::Nofile), old);
    }
}
