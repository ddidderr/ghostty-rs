//! `GhosttyGlobalShortcuts` (ZIG/src/apprt/gtk/class/global_shortcuts.zig):
//! registers `global:` keybinds with the XDG GlobalShortcuts portal over
//! D-Bus and emits `trigger` when one is activated.
//!
//! The `trigger` signal carries the binding action serialized in its keybind
//! syntax (a `String`); use [`GlobalShortcuts::connect_trigger`] to receive
//! the parsed [`ghostty_input::binding::Action`].

#![allow(deprecated)] // gio D-Bus signal_subscribe: replacement API is awkward for portal use

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::OnceLock,
};

use ghostty_input::binding::{Action as BindingAction, set::Value as BindingValue};
use gtk::{
    gio::{self, prelude::*},
    glib::{
        self,
        subclass::{Signal, prelude::*},
        variant::ObjectPath,
    },
};

use crate::class::config::Config;

mod imp {
    use glib::{prelude::*, subclass::prelude::*};

    use super::{BindingAction, Cell, Config, OnceLock, RefCell, Signal, gio, glib};

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::GlobalShortcuts)]
    pub struct GlobalShortcuts {
        /// The configuration that this is using.
        #[property(get, set = Self::set_config, explicit_notify, nullable)]
        pub config: RefCell<Option<Config>>,

        /// The dbus connection.
        #[property(get, set = Self::set_dbus_connection, explicit_notify, nullable)]
        pub dbus_connection: RefCell<Option<gio::DBusConnection>>,

        /// Unique ID (the xdg shortcut string) -> action.
        pub map: RefCell<Vec<(String, BindingAction)>>,

        /// The current portal session handle (D-Bus object path).
        pub handle: RefCell<Option<String>>,

        pub response_subscription: RefCell<Option<gio::SignalSubscriptionId>>,
        pub activate_subscription: RefCell<Option<gio::SignalSubscriptionId>>,

        /// Counter making sure async callbacks of stale sessions are ignored.
        pub generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlobalShortcuts {
        const NAME: &'static str = "GhosttyGlobalShortcuts";
        type Type = super::GlobalShortcuts;
        type ParentType = glib::Object;
    }

    impl GlobalShortcuts {
        fn set_config(&self, config: Option<Config>) {
            *self.config.borrow_mut() = config;
            self.obj().notify_config();
            // propConfig
            if let Err(err) = self.obj().refresh() {
                log::warn!("error refreshing global shortcuts: {err}");
            }
        }

        fn set_dbus_connection(&self, conn: Option<gio::DBusConnection>) {
            let obj = self.obj();
            // If we have a prior dbus connection we need to close our prior
            // registrations first.
            if self.dbus_connection.borrow().is_some() {
                obj.close();
                *self.dbus_connection.borrow_mut() = None;
            }
            let has = conn.is_some();
            *self.dbus_connection.borrow_mut() = conn;
            if has && let Err(err) = obj.refresh() {
                log::warn!("error refreshing global shortcuts: {err}");
            }
            obj.notify_dbus_connection();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for GlobalShortcuts {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted whenever a global shortcut is triggered. The
                    // parameter is the action in keybind syntax.
                    Signal::builder("trigger")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn dispose(&self) {
            // Since we drop references here we may lose access to things like
            // dbus connections, so we need to close all our connections right
            // away instead of in finalize.
            self.obj().close();
            *self.config.borrow_mut() = None;
            *self.dbus_connection.borrow_mut() = None;
        }
    }
}

glib::wrapper! {
    pub struct GlobalShortcuts(ObjectSubclass<imp::GlobalShortcuts>);
}

impl Default for GlobalShortcuts {
    fn default() -> Self {
        Self::new()
    }
}

/// The two portal methods we call.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    CreateSession,
    BindShortcuts,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::CreateSession => "CreateSession",
            Method::BindShortcuts => "BindShortcuts",
        }
    }
}

type Token = String;

/// Generate a random token suitable for use in requests
/// (`ghostty_` + 7 hex digits).
fn generate_token() -> Token {
    format!("ghostty_{:07x}", glib::random_int_range(0, 1 << 28))
}

fn vardict(entries: Vec<(&str, glib::Variant)>) -> glib::Variant {
    let map: HashMap<String, glib::Variant> = entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    map.to_variant()
}

/// `(a{sv})`: the `CreateSession` arguments.
fn create_session_payload(handle_token: &str, session_handle_token: &str) -> glib::Variant {
    let opts = vardict(vec![
        ("handle_token", handle_token.to_variant()),
        ("session_handle_token", session_handle_token.to_variant()),
    ]);
    glib::Variant::tuple_from_iter([opts])
}

/// `(oa(sa{sv})sa{sv})`: the `BindShortcuts` arguments (session handle,
/// shortcuts, parent window, options).
fn bind_shortcuts_payload<'a>(
    handle: &str,
    shortcuts: impl Iterator<Item = (&'a str, String)>,
    request_token: &str,
) -> Option<glib::Variant> {
    let handle = ObjectPath::try_from(handle.to_owned()).ok()?;
    let binds = glib::Variant::array_from_iter_with_type(
        glib::VariantTy::new("(sa{sv})").ok()?,
        shortcuts.map(|(trigger, description)| {
            let opts = vardict(vec![
                ("description", description.to_variant()),
                ("preferred_trigger", trigger.to_variant()),
            ]);
            glib::Variant::tuple_from_iter([trigger.to_variant(), opts])
        }),
    );
    let opts = vardict(vec![("handle_token", request_token.to_variant())]);
    Some(glib::Variant::tuple_from_iter([
        handle.to_variant(),
        binds,
        String::new().to_variant(),
        opts,
    ]))
}

impl GlobalShortcuts {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the D-Bus connection (`setDbusConnection`).
    pub fn set_connection(&self, conn: Option<&gio::DBusConnection>) {
        self.set_dbus_connection(conn);
    }

    /// Connect to the `trigger` signal.
    pub fn connect_trigger<F: Fn(&Self, &BindingAction) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "trigger",
            false,
            glib::closure_local!(move |this: &Self, action: String| {
                match BindingAction::parse(&action) {
                    Ok(a) => f(this, &a),
                    Err(err) => log::warn!("invalid global shortcut action={action}: {err:?}"),
                }
            }),
        )
    }

    fn close(&self) {
        let imp = self.imp();
        let Some(dbus) = imp.dbus_connection.borrow().clone() else {
            return;
        };

        if let Some(id) = imp.response_subscription.borrow_mut().take() {
            dbus.signal_unsubscribe(id);
        }
        if let Some(id) = imp.activate_subscription.borrow_mut().take() {
            dbus.signal_unsubscribe(id);
        }

        if let Some(handle) = imp.handle.borrow_mut().take() {
            // Close existing session
            dbus.call(
                Some("org.freedesktop.portal.Desktop"),
                &handle,
                "org.freedesktop.portal.Session",
                "Close",
                None,
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
                |_| {},
            );
        }

        imp.generation.set(imp.generation.get() + 1);
        imp.map.borrow_mut().clear();
    }

    #[allow(clippy::unnecessary_wraps)]
    fn refresh(&self) -> Result<(), glib::Error> {
        // Always close our previous state first.
        self.close();

        let imp = self.imp();

        // We need configuration to proceed.
        let Some(config) = imp.config.borrow().as_ref().map(Config::get) else {
            return Ok(());
        };

        // Update map
        let mut map: Vec<(String, BindingAction)> = Vec::new();
        for (trigger, value) in &config.keybind.set.bindings {
            let actions: &[BindingAction] = match value {
                BindingValue::Leader(_) => continue,
                BindingValue::Leaf(l) => {
                    if !l.flags.global {
                        continue;
                    }
                    std::slice::from_ref(&l.action)
                }
                BindingValue::LeafChained(l) => {
                    if !l.flags.global {
                        continue;
                    }
                    &l.actions
                }
            };

            // We only allow global keybinds that map to exactly a single
            // action for now. TODO: remove this restriction
            if actions.len() != 1 {
                continue;
            }

            let Some(shortcut) = crate::key::xdg_shortcut_from_trigger(trigger.0) else {
                continue;
            };

            if let Some(slot) = map.iter_mut().find(|(k, _)| *k == shortcut) {
                slot.1 = actions[0].clone();
            } else {
                map.push((shortcut, actions[0].clone()));
            }
        }

        let has = !map.is_empty();
        *imp.map.borrow_mut() = map;

        // Create our session if we have global shortcuts.
        if has {
            self.request(Method::CreateSession);
        }
        Ok(())
    }

    /// Build the payload for a portal method.
    fn make_payload(&self, method: Method, request_token: &str) -> Option<glib::Variant> {
        match method {
            // https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html#org-freedesktop-portal-globalshortcuts-createsession
            Method::CreateSession => Some(create_session_payload(request_token, &generate_token())),
            // https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html#org-freedesktop-portal-globalshortcuts-bindshortcuts
            Method::BindShortcuts => {
                let imp = self.imp();
                let handle = imp.handle.borrow().clone()?;
                let map = imp.map.borrow();
                bind_shortcuts_payload(
                    &handle,
                    map.iter()
                        .map(|(trigger, action)| (trigger.as_str(), action.to_string())),
                    request_token,
                )
            }
        }
    }

    fn on_response(&self, method: Method, dict: &HashMap<String, glib::Variant>) {
        match method {
            Method::CreateSession => {
                let Some(handle) = dict.get("session_handle").and_then(|v| v.str()) else {
                    log::warn!("session handle not found in response={dict:?}");
                    return;
                };
                let imp = self.imp();
                let Some(dbus) = imp.dbus_connection.borrow().clone() else {
                    return;
                };
                *imp.handle.borrow_mut() = Some(handle.to_owned());
                log::debug!("session_handle={handle}");

                // Subscribe to keybind activations
                let this = self.downgrade();
                let id = dbus.signal_subscribe(
                    None,
                    Some("org.freedesktop.portal.GlobalShortcuts"),
                    Some("Activated"),
                    Some("/org/freedesktop/portal/desktop"),
                    Some(handle),
                    gio::DBusSignalFlags::MATCH_ARG0_PATH,
                    move |_, _, _, _, _, params| {
                        if let Some(this) = this.upgrade() {
                            this.shortcut_activated(params);
                        }
                    },
                );
                *imp.activate_subscription.borrow_mut() = Some(id);

                self.request(Method::BindShortcuts);
            }
            Method::BindShortcuts => {}
        }
    }

    /// Submit a request to the global shortcuts portal.
    ///
    /// XDG portals require subscribing to the Request's `Response` signal at
    /// a manually constructed object path *before* making the call (see the
    /// long rant in the Zig source).
    fn request(&self, method: Method) {
        let imp = self.imp();
        let request_token = generate_token();
        let Some(payload) = self.make_payload(method, &request_token) else {
            return;
        };
        let Some(dbus) = imp.dbus_connection.borrow().clone() else {
            return;
        };
        let Some(request_path) = self.request_path(&dbus, &request_token) else {
            return;
        };

        let generation = imp.generation.get();
        let this = self.downgrade();
        let id = dbus.signal_subscribe(
            None,
            Some("org.freedesktop.portal.Request"),
            Some("Response"),
            Some(&request_path),
            None,
            gio::DBusSignalFlags::NONE,
            move |dbus, _, _, _, _, params| {
                let Some(this) = this.upgrade() else { return };
                let imp = this.imp();
                if imp.generation.get() != generation {
                    return;
                }

                // Unsubscribe from the response signal
                if let Some(id) = imp.response_subscription.borrow_mut().take() {
                    dbus.signal_unsubscribe(id);
                }

                let Some((response, dict)) = params.get::<(u32, HashMap<String, glib::Variant>)>()
                else {
                    log::warn!("unexpected response payload type={}", params.type_());
                    return;
                };
                match response {
                    0 => {
                        log::debug!("request successful");
                        this.on_response(method, &dict);
                    }
                    1 => log::debug!("request was cancelled by user"),
                    2 => log::warn!("request ended unexpectedly"),
                    other => log::warn!("unrecognized response code={other}"),
                }
            },
        );
        // Zig asserts there is no prior subscription.
        if let Some(old) = imp.response_subscription.borrow_mut().replace(id) {
            dbus.signal_unsubscribe(old);
        }

        dbus.call(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.GlobalShortcuts",
            method.name(),
            Some(&payload),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
            |res| {
                if let Err(err) = res {
                    log::warn!("request failed={} ({:?})", err.message(), err);
                }
                // TODO(upstream): XDG recommends updating the signal
                // subscription if the actual returned request path differs.
            },
        );
    }

    /// The XDG portal request path for the current Ghostty instance.
    #[allow(clippy::unused_self)]
    fn request_path(&self, dbus: &gio::DBusConnection, token: &str) -> Option<String> {
        let unique = dbus.unique_name()?;
        let unique = unique.strip_prefix(':').unwrap_or(&unique);
        // Replace every `.` with `_`: `:1.192` -> `1_192`.
        let unique = unique.replace('.', "_");
        Some(format!(
            "/org/freedesktop/portal/desktop/request/{unique}/{token}"
        ))
    }

    fn shortcut_activated(&self, params: &glib::Variant) {
        // 2nd value in the tuple is the activated shortcut ID.
        let child = params.child_value(1);
        let Some(id) = child.str() else { return };
        log::debug!("activated={id}");

        let action = {
            let map = self.imp().map.borrow();
            map.iter().find(|(k, _)| k == id).map(|(_, a)| a.clone())
        };
        let Some(action) = action else { return };
        self.emit_by_name::<()>("trigger", &[&action.to_string()]);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn create_session_payload_signature() {
        let v = create_session_payload("tok", "stok");
        assert_eq!(v.type_().as_str(), "(a{sv})");
        let (m,): (HashMap<String, glib::Variant>,) = v.get().unwrap();
        assert_eq!(m["handle_token"].get::<String>().unwrap(), "tok");
        assert_eq!(m["session_handle_token"].get::<String>().unwrap(), "stok");
    }

    #[test]
    fn bind_shortcuts_payload_signature() {
        let v = bind_shortcuts_payload(
            "/org/freedesktop/portal/desktop/session/1_2/x",
            [("CTRL+ALT+x", "new_window".to_owned())].into_iter(),
            "tok",
        )
        .unwrap();
        assert_eq!(v.type_().as_str(), "(oa(sa{sv})sa{sv})");
        assert_eq!(v.n_children(), 4);
        let binds = v.child_value(1);
        assert_eq!(binds.n_children(), 1);
        let bind = binds.child_value(0);
        assert_eq!(bind.child_value(0).str(), Some("CTRL+ALT+x"));
        let opts: HashMap<String, glib::Variant> = bind.child_value(1).get().unwrap();
        assert_eq!(opts["description"].get::<String>().unwrap(), "new_window");
        assert_eq!(
            opts["preferred_trigger"].get::<String>().unwrap(),
            "CTRL+ALT+x"
        );
    }
}
