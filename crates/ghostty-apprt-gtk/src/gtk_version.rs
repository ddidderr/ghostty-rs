//! GTK version helpers (port of `apprt/gtk/gtk_version.zig`).
//!
//! Zig reads the compile-time version from the headers; here it is the
//! version of the GTK development files reported by pkg-config at build time
//! (see `build.rs`) combined with the system library detected at runtime.

/// A (major, minor, micro) version triple.
pub type Version = (u32, u32, u32);

/// The GTK version this crate is compiled against (pkg-config `gtk4`).
/// Newer symbols must still be guarded with [`at_least`]/feature flags.
pub const COMPTIME_VERSION: Version = parse_version(env!("GHOSTTY_GTK_VERSION"));

/// Parses a `major.minor[.micro]` string at compile time.
pub(crate) const fn parse_version(s: &str) -> (u32, u32, u32) {
    let b = s.as_bytes();
    let mut parts = [0u32; 3];
    let mut idx = 0;
    let mut i = 0;
    while i < b.len() && idx < 3 {
        let c = b[i];
        if c == b'.' {
            idx += 1;
        } else if c.is_ascii_digit() {
            parts[idx] = parts[idx] * 10 + (c - b'0') as u32;
        } else {
            break;
        }
        i += 1;
    }
    (parts[0], parts[1], parts[2])
}

/// Returns the runtime GTK version.
pub fn get_runtime_version() -> Version {
    (
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
    )
}

/// Logs the build and runtime GTK version.
pub fn log_version() {
    let (a, b, c) = COMPTIME_VERSION;
    let (x, y, z) = get_runtime_version();
    log::info!("GTK version build={a}.{b}.{c} runtime={x}.{y}.{z}");
}

/// Verifies that the GTK version at runtime is at least the given version.
/// Unlike Zig this can't compile out unknown symbols; callers needing newer
/// API must gate it on a cargo feature too.
pub fn at_least(major: u32, minor: u32, micro: u32) -> bool {
    // Anything below the build level is trivially satisfied by the runtime
    // requirement of the build level, but we still check the runtime.
    runtime_at_least(major, minor, micro)
}

/// Verifies that the GTK runtime version is at least the given version.
pub fn runtime_at_least(major: u32, minor: u32, micro: u32) -> bool {
    get_runtime_version() >= (major, minor, micro)
}

/// Verifies that the GTK runtime version is older than the given version.
pub fn runtime_until(major: u32, minor: u32, micro: u32) -> bool {
    get_runtime_version() < (major, minor, micro)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_test() {
        let (a, b, c) = get_runtime_version();
        for f in [at_least, runtime_at_least] {
            assert!(f(a, b, c));
            assert!(!f(a, b, c + 1));
            assert!(!f(a, b + 1, c));
            assert!(!f(a + 1, b, c));
            assert!(f(a - 1, b, c));
            assert!(f(a - 1, b + 1, c));
            assert!(f(a - 1, b, c + 1));
            assert!(f(a, b - 1, c + 1));
        }
    }

    #[test]
    fn runtime_until_test() {
        let (a, b, c) = get_runtime_version();
        assert!(!runtime_until(a, b, c));
        assert!(runtime_until(a, b, c + 1));
        assert!(runtime_until(a, b + 1, c));
        assert!(runtime_until(a + 1, b, c));
        assert!(!runtime_until(a - 1, b, c));
        assert!(!runtime_until(a - 1, b + 1, c));
        assert!(!runtime_until(a, b - 1, c + 1));
    }
}
