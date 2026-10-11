//! libadwaita version helpers (port of `apprt/gtk/adw_version.zig`).

use crate::gtk_version::Version;

/// The libadwaita feature level this crate is compiled against (`v1_5`).
pub const COMPTIME_VERSION: Version = (1, 5, 0);

/// Returns the runtime libadwaita version.
#[allow(unsafe_code)]
pub fn get_runtime_version() -> Version {
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

/// Verifies that the libadwaita version at runtime is at least the given
/// version (callers needing newer API must also gate on a cargo feature).
pub fn at_least(major: u32, minor: u32, micro: u32) -> bool {
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
}
