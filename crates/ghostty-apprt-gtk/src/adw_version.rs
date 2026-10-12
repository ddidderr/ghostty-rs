//! libadwaita version helpers (port of `apprt/gtk/adw_version.zig`).

use crate::gtk_version::Version;

/// The libadwaita version this crate is compiled against (pkg-config
/// `libadwaita-1`, see `build.rs`).
pub const COMPTIME_VERSION: Version =
    crate::gtk_version::parse_version(env!("GHOSTTY_ADW_VERSION"));

/// Returns the runtime libadwaita version.
#[allow(unsafe_code)]
pub fn get_runtime_version() -> Version {
    #[cfg(debug_assertions)]
    if let Some(v) = crate::fake_version::get("GHOSTTY_DEBUG_FAKE_ADW_VERSION") {
        return v;
    }
    // The safe `adw::major_version()` etc. assert that libadwaita was
    // initialized, but the C functions don't need it (and `+version` has no
    // display to initialize GTK with).
    // SAFETY: these functions only return compile-time constants of the
    // loaded library and have no preconditions.
    unsafe {
        (
            adw::ffi::adw_get_major_version(),
            adw::ffi::adw_get_minor_version(),
            adw::ffi::adw_get_micro_version(),
        )
    }
}

/// Logs the build and runtime libadwaita version.
pub fn log_version() {
    let (a, b, c) = COMPTIME_VERSION;
    let (x, y, z) = get_runtime_version();
    log::info!("libadwaita version build={a}.{b}.{c} runtime={x}.{y}.{z}");
}

/// The libadwaita API level this build uses, i.e. the highest `adw_1_*`
/// cargo feature (the equivalent of the version macros in `adwaita.h` that
/// Zig's `comptime_version` reads).
pub const BUILD_LEVEL: Version = if cfg!(feature = "adw_1_5") {
    (1, 5, 0)
} else if cfg!(feature = "adw_1_4") {
    (1, 4, 0)
} else if cfg!(feature = "adw_1_3") {
    (1, 3, 0)
} else {
    (1, 2, 0)
};

/// Compile-time half of [`at_least`]: whether this build was compiled with at
/// least the given libadwaita API level.
#[must_use]
pub const fn comptime_at_least(major: u32, minor: u32, micro: u32) -> bool {
    let (a, b, c) = BUILD_LEVEL;
    a > major || (a == major && (b > minor || (b == minor && c >= micro)))
}

/// Verifies that the running libadwaita version is at least the given
/// version, for both the build level and the library we are running against
/// (so generally you want both checks, like Zig's `atLeast`).
#[inline]
pub fn at_least(major: u32, minor: u32, micro: u32) -> bool {
    // A lower build level means we must not call newer symbols at all.
    if !comptime_at_least(major, minor, micro) {
        return false;
    }
    runtime_at_least(major, minor, micro)
}

/// Verifies that the libadwaita runtime version is at least the given version.
pub fn runtime_at_least(major: u32, minor: u32, micro: u32) -> bool {
    get_runtime_version() >= (major, minor, micro)
}

/// Whether AdwDialog, AdwAlertDialog, etc. are supported (1.5+).
pub fn supports_dialogs() -> bool {
    at_least(1, 5, 0)
}

pub fn supports_tab_overview() -> bool {
    at_least(1, 4, 0)
}

pub fn supports_switch_row() -> bool {
    at_least(1, 4, 0)
}

pub fn supports_toolbar_view() -> bool {
    at_least(1, 4, 0)
}

pub fn supports_banner() -> bool {
    at_least(1, 3, 0)
}

/// Compile-time (build level only) variants of the `supports_*` checks. Zig
/// uses `supportsX()` at comptime to choose a class's parent type; Rust
/// picks types with the matching `cfg(feature = "adw_1_*")` and these const
/// functions document/assert the same decision.
pub const COMPTIME_SUPPORTS_DIALOGS: bool = comptime_at_least(1, 5, 0);
pub const COMPTIME_SUPPORTS_SWITCH_ROW: bool = comptime_at_least(1, 4, 0);
pub const COMPTIME_SUPPORTS_BANNER: bool = comptime_at_least(1, 3, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_at_least() {
        // libadwaita requires GTK init, which only works on the main thread
        // with a display; skip otherwise.
        if adw::init().is_err() {
            return;
        }
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
    fn version_at_least_build_level() {
        let (a, b, c) = BUILD_LEVEL;
        assert!(at_least(a, b, c) || get_runtime_version() < BUILD_LEVEL);
        assert!(!at_least(a, b, c + 1));
        assert!(!at_least(a, b + 1, c));
        assert!(!at_least(a + 1, b, c));
        assert!(comptime_at_least(a, b, c));
        assert!(!comptime_at_least(a, b, c + 1));
        assert_eq!(COMPTIME_SUPPORTS_DIALOGS, cfg!(feature = "adw_1_5"));
        assert_eq!(COMPTIME_SUPPORTS_SWITCH_ROW, cfg!(feature = "adw_1_4"));
        assert_eq!(COMPTIME_SUPPORTS_BANNER, cfg!(feature = "adw_1_3"));
    }
}
