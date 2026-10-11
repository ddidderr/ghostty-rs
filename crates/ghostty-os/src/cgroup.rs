use std::io::Read;

/// Maximum number of bytes read from the cgroup file (the Zig version reads
/// into a caller-provided buffer; callers there use path-sized buffers).
const READ_LIMIT: u64 = 4096;

/// Returns the path to the cgroup for the given pid.
pub fn current(pid: u32) -> Option<String> {
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
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
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
}
