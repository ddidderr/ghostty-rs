//! Port of apprt/gtk/class/surface_child_exited.zig.
//!
//! Child exited overlay based on `adw.Banner` (Adwaita 1.3+; the crate
//! requires 1.5 so the Zig `SurfaceChildExitedNoop` fallback is omitted).
//! Emits `close-request` when the banner button is clicked.

use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use ghostty_apprt::surface::ChildExited;
use gtk::glib;
use gtk::glib::subclass::Signal;

/// Boxed wrapper so `ChildExited` can be a GObject property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "GhosttyChildExited", nullable)]
pub struct ChildExitedBoxed(pub ChildExited);

glib::wrapper! {
    pub struct SurfaceChildExited(ObjectSubclass<imp::SurfaceChildExited>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SurfaceChildExited {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceChildExited {
    #[must_use]
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the child exited data (Zig: `setData`).
    pub fn set_child_exited(&self, data: Option<&ChildExited>) {
        self.set_data(data.map(|d| ChildExitedBoxed(*d)));
    }

    pub fn connect_close_request<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "close-request",
            false,
            glib::closure_local!(move |this: &Self| f(this)),
        )
    }
}

mod imp {
    use std::cell::RefCell;

    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties, Default)]
    #[properties(wrapper_type = super::SurfaceChildExited)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.3/surface-child-exited.ui")]
    pub struct SurfaceChildExited {
        /// The child exited data sent by the apprt.
        #[property(get, set = Self::set_data, nullable)]
        pub data: RefCell<Option<ChildExitedBoxed>>,

        #[template_child]
        pub banner: TemplateChild<adw::Banner>,
    }

    impl SurfaceChildExited {
        fn set_data(&self, v: Option<ChildExitedBoxed>) {
            // Zig only notifies when new data is given; the derive notifies
            // on every set which is harmless.
            self.data.replace(v);
        }
    }

    #[gtk::template_callbacks]
    impl SurfaceChildExited {
        /// `notify::data`; the template instance is the emitter.
        #[template_callback]
        fn notify_data(this: &super::SurfaceChildExited, _pspec: &glib::ParamSpec) {
            let imp = this.imp();
            let banner = &imp.banner;
            let Some(data) = imp.data.borrow().as_ref().map(|d| d.0) else {
                // Not localized on purpose.
                banner.set_title("This is a bug in Ghostty. Please report it.");
                return;
            };
            if data.exit_code == 0 {
                banner.set_title("Command succeeded");
                this.add_css_class("normal");
                this.remove_css_class("abnormal");
            } else {
                banner.set_title("Command failed");
                this.remove_css_class("normal");
                this.add_css_class("abnormal");
            }
        }

        #[template_callback]
        fn clicked(banner: &adw::Banner) {
            if let Some(this) = banner
                .ancestor(super::SurfaceChildExited::static_type())
                .and_downcast::<super::SurfaceChildExited>()
            {
                this.emit_by_name::<()>("close-request", &[]);
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SurfaceChildExited {
        const NAME: &'static str = "GhosttySurfaceChildExited";
        type Type = super::SurfaceChildExited;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SurfaceChildExited {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                // Emitted when the banner would like to be closed.
                vec![Signal::builder("close-request").build()]
            })
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for SurfaceChildExited {}
    impl BinImpl for SurfaceChildExited {}
}
