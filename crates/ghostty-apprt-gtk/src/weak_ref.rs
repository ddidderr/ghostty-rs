//! Weak object references (port of `apprt/gtk/weak_ref.zig`). glib-rs
//! already provides `glib::WeakRef<T>`; this wrapper keeps the Zig API
//! (`set`/`get`/`empty`).

use gtk::glib;
use gtk::glib::prelude::*;

/// A weak reference to a GObject of type `T`.
#[derive(Debug, Default)]
pub struct WeakRef<T: ObjectType>(glib::WeakRef<T>);

impl<T: ObjectType> WeakRef<T> {
    /// An empty weak reference.
    pub fn empty() -> Self {
        Self(glib::WeakRef::new())
    }

    pub fn set(&self, v: Option<&T>) {
        self.0.set(v);
    }

    pub fn get(&self) -> Option<T> {
        self.0.upgrade()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_ref() {
        let r: WeakRef<glib::Object> = WeakRef::empty();
        let obj = glib::Object::new::<glib::Object>();
        r.set(Some(&obj));
        assert!(r.get().is_some());
        drop(obj);
        assert!(r.get().is_none());
    }
}
