//! Port of apprt/gtk/class/surface_scrolled_window.zig.
//!
//! A wrapper widget that embeds a Surface inside a `GtkScrolledWindow`,
//! providing scrollbar functionality for the terminal surface. The
//! `surface` property can be set at construction or changed dynamically.
//!
//! The GType name `GhostttySurfaceScrolledWindow` (sic, three t's) matches
//! the Zig source and the blueprint template.
//!
//! To limit compile coupling the `surface` property is typed as
//! `GtkWidget` (any `crate::class::surface::Surface` is accepted) and the
//! `config` property as `GObject` (a `crate::class::config::Config`).

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

glib::wrapper! {
    pub struct SurfaceScrolledWindow(ObjectSubclass<imp::SurfaceScrolledWindow>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SurfaceScrolledWindow {
    fn default() -> Self {
        Self::new(None::<&gtk::Widget>)
    }
}

impl SurfaceScrolledWindow {
    /// Create a scrolled window, optionally with the given surface widget.
    #[must_use]
    pub fn new(surface: Option<&impl IsA<gtk::Widget>>) -> Self {
        glib::Object::builder()
            .property("surface", surface.map(|s| s.as_ref().clone()))
            .build()
    }

    /// Zig: `getSurface`. Downcast to `Surface` at the call site.
    #[must_use]
    pub fn get_surface(&self) -> Option<gtk::Widget> {
        self.surface()
    }
}

mod imp {
    use std::cell::RefCell;

    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties, Default)]
    #[properties(wrapper_type = super::SurfaceScrolledWindow)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/surface-scrolled-window.ui")]
    pub struct SurfaceScrolledWindow {
        #[property(get, set, nullable)]
        pub config: RefCell<Option<glib::Object>>,

        pub config_binding: RefCell<Option<glib::Binding>>,

        #[property(get, set = Self::set_surface, nullable)]
        pub surface: RefCell<Option<gtk::Widget>>,
    }

    impl SurfaceScrolledWindow {
        fn set_surface(&self, v: Option<gtk::Widget>) {
            if *self.surface.borrow() == v {
                return;
            }
            self.surface.replace(v);
        }
    }

    #[gtk::template_callbacks]
    impl SurfaceScrolledWindow {
        #[template_callback(function)]
        fn scrollbar_policy(config: Option<glib::Object>) -> gtk::PolicyType {
            use crate::class::config::Config;
            use ghostty_config::types_b::Scrollbar;

            let Some(config) = config.and_then(|c| c.downcast::<Config>().ok()) else {
                return gtk::PolicyType::Automatic;
            };
            match config.get().scrollbar {
                Scrollbar::Never => gtk::PolicyType::Never,
                Scrollbar::System => gtk::PolicyType::Automatic,
            }
        }

        #[template_callback]
        fn notify_surface(this: &super::SurfaceScrolledWindow, _pspec: &glib::ParamSpec) {
            let imp = this.imp();
            let Some(child) = this.child() else {
                return;
            };
            let Ok(scrolled) = child.downcast::<gtk::ScrolledWindow>() else {
                return;
            };
            let surface = imp.surface.borrow().clone();
            scrolled.set_child(surface.as_ref());

            // Unbind old config binding if it exists
            if let Some(binding) = imp.config_binding.take() {
                binding.unbind();
            }

            // Bind config from surface to our config property
            if let Some(surface) = surface {
                let binding = surface
                    .bind_property("config", this, "config")
                    .sync_create()
                    .build();
                imp.config_binding.replace(Some(binding));
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SurfaceScrolledWindow {
        const NAME: &'static str = "GhostttySurfaceScrolledWindow";
        type Type = super::SurfaceScrolledWindow;
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
    impl ObjectImpl for SurfaceScrolledWindow {
        fn dispose(&self) {
            if let Some(binding) = self.config_binding.take() {
                binding.unbind();
            }
            self.config.replace(None);
            self.dispose_template();
        }
    }

    impl WidgetImpl for SurfaceScrolledWindow {}
    impl BinImpl for SurfaceScrolledWindow {}
}
