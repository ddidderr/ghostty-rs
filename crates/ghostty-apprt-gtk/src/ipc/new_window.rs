//! `+new-window` IPC (port of `apprt/gtk/ipc/new_window.zig`).
//!
//! Use a D-Bus method call to open a new window on GTK. See
//! <https://wiki.gnome.org/Projects/GLib/GApplication/DBusAPI>.
//!
//! `ghostty +new-window` is equivalent to (release build):
//!
//! ```text
//! gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty --method org.gtk.Actions.Activate new-window [] []
//! ```
//!
//! `ghostty +new-window -e echo hello` is equivalent to:
//!
//! ```text
//! gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty --method org.gtk.Actions.Activate new-window-command '[<@as ["-e" "echo" "hello"]>]' []
//! ```

use ghostty_apprt::ipc::{Error, NewWindow, Target};
use gtk::prelude::*;

use super::dbus::DBus;

pub fn new_window(target: &Target, value: &NewWindow) -> Result<bool, Error> {
    let mut dbus = DBus::new(
        target,
        if value.arguments.is_none() {
            "new-window"
        } else {
            "new-window-command"
        },
    )?;

    if let Some(arguments) = &value.arguments {
        // If any arguments were specified on the command line, the first
        // parameter is an array of strings that contain the arguments. They
        // will be sent to the main Ghostty instance and interpreted as CLI
        // arguments.
        dbus.add_parameter(&arguments.to_variant());
    }

    dbus.send()?;
    Ok(true)
}
