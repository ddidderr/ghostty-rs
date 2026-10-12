//! Port of apprt/gtk/class/config_errors_dialog.zig (`GhosttyConfigErrorsDialog`).
//!
//! Property `config` (GhosttyConfig); signal `reload-config`.

use std::{cell::RefCell, sync::OnceLock};

use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::subclass::Signal};

use super::{
    config::Config,
    dialog::{Dialog, dialog_subclass_wrapper, impl_dialog_parent},
};

mod imp {
    use gtk::{glib, glib::prelude::*, subclass::prelude::*};

    use super::{
        Config,
        Dialog,
        ObjectImpl,
        ObjectSubclass,
        ObjectSubclassExt,
        OnceLock,
        RefCell,
        Signal,
        WidgetImpl,
    };

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
    super::impl_dialog_parent!(ConfigErrorsDialog, |this, response| {
        if response != "reload" {
            return;
        }
        this.obj().emit_by_name::<()>("reload-config", &[]);
    });
}

dialog_subclass_wrapper!(ConfigErrorsDialog, imp::ConfigErrorsDialog);

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
