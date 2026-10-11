/// Returns true if the program was launched from a desktop environment.
///
/// On Linux GTK, this returns true if the program was launched using the
/// desktop file. This also includes when `gtk-launch` is used because I
/// can't find a way to distinguish the two scenarios.
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

    let pid = rustix::process::getpid().as_raw_nonzero().get();
    let Ok(gio_pid) = gio_pid_str.to_string_lossy().parse::<i32>() else {
        return false;
    };

    gio_pid == pid
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

    fn de(session: Option<&str>, current: Option<&str>) -> DesktopEnvironment {
        temp_env::with_vars(
            [
                ("XDG_SESSION_DESKTOP", session),
                ("XDG_CURRENT_DESKTOP", current),
            ],
            desktop_environment,
        )
    }

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
