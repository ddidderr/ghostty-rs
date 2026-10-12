//! Port of apprt/gtk/class/dialog.zig (`GhosttyDialog`).
//!
//! Thin abstraction over `adw::AlertDialog` and `adw::MessageDialog`, chosen
//! at compile time from the libadwaita build level exactly like Zig's
//! `Parent = if (adw_version.supportsDialogs()) adw.AlertDialog else
//! adw.MessageDialog` (the `adw_1_5` cargo feature plays the role of the
//! header version). Subclasses implement [`DialogImpl`] and override
//! `response` through [`impl_dialog_parent!`].
//!
//! Once we drop support for Adwaita <= 1.2, this can be fully removed and we
//! can use `adw::AlertDialog` directly.

use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

/// The parent class of [`Dialog`] (Zig: `Dialog.Parent`).
#[cfg(feature = "adw_1_5")]
pub type DialogParent = adw::AlertDialog;
/// The parent class of [`Dialog`] (Zig: `Dialog.Parent`).
#[cfg(not(feature = "adw_1_5"))]
pub type DialogParent = adw::MessageDialog;

mod imp {
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Default)]
    pub struct Dialog;

    #[glib::object_subclass]
    impl ObjectSubclass for Dialog {
        const NAME: &'static str = "GhosttyDialog";
        type Type = super::Dialog;
        type ParentType = super::DialogParent;
    }

    impl ObjectImpl for Dialog {}
    impl WidgetImpl for Dialog {}
    #[cfg(feature = "adw_1_5")]
    impl AdwDialogImpl for Dialog {}
    #[cfg(feature = "adw_1_5")]
    impl AdwAlertDialogImpl for Dialog {}
    #[cfg(not(feature = "adw_1_5"))]
    impl WindowImpl for Dialog {}
    #[cfg(not(feature = "adw_1_5"))]
    impl MessageDialogImpl for Dialog {}
}

#[cfg(feature = "adw_1_5")]
glib::wrapper! {
    pub struct Dialog(ObjectSubclass<imp::Dialog>)
        @extends adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

#[cfg(not(feature = "adw_1_5"))]
glib::wrapper! {
    pub struct Dialog(ObjectSubclass<imp::Dialog>)
        @extends adw::MessageDialog, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

/// Implemented by subclasses of [`Dialog`].
#[cfg(feature = "adw_1_5")]
pub trait DialogImpl: AdwAlertDialogImpl {}
/// Implemented by subclasses of [`Dialog`].
#[cfg(not(feature = "adw_1_5"))]
pub trait DialogImpl: MessageDialogImpl {}

#[allow(unsafe_code)]
// SAFETY: `Dialog` adds no class/instance data of its own; we simply forward
// to the parent class/instance initialization, which is valid for any
// `T: DialogImpl` (a subtype of the parent's `*Impl` trait).
unsafe impl<T: DialogImpl> IsSubclassable<T> for Dialog {
    fn class_init(class: &mut glib::Class<Self>) {
        <DialogParent as IsSubclassable<T>>::class_init(class.upcast_ref_mut());
    }

    fn instance_init(instance: &mut glib::subclass::InitializingObject<T>) {
        <DialogParent as IsSubclassable<T>>::instance_init(instance);
    }
}

impl Dialog {
    /// Present the dialog over `parent`.
    #[cfg(feature = "adw_1_5")]
    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        adw::prelude::AdwDialogExt::present(self, parent);
    }

    /// Present the dialog over `parent`.
    #[cfg(not(feature = "adw_1_5"))]
    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        // Reset the previous parent window
        self.set_transient_for(None::<&gtk::Window>);

        // We need to get the window for the parent in order to set the
        // transient-for property on the MessageDialog.
        if let Some(window) = parent
            .and_then(gtk::prelude::WidgetExt::root)
            .and_then(|root| root.downcast::<gtk::Window>().ok())
        {
            self.set_transient_for(Some(&window));
        }

        gtk::prelude::GtkWindowExt::present(self);
    }

    /// Close the dialog without emitting `close-attempt`.
    #[cfg(feature = "adw_1_5")]
    pub fn close(&self) {
        self.force_close();
    }

    /// Close the dialog.
    #[cfg(not(feature = "adw_1_5"))]
    pub fn close(&self) {
        gtk::prelude::GtkWindowExt::close(self);
    }
}

/// Declares the `glib::wrapper!` of a [`Dialog`] subclass with the right
/// `@extends` chain for the libadwaita build level.
macro_rules! dialog_subclass_wrapper {
    ($name:ident, $imp:ty) => {
        #[cfg(feature = "adw_1_5")]
        glib::wrapper! {
            pub struct $name(ObjectSubclass<$imp>)
                @extends $crate::class::dialog::Dialog, adw::AlertDialog, adw::Dialog, gtk::Widget,
                @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::ShortcutManager;
        }

        #[cfg(not(feature = "adw_1_5"))]
        glib::wrapper! {
            pub struct $name(ObjectSubclass<$imp>)
                @extends $crate::class::dialog::Dialog, adw::MessageDialog, gtk::Window,
                    gtk::Widget,
                @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::Native, gtk::Root, gtk::ShortcutManager;
        }
    };
}

/// Implements the parent-class traits of a [`Dialog`] subclass (the
/// `Adw{Dialog,AlertDialog}Impl` or `{Window,MessageDialog}Impl` pair),
/// overriding `response` with `|this, response| body`.
macro_rules! impl_dialog_parent {
    ($ty:ty, |$this:ident, $response:ident| $body:block) => {
        #[cfg(feature = "adw_1_5")]
        impl adw::subclass::prelude::AdwDialogImpl for $ty {}
        #[cfg(feature = "adw_1_5")]
        impl adw::subclass::prelude::AdwAlertDialogImpl for $ty {
            fn response(&self, $response: &str) {
                let $this = self;
                $body
            }
        }
        #[cfg(not(feature = "adw_1_5"))]
        impl gtk::subclass::prelude::WindowImpl for $ty {}
        #[cfg(not(feature = "adw_1_5"))]
        impl adw::subclass::prelude::MessageDialogImpl for $ty {
            fn response(&self, $response: &str) {
                let $this = self;
                $body
            }
        }
        impl $crate::class::dialog::DialogImpl for $ty {}
    };
}

pub(crate) use dialog_subclass_wrapper;
pub(crate) use impl_dialog_parent;
