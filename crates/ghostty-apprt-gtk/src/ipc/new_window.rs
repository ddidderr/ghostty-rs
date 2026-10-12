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
        dbus.add_parameter(&arguments_variant(arguments));
    }

    dbus.send()?;
    Ok(true)
}

/// Zig constructs each GVariant string from serialized bytes including its NUL.
/// Keep valid wire bytes unchanged. Zig marks these strings trusted, causing
/// GIO to abort when serializing invalid UTF-8 or an interior NUL. Fail the same
/// way explicitly: an untrusted GVariant would silently substitute an empty
/// string, while a malformed trusted variant would violate Rust GLib's contract.
fn arguments_variant(arguments: &[Vec<u8>]) -> glib::Variant {
    glib::Variant::array_from_iter_with_type(
        glib::VariantTy::STRING,
        arguments.iter().map(|arg| {
            if arg.contains(&0) || std::str::from_utf8(arg).is_err() {
                log::error!("invalid D-Bus string in new-window arguments");
                std::process::abort();
            }
            let mut bytes = arg.clone();
            bytes.push(0);
            glib::Variant::from_bytes_with_type(
                &glib::Bytes::from_owned(bytes),
                glib::VariantTy::STRING,
            )
        }),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[cfg(unix)]
    #[test]
    fn invalid_dbus_strings_fail_without_substitution() {
        use std::os::unix::process::ExitStatusExt;
        const CHILD: &str = "GHOSTTY_TEST_INVALID_DBUS_VALUE";
        if let Ok(kind) = std::env::var(CHILD) {
            let value: &[u8] = if kind == "utf8" {
                b"--title=bad\xff"
            } else {
                b"--title=bad\0tail"
            };
            arguments_variant(&[value.to_vec()]);
            panic!("invalid D-Bus argument must abort like Zig/GIO");
        }
        for kind in ["utf8", "nul"] {
            // Disable core dumps in the child without changing this test process.
            let result = std::process::Command::new("/bin/sh")
                .args(["-c", "ulimit -c 0; exec \"$@\"", "ghostty-dbus-test"])
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ipc::new_window::tests::invalid_dbus_strings_fail_without_substitution",
                ])
                .env(CHILD, kind)
                .output()
                .unwrap();
            assert_eq!(result.status.signal(), Some(libc::SIGABRT));
        }
    }

    #[test]
    fn arguments_are_plain_dbus_strings_not_escape_encoded() {
        let argument = "--working-directory=/tmp/\u{fdd0}é";
        let variant = arguments_variant(&[argument.as_bytes().to_vec()]);
        assert_eq!(variant.get::<Vec<String>>().unwrap(), [argument]);
        assert_eq!(
            variant.child_value(0).data(),
            [argument.as_bytes(), b"\0"].concat()
        );
    }
}
