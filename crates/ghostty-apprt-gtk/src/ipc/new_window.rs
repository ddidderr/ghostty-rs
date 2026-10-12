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
use gtk::glib;

use super::dbus::DBus;

pub fn new_window(target: &Target, value: &NewWindow) -> Result<bool, Error> {
    // Validate before touching the bus (deviant fix, see `arguments_variant`).
    let command = value
        .arguments
        .as_deref()
        .map(arguments_variant)
        .transpose()?;
    let mut dbus = DBus::new(
        target,
        if value.arguments.is_none() {
            "new-window"
        } else {
            "new-window-command"
        },
    )?;

    if let Some(command) = &command {
        // If any arguments were specified on the command line, the first
        // parameter is an array of strings that contain the arguments. They
        // will be sent to the main Ghostty instance and interpreted as CLI
        // arguments.
        dbus.add_parameter(command);
    }

    dbus.send()?;
    Ok(true)
}

/// Zig constructs each GVariant string from serialized bytes including its NUL
/// and marks it trusted (`new_window.zig:49-53`), so GIO aborts the process when
/// serializing invalid UTF-8 or an interior NUL (input-triggered crash).
/// DEVIANT FIX: reject such arguments gracefully instead (see
/// DEVIANT_FIXES.md); valid wire bytes are unchanged.
fn arguments_variant(arguments: &[Vec<u8>]) -> Result<glib::Variant, Error> {
    if let Some(bad) = arguments
        .iter()
        .find(|arg| arg.contains(&0) || std::str::from_utf8(arg).is_err())
    {
        log::error!("invalid D-Bus string in new-window arguments");
        eprintln!(
            "new-window argument is not valid UTF-8 or contains a NUL byte: {}",
            String::from_utf8_lossy(bad).escape_debug()
        );
        return Err(Error::IpcFailed);
    }
    Ok(glib::Variant::array_from_iter_with_type(
        glib::VariantTy::STRING,
        arguments.iter().map(|arg| {
            let mut bytes = arg.clone();
            bytes.push(0);
            glib::Variant::from_bytes_with_type(
                &glib::Bytes::from_owned(bytes),
                glib::VariantTy::STRING,
            )
        }),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn invalid_dbus_strings_are_rejected_without_abort() {
        for value in [&b"--title=bad\xff"[..], &b"--title=bad\0tail"[..]] {
            let result = arguments_variant(&[b"ok".to_vec(), value.to_vec()]);
            assert!(matches!(result, Err(Error::IpcFailed)));
        }
        assert!(arguments_variant(&[b"ok".to_vec()]).is_ok());
    }

    #[test]
    fn arguments_are_plain_dbus_strings_not_escape_encoded() {
        let argument = "--working-directory=/tmp/\u{fdd0}é";
        let variant = arguments_variant(&[argument.as_bytes().to_vec()]).unwrap();
        assert_eq!(variant.get::<Vec<String>>().unwrap(), [argument]);
        assert_eq!(
            variant.child_value(0).data(),
            [argument.as_bytes(), b"\0"].concat()
        );
    }
}
