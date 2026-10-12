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

/// The GTK API level this build uses, i.e. the highest `gtk_4_*` cargo
/// feature (the equivalent of the version macros in the C headers that Zig's
/// `comptime_version` reads). Code calling newer symbols must be gated with
/// the matching `cfg(feature = ...)` as well as [`at_least`].
pub const BUILD_LEVEL: Version = if cfg!(feature = "gtk_4_16") {
    (4, 16, 0)
} else {
    (4, 14, 0)
};

/// Returns the runtime GTK version.
pub fn get_runtime_version() -> Version {
    #[cfg(debug_assertions)]
    if let Some(v) = crate::fake_version::get("GHOSTTY_DEBUG_FAKE_GTK_VERSION") {
        return v;
    }
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

/// Compile-time half of [`at_least`]: whether this build was compiled with
/// at least the given GTK API level. `const` so that, like Zig's `comptime`
/// check, it folds away the runtime check when it fails.
#[must_use]
pub const fn comptime_at_least(major: u32, minor: u32, micro: u32) -> bool {
    let (a, b, c) = BUILD_LEVEL;
    a > major || (a == major && (b > minor || (b == minor && c >= micro)))
}

/// Verifies that the GTK version is at least the given version.
///
/// Both the build level ([`comptime_at_least`]) and the library we are
/// running against must satisfy it. Use this where the check affects which
/// symbols are called; for checks that only depend on GTK's runtime behavior
/// use [`runtime_at_least`].
#[inline]
pub fn at_least(major: u32, minor: u32, micro: u32) -> bool {
    // If our build level is lower than the given version we return false
    // immediately; this prevents us from calling unknown symbols.
    if !comptime_at_least(major, minor, micro) {
        return false;
    }
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
        {
            let f = runtime_at_least;
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
    fn at_least_build_level_test() {
        // Zig's `atLeast` test, against the build level (our "headers").
        let (a, b, c) = BUILD_LEVEL;
        assert!(at_least(a, b, c));
        assert!(!at_least(a, b, c + 1));
        assert!(!at_least(a, b + 1, c));
        assert!(!at_least(a + 1, b, c));
        assert!(at_least(a - 1, b, c));
        assert!(at_least(a - 1, b + 1, c));
        assert!(at_least(a - 1, b, c + 1));
        assert!(at_least(a, b - 1, c + 1));
        assert!(comptime_at_least(a, b, c));
        assert!(!comptime_at_least(a, b, c + 1));
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
