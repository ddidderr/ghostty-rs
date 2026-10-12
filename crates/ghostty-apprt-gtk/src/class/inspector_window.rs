//! Port of apprt/gtk/class/inspector_window.zig (`GhosttyInspectorWindow`).
//!
//! Window for displaying the Ghostty inspector.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::debug_warning::DebugWarning;
use super::inspector_widget::InspectorWidget;
use super::surface::Surface;

glib::wrapper! {
    pub struct InspectorWindow(ObjectSubclass<imp::InspectorWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

use gtk::gio;

impl InspectorWindow {
    pub fn new(surface: &Surface) -> Self {
        crate::register_resources();
        glib::Object::builder().property("surface", surface).build()
    }

    /// Present the window.
    pub fn present_window(&self) {
        self.present();
    }

    /// Queue a render of the embedded widget.
    pub fn queue_render(&self) {
        self.imp().inspector_widget.queue_render();
    }

    /// Debug aid: synthesize a left click in the inspector.
    pub fn debug_click(&self, x: f64, y: f64) {
        self.imp().inspector_widget.debug_click(x, y);
    }

    /// Debug aid: synthesize a mouse wheel scroll in the inspector.
    pub fn debug_scroll(&self, x: f64, y: f64, dy: f64) {
        self.imp().inspector_widget.debug_scroll(x, y, dy);
    }

    /// Debug aid: write the next rendered inspector frame to a PNG file.
    pub fn debug_screenshot(&self, path: &str) {
        self.imp().inspector_widget.debug_screenshot(path);
    }
}

mod imp {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties, Default)]
    #[properties(wrapper_type = super::InspectorWindow)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/inspector-window.ui")]
    pub struct InspectorWindow {
        /// The surface that we are attached to.
        #[property(get, set = Self::set_surface, nullable)]
        pub surface: glib::WeakRef<Surface>,

        /// The embedded inspector widget.
        #[template_child]
        pub inspector_widget: TemplateChild<InspectorWidget>,

        /// Whether this is a debug build (shows the debug warning).
        #[property(get = Self::get_debug, type = bool, name = "debug")]
        _debug_prop: (),
    }

    impl InspectorWindow {
        #[allow(clippy::unused_self)]
        fn get_debug(&self) -> bool {
            cfg!(debug_assertions)
        }

        #[allow(clippy::needless_pass_by_value)] // property setter signature
        fn set_surface(&self, surface: Option<Surface>) {
            self.surface.set(surface.as_ref());
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InspectorWindow {
        const NAME: &'static str = "GhosttyInspectorWindow";
        type Type = super::InspectorWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            DebugWarning::ensure_type();
            InspectorWidget::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InspectorWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Add our dev CSS class if we're in debug mode.
            if cfg!(debug_assertions) {
                obj.add_css_class("devel");
            }

            // Set our window icon. We can't set this in the blueprint file
            // because its dependent on the build config.
            obj.set_icon_name(Some(crate::APPLICATION_ID));
        }

        fn dispose(&self) {
            // You MUST clear all weak refs in dispose, otherwise it causes
            // memory corruption on dispose on the TARGET (weak referenced)
            // object (Zig comment).
            self.set_surface(None);
            self.dispose_template();
        }
    }

    impl WidgetImpl for InspectorWindow {}
    impl WindowImpl for InspectorWindow {}
    impl ApplicationWindowImpl for InspectorWindow {}
    impl AdwApplicationWindowImpl for InspectorWindow {}

    #[gtk::template_callbacks]
    impl InspectorWindow {
        /// `notify::surface` of the inspector widget: if the inspector's
        /// surface went away, we destroy the window.
        #[template_callback]
        fn notify_inspector_surface(
            inspector: &InspectorWidget,
            _pspec: &glib::ParamSpec,
            this: &super::InspectorWindow,
        ) {
            if inspector.get_surface().is_none() {
                this.destroy();
            }
        }
    }
}
