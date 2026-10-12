//! Transient systemd scope creation over D-Bus (port of `apprt/gtk/cgroup.zig`).

use gtk::{gio, glib, prelude::*};

/// Properties for the transient scope.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// `MemoryHigh` in bytes.
    pub memory_high: Option<u64>,
    /// `TasksMax`.
    pub tasks_max: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("D-Bus call failed")]
    DbusCallFailed,
}

/// The scope unit name for the given PID.
pub fn fmt_scope(pid: u32) -> String {
    format!("app-ghostty-surface-transient-{pid}.scope")
}

/// Create a transient systemd scope containing `pid` via
/// `org.freedesktop.systemd1.Manager.StartTransientUnit`.
pub fn create_scope(dbus: &gio::DBusConnection, pid: u32, options: Options) -> Result<(), Error> {
    // The unit name needs to be unique. We use the PID for this.
    let name = fmt_scope(pid);

    // Properties
    let mut properties: Vec<(String, glib::Variant)> = Vec::new();
    if let Some(v) = options.memory_high {
        properties.push(("MemoryHigh".into(), v.to_variant()));
    }
    if let Some(v) = options.tasks_max {
        properties.push(("TasksMax".into(), v.to_variant()));
    }
    // https://www.freedesktop.org/software/systemd/man/latest/systemd-oomd.service.html
    properties.push(("ManagedOOMMemoryPressure".into(), "kill".to_variant()));
    // PID to move into the unit
    properties.push(("PIDs".into(), vec![pid].to_variant()));

    // Aux - unused but must be present
    let aux: Vec<(String, Vec<(String, glib::Variant)>)> = Vec::new();

    let params = (name, "fail".to_owned(), properties, aux).to_variant();

    dbus.call_sync(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "StartTransientUnit",
        Some(&params),
        Some(glib::VariantTy::new("(o)").map_err(|_| Error::DbusCallFailed)?),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
    .map_err(|e| {
        log::error!("creating transient cgroup scope failed err={e}");
        Error::DbusCallFailed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_scope_test() {
        assert_eq!(fmt_scope(42), "app-ghostty-surface-transient-42.scope");
        assert!(fmt_scope(u32::MAX).len() < 256);
    }

    #[test]
    fn scope_params_signature() {
        let props: Vec<(String, glib::Variant)> = vec![("PIDs".into(), vec![1u32].to_variant())];
        let aux: Vec<(String, Vec<(String, glib::Variant)>)> = Vec::new();
        let v = ("n".to_owned(), "fail".to_owned(), props, aux).to_variant();
        assert_eq!(v.type_().as_str(), "(ssa(sv)a(sa(sv)))");
    }
}
