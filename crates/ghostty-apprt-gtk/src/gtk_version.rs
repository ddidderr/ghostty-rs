//! GTK version helpers (port of `apprt/gtk/gtk_version.zig`).
//!
//! Zig reads the compile-time version from the headers; here it is the
//! version of the gtk4-rs `v4_*` feature level this crate is compiled against
//! (see `Cargo.toml`) combined with the system library detected at runtime.

/// A (major, minor, micro) version triple.
pub type Version = (u32, u32, u32);

/// The minimum GTK version this crate is compiled against (feature level
/// `v4_14`). Newer symbols must be guarded with [`at_least`]/feature flags.
pub const COMPTIME_VERSION: Version = (4, 14, 0);

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
