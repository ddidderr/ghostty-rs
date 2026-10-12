//! D-Bus helper for IPC (port of `apprt/gtk/ipc/DBus.zig`).

use ghostty_apprt::ipc::{Error, Target};
use gtk::{gio, glib, prelude::*};

/// A pending `org.gtk.Actions.Activate` call.
pub struct DBus {
    dbus: gio::DBusConnection,
    bus_name: String,
    object_path: String,
    action: String,
    parameters: Vec<glib::Variant>,
}

/// Bus name and object path for a target.
pub fn bus_name_and_path(target: &Target) -> (String, String) {
    match target {
        // Force the usage of the class specified on the CLI to determine the
        // bus name and object path.
        Target::Class(class) => (
            class.clone(),
            format!("/{class}").replace('.', "/").replace('-', "_"),
        ),
        Target::Detect => (
            crate::APPLICATION_ID.to_owned(),
            crate::OBJECT_PATH.to_owned(),
        ),
    }
}

impl DBus {
    pub fn new(target: &Target, action: &str) -> Result<Self, Error> {
        let (bus_name, object_path) = bus_name_and_path(target);

        if !gio::Application::id_is_valid(&bus_name) {
            eprintln!("D-Bus bus name is not valid: {bus_name}");
            return Err(Error::IpcFailed);
        }

        if !glib::Variant::is_object_path(&object_path) {
            eprintln!("D-Bus object path is not valid: {object_path}");
            return Err(Error::IpcFailed);
        }

        // Get a connection to the DBus session bus.
        let dbus =
            gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| {
                eprintln!("Unable to establish connection to D-Bus session bus: {e}");
                Error::IpcFailed
            })?;

        Ok(Self {
            dbus,
            bus_name,
            object_path,
            action: action.to_owned(),
            parameters: Vec::new(),
        })
    }

    pub fn add_parameter(&mut self, variant: &glib::Variant) {
        self.parameters.push(variant.clone());
    }

    /// The `(sava{sv})` payload (action, parameters, platform data).
    pub fn payload(&self) -> glib::Variant {
        // We have no platform data.
        let platform: std::collections::HashMap<String, glib::Variant> =
            std::collections::HashMap::new();
        (self.action.clone(), self.parameters.clone(), platform).to_variant()
    }

    pub fn send(&self) -> Result<(), Error> {
        let payload = self.payload();
        self.dbus
            .call_sync(
                Some(&self.bus_name),
                &self.object_path,
                "org.gtk.Actions",
                "Activate",
                Some(&payload),
                None, // we don't do anything with the return value
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            )
            .map(|_| ())
            .map_err(|e| {
                eprintln!("D-Bus method call returned an error err={e}");
                Error::IpcFailed
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_bus_name_and_path() {
        let (n, p) = bus_name_and_path(&Target::Class("com.example.my-app".into()));
        assert_eq!(n, "com.example.my-app");
        assert_eq!(p, "/com/example/my_app");
    }

    #[test]
    fn detect_bus_name_and_path() {
        let (n, p) = bus_name_and_path(&Target::Detect);
        assert_eq!(n, crate::APPLICATION_ID);
        assert_eq!(p, crate::OBJECT_PATH);
    }
}
