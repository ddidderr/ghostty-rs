//! Port of apprt/gtk/class/dialog.zig (`GhosttyDialog`).
//!
//! Thin `adw::AlertDialog` subclass (we require libadwaita >= 1.5 so the
//! `adw::MessageDialog` fallback is not needed). Subclasses implement
//! [`DialogImpl`] and may override `response` via `AlertDialogImpl`.

use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

mod imp {
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Default)]
    pub struct Dialog;

    #[glib::object_subclass]
    impl ObjectSubclass for Dialog {
        const NAME: &'static str = "GhosttyDialog";
        type Type = super::Dialog;
        type ParentType = adw::AlertDialog;
    }

    impl ObjectImpl for Dialog {}
    impl WidgetImpl for Dialog {}
    impl AdwDialogImpl for Dialog {}
    impl AdwAlertDialogImpl for Dialog {}
}

glib::wrapper! {
    pub struct Dialog(ObjectSubclass<imp::Dialog>)
        @extends adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

/// Implemented by subclasses of [`Dialog`].
pub trait DialogImpl: AdwAlertDialogImpl {}

#[allow(unsafe_code)]
// SAFETY: `Dialog` adds no class/instance data of its own; we simply forward
// to the parent (`adw::AlertDialog`) class/instance initialization, which is
// valid for any `T: DialogImpl` (a subtype of `AdwAlertDialogImpl`).
unsafe impl<T: DialogImpl> IsSubclassable<T> for Dialog {
    fn class_init(class: &mut glib::Class<Self>) {
        <adw::AlertDialog as IsSubclassable<T>>::class_init(class.upcast_ref_mut());
    }

    fn instance_init(instance: &mut glib::subclass::InitializingObject<T>) {
        <adw::AlertDialog as IsSubclassable<T>>::instance_init(instance);
    }
}

impl Dialog {
    /// Present the dialog over `parent`.
    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        adw::prelude::AdwDialogExt::present(self, parent);
    }

    /// Close the dialog without emitting `close-attempt`.
    pub fn close(&self) {
        self.force_close();
    }
}
