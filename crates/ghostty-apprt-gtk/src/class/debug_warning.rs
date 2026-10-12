//! Port of apprt/gtk/class/debug_warning.zig (`GhosttyDebugWarning`).
//!
//! Debug warning banner. It is based on `adw::Banner` (a `adw::Bin` with the
//! 1.3 template) if we're using Adwaita 1.3 or newer (the `adw_1_3` build
//! level, Zig: `adw_version.supportsBanner()`). Otherwise it is a `gtk::Box`
//! holding a label (1.2 template).

use gtk::glib;

mod imp {
    use gtk::{glib, subclass::prelude::*};

    #[derive(Default, gtk::CompositeTemplate)]
    #[cfg_attr(
        feature = "adw_1_3",
        template(resource = "/com/mitchellh/ghostty/ui/1.3/debug-warning.ui")
    )]
    #[cfg_attr(
        not(feature = "adw_1_3"),
        template(resource = "/com/mitchellh/ghostty/ui/1.2/debug-warning.ui")
    )]
    pub struct DebugWarning;

    #[glib::object_subclass]
    impl ObjectSubclass for DebugWarning {
        const NAME: &'static str = "GhosttyDebugWarning";
        type Type = super::DebugWarning;
        #[cfg(feature = "adw_1_3")]
        type ParentType = adw::Bin;
        #[cfg(not(feature = "adw_1_3"))]
        type ParentType = gtk::Box;

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
    #[cfg(feature = "adw_1_3")]
    impl adw::subclass::prelude::BinImpl for DebugWarning {}
    #[cfg(not(feature = "adw_1_3"))]
    impl BoxImpl for DebugWarning {}
}

#[cfg(feature = "adw_1_3")]
glib::wrapper! {
    pub struct DebugWarning(ObjectSubclass<imp::DebugWarning>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

#[cfg(not(feature = "adw_1_3"))]
glib::wrapper! {
    pub struct DebugWarning(ObjectSubclass<imp::DebugWarning>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
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
