/// Returns true if the program was launched by D-Bus activation.
///
/// On Linux GTK, this returns true if the program was launched using D-Bus
/// activation. It will return false if Ghostty was launched any other way.
pub fn launched_by_dbus_activation() -> bool {
    // On Linux, D-Bus activation sets `DBUS_STARTER_ADDRESS` and
    // `DBUS_STARTER_BUS_TYPE`. If these environment variables are present
    // (no matter the value) we were launched by D-Bus activation.
    std::env::var_os("DBUS_STARTER_ADDRESS").is_some()
        && std::env::var_os("DBUS_STARTER_BUS_TYPE").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::with_env_lock;

    #[test]
    fn dbus_activation_env() {
        temp_env::with_vars(
            [
                ("DBUS_STARTER_ADDRESS", Some("x")),
                ("DBUS_STARTER_BUS_TYPE", Some("session")),
            ],
            || assert!(launched_by_dbus_activation()),
        );
        temp_env::with_vars(
            [
                ("DBUS_STARTER_ADDRESS", Some("x")),
                ("DBUS_STARTER_BUS_TYPE", None),
            ],
            || assert!(!launched_by_dbus_activation()),
        );
        with_env_lock(|| {});
    }
}
