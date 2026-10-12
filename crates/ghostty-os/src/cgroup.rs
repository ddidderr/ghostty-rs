use std::io::Read;

/// Maximum number of bytes read from the cgroup file (the Zig version reads
/// into a caller-provided buffer; callers there use path-sized buffers).
const READ_LIMIT: u64 = 4096;

/// Returns the path to the cgroup for the given pid. Non-UTF-8 bytes are
/// carried via [`crate::osbytes`]; see [`current_os`] for the raw bytes.
pub fn current(pid: u32) -> Option<String> {
    current_os(pid).map(|p| crate::osbytes::os_to_config_string(&p))
}

/// Byte-preserving [`current`].
pub fn current_os(pid: u32) -> Option<std::ffi::OsString> {
    // Read our cgroup by opening /proc/<pid>/cgroup and reading the first
    // line. The first line will look something like this:
    // 0::/user.slice/user-1000.slice/session-1.scope
    // The cgroup path is the third field.
    let file = std::fs::File::open(format!("/proc/{pid}/cgroup")).ok()?;
    let mut contents = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut contents).ok()?;

    // Find the last ':'
    let idx = contents.iter().rposition(|&b| b == b':')?;
    let rest = &contents[idx + 1..];
    let end = rest
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\r' | b'\n'))
        .map_or(0, |i| i + 1);
    Some(crate::osbytes::os_from_bytes(rest[..end].to_vec()))
}

/// True if the final path component of `pid`'s cgroup equals `expected`.
pub fn scope_matches(pid: u32, expected: &str) -> bool {
    current_os(pid).is_some_and(|raw| {
        let raw = crate::osbytes::os_bytes(&raw);
        raw.rsplit(|&b| b == b'/').next() == Some(expected.as_bytes()) && raw.contains(&b'/')
    })
}

/// Poll (every 25ms, at most 250ms) until `pid`'s cgroup ends in `expected`.
/// Returns true if seen, false on timeout. This is the single wait loop
/// shared by the termio `Command` hooks and apprt-gtk's pre-exec/post-fork.
pub fn wait_for_scope(pid: u32, expected: &str) -> bool {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > std::time::Duration::from_millis(250) {
            return false;
        }
        if scope_matches(pid, expected) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_cgroup_of_self() {
        // Not part of the Zig tests; sanity check that parsing works when
        // /proc is available.
        if let Some(p) = current(std::process::id()) {
            assert!(!p.ends_with('\n'));
        }
        assert!(current(u32::MAX).is_none());
    }

    #[test]
    fn scope_matches_requires_slash_and_last_component() {
        // Our own cgroup path always matches its own last component.
        if let Some(p) = current(std::process::id())
            && let Some((_, last)) = p.rsplit_once('/')
        {
            assert!(scope_matches(std::process::id(), last));
        }
        assert!(!scope_matches(u32::MAX, "x"));
    }
}
