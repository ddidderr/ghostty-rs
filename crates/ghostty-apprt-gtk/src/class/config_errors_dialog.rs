//! Port of apprt/gtk/class/config_errors_dialog.zig (`GhosttyConfigErrorsDialog`).
//!
//! Property `config` (GhosttyConfig); signal `reload-config`.

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::glib::subclass::Signal;
use std::sync::OnceLock;

use super::config::Config;
use super::dialog::{Dialog, DialogImpl};

mod imp {
    use super::{
        AdwAlertDialogImpl, AdwDialogImpl, Config, Dialog, DialogImpl, ObjectImpl, ObjectSubclass,
        ObjectSubclassExt, OnceLock, RefCell, Signal, WidgetImpl,
    };
    use gtk::glib;
    use gtk::glib::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default, glib::Properties, gtk::CompositeTemplate)]
    #[properties(wrapper_type = super::ConfigErrorsDialog)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/config-errors-dialog.ui")]
    pub struct ConfigErrorsDialog {
        #[property(get, set, nullable)]
        pub config: RefCell<Option<Config>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConfigErrorsDialog {
        const NAME: &'static str = "GhosttyConfigErrorsDialog";
        type Type = super::ConfigErrorsDialog;
        type ParentType = Dialog;

        fn class_init(klass: &mut Self::Class) {
            // Types referenced by the template must be registered first.
            Config::ensure_type();
            Dialog::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ConfigErrorsDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("reload-config").build()])
        }

        fn dispose(&self) {
            self.config.replace(None);
            self.dispose_template();
        }
    }
    impl WidgetImpl for ConfigErrorsDialog {}
    impl AdwDialogImpl for ConfigErrorsDialog {}
    impl AdwAlertDialogImpl for ConfigErrorsDialog {
        fn response(&self, response: &str) {
            if response != "reload" {
                return;
            }
            self.obj().emit_by_name::<()>("reload-config", &[]);
        }
    }
    impl DialogImpl for ConfigErrorsDialog {}
}

glib::wrapper! {
    pub struct ConfigErrorsDialog(ObjectSubclass<imp::ConfigErrorsDialog>)
        @extends Dialog, adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl ConfigErrorsDialog {
    pub fn new(config: &Config) -> Self {
        crate::register_resources();
        glib::Object::builder().property("config", config).build()
    }

    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        self.upcast_ref::<Dialog>().present(parent);
    }

    pub fn close(&self) {
        self.upcast_ref::<Dialog>().close();
    }

    pub fn connect_reload_config<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "reload-config",
            false,
            glib::closure_local!(move |obj: &Self| f(obj)),
        )
    }
}
