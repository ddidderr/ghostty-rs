//! Port of apprt/gtk/class/close_confirmation_dialog.zig
//! (`GhosttyCloseConfirmationDialog`). Property `target`; signals
//! `close-request` and `cancel`.

use std::{cell::Cell, sync::OnceLock};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_os::i18n::tr;
use gtk::{glib, glib::subclass::Signal};

use super::dialog::{Dialog, DialogImpl};

/// The target of a close dialog (`GhosttyCloseConfirmationDialogTarget`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "GhosttyCloseConfirmationDialogTarget")]
#[repr(i32)]
pub enum Target {
    #[default]
    App,
    Tab,
    Window,
    Surface,
}

impl Target {
    pub fn title(self) -> String {
        match self {
            Target::App => tr("Quit Ghostty?"),
            Target::Tab => tr("Close Tab?"),
            Target::Window => tr("Close Window?"),
            Target::Surface => tr("Close Split?"),
        }
    }

    pub fn body(self) -> String {
        match self {
            Target::App => tr("All terminal sessions will be terminated."),
            Target::Tab => tr("All terminal sessions in this tab will be terminated."),
            Target::Window => tr("All terminal sessions in this window will be terminated."),
            Target::Surface => {
                tr("The currently running process in this split will be terminated.")
            }
        }
    }
}

mod imp {
    use gtk::{glib, glib::prelude::*, subclass::prelude::*};

    use super::{
        AdwAlertDialogImpl,
        AdwDialogImpl,
        Cell,
        Dialog,
        DialogImpl,
        ObjectImpl,
        ObjectSubclass,
        OnceLock,
        Signal,
        Target,
        WidgetImpl,
    };

    #[derive(Default, glib::Properties, gtk::CompositeTemplate)]
    #[properties(wrapper_type = super::CloseConfirmationDialog)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/close-confirmation-dialog.ui")]
    pub struct CloseConfirmationDialog {
        #[property(get, set, builder(Target::App))]
        pub target: Cell<Target>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CloseConfirmationDialog {
        const NAME: &'static str = "GhosttyCloseConfirmationDialog";
        type Type = super::CloseConfirmationDialog;
        type ParentType = Dialog;

        fn class_init(klass: &mut Self::Class) {
            Dialog::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CloseConfirmationDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("close-request").build(),
                    Signal::builder("cancel").build(),
                ]
            })
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }
    impl WidgetImpl for CloseConfirmationDialog {}
    impl AdwDialogImpl for CloseConfirmationDialog {}
    impl AdwAlertDialogImpl for CloseConfirmationDialog {
        fn response(&self, response: &str) {
            let name = if response == "close" {
                "close-request"
            } else {
                "cancel"
            };
            self.obj().emit_by_name::<()>(name, &[]);
        }
    }
    impl DialogImpl for CloseConfirmationDialog {}
}

glib::wrapper! {
    pub struct CloseConfirmationDialog(ObjectSubclass<imp::CloseConfirmationDialog>)
        @extends Dialog, adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl CloseConfirmationDialog {
    pub fn new(target: Target) -> Self {
        crate::register_resources();
        glib::Object::builder().property("target", target).build()
    }

    /// Sets the heading/body from the target and presents over `parent`.
    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        let target = self.target();
        self.set_heading(Some(&target.title()));
        self.set_body(&target.body());
        self.upcast_ref::<Dialog>().present(parent);
    }

    pub fn close(&self) {
        self.upcast_ref::<Dialog>().close();
    }

    pub fn connect_close_request<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "close-request",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }

    pub fn connect_cancel<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "cancel",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
}
