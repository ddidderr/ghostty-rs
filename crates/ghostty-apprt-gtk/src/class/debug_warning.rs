//! Port of apprt/gtk/class/debug_warning.zig (`GhosttyDebugWarning`).
//! Uses the `adw::Banner` variant (libadwaita >= 1.3).

use gtk::glib;

mod imp {
    use gtk::{glib, subclass::prelude::*};

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.3/debug-warning.ui")]
    pub struct DebugWarning;

    #[glib::object_subclass]
    impl ObjectSubclass for DebugWarning {
        const NAME: &'static str = "GhosttyDebugWarning";
        type Type = super::DebugWarning;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DebugWarning {
        fn dispose(&self) {
            self.dispose_template();
        }
    }
    impl WidgetImpl for DebugWarning {}
    impl adw::subclass::prelude::BinImpl for DebugWarning {}
}

glib::wrapper! {
    pub struct DebugWarning(ObjectSubclass<imp::DebugWarning>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DebugWarning {
    pub fn new() -> Self {
        crate::register_resources();
        glib::Object::new()
    }
}

impl Default for DebugWarning {
    fn default() -> Self {
        Self::new()
    }
}
