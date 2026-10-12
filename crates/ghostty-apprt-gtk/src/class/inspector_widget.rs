//! Port of apprt/gtk/class/inspector_widget.zig (`GhosttyInspectorWidget`).
//!
//! Widget for displaying the Ghostty inspector. Setting its `surface`
//! property activates the inspector on that surface's core surface and
//! clearing it deactivates it again.

use std::{cell::RefCell, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_apprt::runtime_api::CoreSurface;
use gtk::glib;

use super::{
    imgui_widget::{ImguiWidget, ImguiWidgetImpl},
    surface::Surface,
};

glib::wrapper! {
    pub struct InspectorWidget(ObjectSubclass<imp::InspectorWidget>)
        @extends ImguiWidget, adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InspectorWidget {
    /// Queue a render of the Dear ImGui widget.
    pub fn queue_render(&self) {
        self.upcast_ref::<ImguiWidget>().queue_render();
    }

    pub fn get_surface(&self) -> Option<Surface> {
        self.imp().surface.upgrade()
    }

    /// Debug aid: synthesize a left click.
    pub fn debug_click(&self, x: f64, y: f64) {
        self.upcast_ref::<ImguiWidget>().debug_click(x, y);
    }

    /// Debug aid: synthesize a mouse wheel scroll.
    pub fn debug_scroll(&self, x: f64, y: f64, dy: f64) {
        self.upcast_ref::<ImguiWidget>().debug_scroll(x, y, dy);
    }

    /// Debug aid: write the next rendered frame to a PNG file.
    pub fn debug_screenshot(&self, path: &str) {
        self.upcast_ref::<ImguiWidget>().debug_screenshot(path);
    }
}

impl ImguiWidgetImpl for imp::InspectorWidget {}

mod imp {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties, Default)]
    #[properties(wrapper_type = super::InspectorWidget)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/inspector-widget.ui")]
    pub struct InspectorWidget {
        /// The surface that we are attached to. This is NOT strongly
        /// referenced (the GObject weak reference is cleared when the surface
        /// is disposed, which closes our window).
        pub surface: glib::WeakRef<Surface>,

        /// The core surface of `surface`, kept so that we can deactivate the
        /// inspector when the surface is going away (the GTK surface only
        /// drops its core surface when finalized, after the window holding
        /// this widget was destroyed in its dispose).
        core: RefCell<Option<Rc<dyn CoreSurface>>>,

        #[property(get = Self::get_surface, set = Self::set_surface, nullable, type = Option<Surface>, name = "surface")]
        _surface_prop: (),
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InspectorWidget {
        const NAME: &'static str = "GhosttyInspectorWidget";
        type Type = super::InspectorWidget;
        type ParentType = ImguiWidget;

        fn class_init(klass: &mut Self::Class) {
            ImguiWidget::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InspectorWidget {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let weak = obj.downgrade();
            obj.upcast_ref::<ImguiWidget>().set_hooks(
                // Called to do initial setup of the UI.
                Box::new(|ctx, _scale| ghostty_inspector::setup(ctx)),
                // Called for every frame to draw the UI.
                Box::new(move |ui| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let Some(surface) = this.imp().surface.upgrade() else {
                        return;
                    };
                    let Some(core) = surface.core() else {
                        return;
                    };
                    core.render_inspector(ui);
                }),
            );
        }

        fn dispose(&self) {
            // Clear our surface so it deactivates the inspector.
            self.set_surface(None);
            self.dispose_template();
        }
    }

    impl WidgetImpl for InspectorWidget {}
    impl BinImpl for InspectorWidget {}

    impl InspectorWidget {
        fn get_surface(&self) -> Option<Surface> {
            self.surface.upgrade()
        }

        fn set_surface(&self, surface: Option<Surface>) {
            // Do nothing if we're not changing the value.
            if surface == self.surface.upgrade() {
                return;
            }

            // Setup our notification to happen at the end because we're
            // changing values no matter what.
            let obj = self.obj();
            let _guard = obj.freeze_notify();
            obj.notify("surface");

            // Deactivate the inspector on the old surface if it exists and
            // set our value to null.
            self.surface.set(None);
            if let Some(core) = self.core.take() {
                core.deactivate_inspector();
            }

            // Activate the inspector on the new surface.
            let Some(surface) = surface else {
                return;
            };
            let Some(core) = surface.core() else {
                return;
            };
            if let Err(err) = core.activate_inspector() {
                log::warn!("failed to activate inspector err={err}");
                return;
            }

            self.surface.set(Some(&surface));
            self.core.replace(Some(core));
            obj.queue_render();
        }
    }
}
