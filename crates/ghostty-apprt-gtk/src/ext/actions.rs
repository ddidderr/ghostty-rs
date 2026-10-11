//! GAction helpers (port of `apprt/gtk/ext/actions.zig`).

use gtk::gio;
use gtk::glib;
use gtk::glib::prelude::*;
use gtk::prelude::*;

fn g_action_name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c == b'-' || c == b'.' || c.is_ascii_alphanumeric())
}

/// Callback signature: `(action, parameter, self)`.
pub type Callback<T> = fn(&gio::SimpleAction, Option<&glib::Variant>, &T);

/// An action definition for objects of type `T`.
pub struct Action<T> {
    pub name: &'static str,
    pub callback: Callback<T>,
    pub parameter_type: Option<glib::VariantType>,
    pub state: Option<glib::Variant>,
}

impl<T> Action<T> {
    pub fn new(
        name: &'static str,
        callback: Callback<T>,
        parameter_type: Option<glib::VariantType>,
    ) -> Self {
        assert!(g_action_name_is_valid(name));
        Self {
            name,
            callback,
            parameter_type,
            state: None,
        }
    }

    pub fn stateful(
        name: &'static str,
        callback: Callback<T>,
        parameter_type: Option<glib::VariantType>,
        state: glib::Variant,
    ) -> Self {
        assert!(g_action_name_is_valid(name));
        Self {
            name,
            callback,
            parameter_type,
            state: Some(state),
        }
    }
}

/// Add the actions to `obj` itself (it must be an `ActionMap`).
pub fn add<T: IsA<gio::ActionMap>>(obj: &T, actions: &[Action<T>]) {
    add_to_map(obj, obj.upcast_ref::<gio::ActionMap>(), actions);
}

/// Add the actions to `map`; callbacks receive `obj` (held weakly, so no
/// reference cycle is created).
pub fn add_to_map<T: ObjectType>(obj: &T, map: &impl IsA<gio::ActionMap>, actions: &[Action<T>]) {
    for entry in actions {
        assert!(g_action_name_is_valid(entry.name));
        let action = if let Some(state) = &entry.state {
            gio::SimpleAction::new_stateful(entry.name, entry.parameter_type.as_deref(), state)
        } else {
            gio::SimpleAction::new(entry.name, entry.parameter_type.as_deref())
        };
        let weak = obj.downgrade();
        let callback = entry.callback;
        action.connect_activate(move |a, p| {
            if let Some(obj) = weak.upgrade() {
                callback(a, p, &obj);
            }
        });
        map.add_action(&action);
    }
}

/// Collect the actions into a group inserted into the widget under `name`.
pub fn add_as_group<T: IsA<gtk::Widget>>(
    obj: &T,
    name: &str,
    actions: &[Action<T>],
) -> gio::SimpleActionGroup {
    assert!(g_action_name_is_valid(name));

    // We're a plain widget that doesn't implement ActionGroup directly.
    let group = gio::SimpleActionGroup::new();
    add_to_map(obj, &group, actions);
    obj.insert_action_group(name, Some(&group));
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_action_name_is_valid_test() {
        assert!(g_action_name_is_valid("ring-bell"));
        assert!(!g_action_name_is_valid("ring_bell"));
        assert!(!g_action_name_is_valid(""));
    }

    fn cb(_: &gio::SimpleAction, v: Option<&glib::Variant>, b: &gtk::Box) {
        if let Some(v) = v.and_then(glib::Variant::get::<i32>) {
            b.set_spacing(v);
        }
    }

    #[test]
    fn adding_actions_to_an_object() {
        // This test requires a connection to an active display environment.
        if gtk::init().is_err() {
            return;
        }

        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let actions = [Action::new(
            "test",
            cb,
            Some(i32::static_variant_type().into_owned()),
        )];
        let _group = add_as_group(&b, "test", &actions);

        let expected = 12345;
        assert!(
            b.activate_action("test.test", Some(&expected.to_variant()))
                .is_ok()
        );
        while glib::MainContext::default().iteration(false) {}
        assert_eq!(b.spacing(), expected);
    }
}
