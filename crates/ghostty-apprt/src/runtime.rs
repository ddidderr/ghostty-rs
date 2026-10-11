//! Port of `runtime.zig` (and `none.zig`).

/// Runtime is the runtime to use for Ghostty. All runtimes do not provide
/// equivalent feature sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// Will not produce an executable at all (lib only). Matches `none.zig`:
    /// there is no apprt to communicate with, so IPC always returns `false`.
    None,

    /// GTK4. Rich windowed application. This uses a full GObject-based
    /// approach to building the application.
    Gtk,
}

impl Runtime {
    /// The default runtime for the current target. The Linux and FreeBSD
    /// default is GTK because it is a full featured application; otherwise
    /// `None`.
    #[must_use]
    pub fn default_for_target() -> Self {
        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            Runtime::Gtk
        } else {
            Runtime::None
        }
    }
}

/// `none.zig` `App::performIpc`: always false as there is no apprt to
/// communicate with.
///
/// # Errors
/// Never fails.
pub fn none_perform_ipc(
    _target: &crate::ipc::Target,
    _action: &crate::ipc::Action,
) -> Result<bool, crate::ipc::Error> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_is_gtk_on_linux() {
        if cfg!(target_os = "linux") {
            assert_eq!(Runtime::default_for_target(), Runtime::Gtk);
        }
    }

    #[test]
    fn none_ipc_returns_false() {
        let r = none_perform_ipc(
            &crate::ipc::Target::Detect,
            &crate::ipc::Action::NewWindow(crate::ipc::NewWindow { arguments: None }),
        );
        assert!(matches!(r, Ok(false)));
    }
}
