use std::io::Read;

/// Returns the kernel release string (`/proc/sys/kernel/osrelease`).
pub fn get_kernel_info() -> Option<String> {
    let file = std::fs::File::open("/proc/sys/kernel/osrelease").ok()?;

    // 128 bytes should be enough to hold the kernel information. Zig's
    // readToEndAlloc fails if the file is larger than the limit.
    let mut buf = Vec::new();
    file.take(129).read_to_end(&mut buf).ok()?;
    if buf.len() > 128 {
        return None;
    }
    let s = String::from_utf8_lossy(&buf);
    Some(s.trim_matches(|c: char| c.is_ascii_whitespace()).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_proc_sys_kernel_osrelease() {
        let kernel_info = get_kernel_info().unwrap();
        // Since we can't hardcode the info in tests, just check
        // if something was read from the file
        assert!(!kernel_info.is_empty());
    }
}
