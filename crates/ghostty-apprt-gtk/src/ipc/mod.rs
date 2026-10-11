//! IPC to a running instance over D-Bus (`apprt/gtk/ipc/*.zig` and
//! `App.performIpc`).

pub mod dbus;
pub mod new_window;

use ghostty_apprt::ipc::{Action, Error, Target};

/// Send the given IPC to a running Ghostty. Returns `Ok(true)` if the action
/// was performed. This is a free function: it's called from a CLI process
/// without a full-featured app (`App.performIpc`).
pub fn perform_ipc(target: &Target, action: &Action) -> Result<bool, Error> {
    match action {
        Action::NewWindow(v) => new_window::new_window(target, v),
    }
}
