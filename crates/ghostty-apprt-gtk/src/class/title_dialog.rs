//! Port of apprt/gtk/class/title_dialog.zig (`GhosttyTitleDialog`).
//! Properties `target`, `initial-value`; signal `set(title: string)`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use ghostty_os::i18n::tr;
use gtk::glib;
use gtk::glib::subclass::Signal;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

/// `GhosttyTitleDialogTarget`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "GhosttyTitleDialogTarget")]
#[repr(i32)]
pub enum Target {
    #[default]
    Surface,
    Tab,
}

impl Target {
    pub fn title(self) -> String {
        match self {
            Target::Surface => tr("Change Terminal Title"),
            Target::Tab => tr("Change Tab Title"),
        }
    }
}

mod imp {
    use super::{
        AdwAlertDialogImpl, AdwDialogImpl, Cell, ObjectImpl, ObjectSubclass, OnceLock, RefCell,
        Signal, Target, WidgetImpl,
    };
    use gtk::glib;
    use gtk::glib::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default, glib::Properties, gtk::CompositeTemplate)]
    #[properties(wrapper_type = super::TitleDialog)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/title-dialog.ui")]
    pub struct TitleDialog {
        #[property(get, set, builder(Target::Surface))]
        pub target: Cell<Target>,

        /// The initial value of the entry field.
        #[property(get, set, nullable, name = "initial-value")]
        pub initial_value: RefCell<Option<String>>,

        #[template_child]
        pub entry: TemplateChild<gtk::Entry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TitleDialog {
        const NAME: &'static str = "GhosttyTitleDialog";
        type Type = super::TitleDialog;
        type ParentType = adw::AlertDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for TitleDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("set")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }
    impl WidgetImpl for TitleDialog {}
    impl AdwDialogImpl for TitleDialog {}
    impl AdwAlertDialogImpl for TitleDialog {}
}

glib::wrapper! {
    pub struct TitleDialog(ObjectSubclass<imp::TitleDialog>)
        @extends adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl TitleDialog {
    pub fn new(target: Target, initial_value: Option<&str>) -> Self {
        crate::register_resources();
        glib::Object::builder()
            .property("target", target)
            .property("initial-value", initial_value)
            .build()
    }

    /// Present the dialog, anchored to the window containing `parent`.
    /// Emits `set` with the entered title if the user accepts.
    pub fn present(&self, parent: &impl IsA<gtk::Widget>) {
        let parent_w = parent.upcast_ref::<gtk::Widget>();
        let anchor: gtk::Widget = parent_w
            .ancestor(adw::ApplicationWindow::static_type())
            .or_else(|| parent_w.ancestor(adw::Window::static_type()))
            .unwrap_or_else(|| parent_w.clone());

        let imp = self.imp();
        if let Some(v) = imp.initial_value.borrow().as_deref() {
            imp.entry.buffer().set_text(v);
        }

        self.set_heading(Some(&imp.target.get().title()));

        let this = self.clone();
        self.clone().choose(
            Some(&anchor),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                if response != "ok" {
                    return;
                }
                let title = this.imp().entry.buffer().text().to_string();
                this.emit_by_name::<()>("set", &[&title]);
            },
        );
    }

    pub fn connect_set<F: Fn(&Self, &str) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "set",
            false,
            glib::closure_local!(move |obj: &Self, title: &str| f(obj, title)),
        )
    }
}
