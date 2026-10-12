/// Returns true if the program was launched from a desktop environment.
///
/// On Linux GTK, this returns true if the program was launched using the
/// desktop file. This also includes when `gtk-launch` is used because I
/// can't find a way to distinguish the two scenarios.
#[cfg(target_os = "macos")]
pub fn launched_from_desktop() -> bool {
    // macOS apps launched from finder or `open` always have the init
    // process as their parent.
    macos_launched_from_desktop(false, std::env::var_os(MAC_LAUNCH_SOURCE_ENV), || {
        rustix::process::getppid().map_or(0, rustix::process::Pid::as_raw_pid)
    })
}

/// iPhone/iPad is always launched from the "desktop".
#[cfg(target_os = "ios")]
pub fn launched_from_desktop() -> bool {
    true
}

/// See the other variants.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub fn launched_from_desktop() -> bool {
    // On Linux and BSD, GTK sets GIO_LAUNCHED_DESKTOP_FILE and
    // GIO_LAUNCHED_DESKTOP_FILE_PID. We only check the latter to see if
    // we match the PID and assume that if we do, we were launched from
    // the desktop file. Pid comparing catches the scenario where
    // another terminal was launched from a desktop file and then launches
    // Ghostty and Ghostty inherits the env.
    let Some(gio_pid_str) = std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE_PID") else {
        return false;
    };

    // (`std::process::id` is `getpid` / `GetCurrentProcessId`.)
    let pid = std::process::id();
    let Ok(gio_pid) = gio_pid_str.to_string_lossy().parse::<u32>() else {
        return false;
    };

    gio_pid == pid
}

/// Environment variable set by the macOS app to tell libghostty how it was
/// launched ("app", "cli" or "zig_run").
pub const MAC_LAUNCH_SOURCE_ENV: &str = "GHOSTTY_MAC_LAUNCH_SOURCE";

/// `launched_from_desktop` on macOS. `artifact_is_lib` mirrors
/// `build_config.artifact == .lib`: when embedded as a library, the app
/// bundle may tell us via [`MAC_LAUNCH_SOURCE_ENV`] that it launched us even
/// though our parent isn't `launchd`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn macos_launched_from_desktop(
    artifact_is_lib: bool,
    source: Option<std::ffi::OsString>,
    getppid: impl FnOnce() -> i32,
) -> bool {
    if artifact_is_lib && let Some(source) = source {
        // Source can be "app", "cli", or "zig_run". We assume its the
        // desktop only if its "app".
        if source == "app" {
            return true;
        }
    }

    getppid() == 1
}

/// The libghostty variant of [`launched_from_desktop`] on macOS (honours the
/// `GHOSTTY_MAC_LAUNCH_SOURCE` environment variable).
#[cfg(target_os = "macos")]
pub fn launched_from_desktop_lib() -> bool {
    macos_launched_from_desktop(true, std::env::var_os(MAC_LAUNCH_SOURCE_ENV), || {
        rustix::process::getppid().map_or(0, rustix::process::Pid::as_raw_pid)
    })
}

/// The list of desktop environments that we detect. New Linux desktop
/// environments should only be added to this list if there's a specific reason
/// to differentiate between `gnome` and `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Gnome,
    Macos,
    Other,
    Windows,
}

/// Detect what desktop environment we are running under. This is mainly used
/// on Linux and BSD to enable or disable certain features but there may be more uses in
/// the future.
pub fn desktop_environment() -> DesktopEnvironment {
    if cfg!(windows) {
        return DesktopEnvironment::Windows;
    }
    if cfg!(target_os = "macos") {
        return DesktopEnvironment::Macos;
    }

    // Use $XDG_SESSION_DESKTOP to determine what DE we are using on Linux
    // https://www.freedesktop.org/software/systemd/man/latest/pam_systemd.html#desktop=
    if let Some(sd) = std::env::var_os("XDG_SESSION_DESKTOP") {
        let sd = sd.to_string_lossy();
        if sd.eq_ignore_ascii_case("gnome") {
            return DesktopEnvironment::Gnome;
        }
        if sd.eq_ignore_ascii_case("gnome-xorg") {
            return DesktopEnvironment::Gnome;
        }
    }

    // If $XDG_SESSION_DESKTOP is not set, or doesn't match any known
    // DE, check $XDG_CURRENT_DESKTOP. $XDG_CURRENT_DESKTOP is a
    // colon-separated list of up to three desktop names, although we
    // only look at the first.
    // https://specifications.freedesktop.org/desktop-entry-spec/latest/recognized-keys.html
    if let Some(cd) = std::env::var_os("XDG_CURRENT_DESKTOP") {
        let cd = cd.to_string_lossy();
        let first = cd.split(':').next().unwrap_or("");
        if first.eq_ignore_ascii_case("gnome") {
            return DesktopEnvironment::Gnome;
        }
    }

    DesktopEnvironment::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[allow(dead_code)]
    fn de(session: Option<&str>, current: Option<&str>) -> DesktopEnvironment {
        temp_env::with_vars(
            [
                ("XDG_SESSION_DESKTOP", session),
                ("XDG_CURRENT_DESKTOP", current),
            ],
            desktop_environment,
        )
    }

    #[cfg(unix)]
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn desktop_environment_detection() {
        use DesktopEnvironment::{Gnome, Other};
        assert_eq!(de(Some("gnome"), None), Gnome);
        assert_eq!(de(Some("gnome-xorg"), None), Gnome);
        assert_eq!(de(Some("foobar"), None), Other);
        assert_eq!(de(None, None), Other);
        assert_eq!(de(None, Some("GNOME")), Gnome);
        assert_eq!(de(None, Some("FOOBAR")), Other);
        assert_eq!(de(None, Some("GNOME:Unity")), Gnome);
        assert_eq!(de(None, Some("Unity:GNOME")), Other);
        assert_eq!(de(Some("foobar"), Some("GNOME")), Gnome);
    }

    #[test]
    fn macos_launch_detection() {
        use std::ffi::OsString;
        // Parent is launchd.
        assert!(macos_launched_from_desktop(false, None, || 1));
        assert!(!macos_launched_from_desktop(false, None, || 4242));
        // The env var only counts for the lib artifact and only for "app".
        let app = Some(OsString::from("app"));
        assert!(macos_launched_from_desktop(true, app.clone(), || 4242));
        assert!(!macos_launched_from_desktop(false, app, || 4242));
        assert!(!macos_launched_from_desktop(
            true,
            Some(OsString::from("cli")),
            || 4242
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn launched_from_desktop_pid_match() {
        let pid = std::process::id().to_string();
        temp_env::with_var("GIO_LAUNCHED_DESKTOP_FILE_PID", Some(pid), || {
            assert!(launched_from_desktop());
        });
        temp_env::with_var("GIO_LAUNCHED_DESKTOP_FILE_PID", Some("1"), || {
            assert!(!launched_from_desktop());
        });
        temp_env::with_var("GIO_LAUNCHED_DESKTOP_FILE_PID", Some("abc"), || {
            assert!(!launched_from_desktop());
        });
        temp_env::with_var("GIO_LAUNCHED_DESKTOP_FILE_PID", None::<&str>, || {
            assert!(!launched_from_desktop());
        });
    }
}
