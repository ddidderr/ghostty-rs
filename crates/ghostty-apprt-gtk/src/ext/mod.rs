//! Extensions/helpers for GTK objects (port of `apprt/gtk/ext.zig`).
//!
//! `boxedCopy`/`boxedFree`/`listFind` are subsumed by glib-rs (`Clone`,
//! `Drop`, iterators) and are not ported.

pub mod actions;
pub mod slice;

use gtk::{glib, glib::prelude::*, prelude::*};
pub use slice::StringList;

/// Get the first ancestor of `widget` of type `T`.
pub fn get_ancestor<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Option<T> {
    widget.ancestor(T::static_type()).and_downcast::<T>()
}

/// Whether the value holds (a subtype of) the given type.
pub fn g_value_holds(value: Option<&glib::Value>, g_type: glib::Type) -> bool {
    let Some(value) = value else { return false };
    value.type_() == g_type || value.type_().is_a(g_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_value_holds_test() {
        let v = 5i32.to_value();
        assert!(g_value_holds(Some(&v), glib::Type::I32));
        assert!(!g_value_holds(Some(&v), glib::Type::STRING));
        assert!(!g_value_holds(None, glib::Type::I32));
    }
}
