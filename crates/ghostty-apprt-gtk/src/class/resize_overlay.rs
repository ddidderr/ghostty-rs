//! Port of apprt/gtk/class/resize_overlay.zig.
//!
//! The overlay that shows the current size while a surface is resizing.
//! This can be used generically to show pretty much anything with a
//! disappearing overlay.
//!
//! General usage:
//!
//!   1. Add it to an overlay
//!   2. Set the label with `set_label`
//!   3. Schedule to show it with `schedule`

use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

glib::wrapper! {
    pub struct ResizeOverlay(ObjectSubclass<imp::ResizeOverlay>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ResizeOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl ResizeOverlay {
    #[must_use]
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Schedule the overlay to be shown. To avoid flickering during
    /// resizes we schedule the overlay to be shown on the next idle tick.
    pub fn schedule(&self) {
        let imp = self.imp();

        // If we have a delay timer then we're not showing anything yet.
        if imp.delay_timer.borrow().is_some() {
            return;
        }

        // Wait until GTK is idle (we're called in the middle of resizing).
        if imp.idler.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        let id = glib::idle_add_local_once(move || {
            if let Some(this) = weak.upgrade() {
                this.on_idle();
            }
        });
        imp.idler.replace(Some(id));
    }

    fn on_idle(&self) {
        let imp = self.imp();
        // The idler is complete no matter what.
        imp.idler.replace(None);

        // Cancel our previous show timer no matter what.
        if let Some(t) = imp.timer.take() {
            t.remove();
        }

        // If we have a label to show, show ourselves; otherwise hide.
        let Some(text) = self.label() else {
            self.set_visible(false);
            return;
        };

        imp.label.set_label(&text);
        self.set_visible(true);

        // Setup the new timer to hide ourselves after the delay.
        let weak = self.downgrade();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(u64::from(self.duration())),
            move || {
                if let Some(this) = weak.upgrade() {
                    this.imp().timer.replace(None);
                    this.set_visible(false);
                }
            },
        );
        imp.timer.replace(Some(id));
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::ResizeOverlay)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/resize-overlay.ui")]
    pub struct ResizeOverlay {
        /// The label with the text
        #[template_child]
        pub label: TemplateChild<gtk::Label>,

        /// The time that the overlay appears (ms).
        #[property(get, set, minimum = 250, default = 750)]
        pub duration: Cell<u32>,

        /// The first delay before any overlay is shown (ms).
        #[property(get, set, minimum = 250, default = 250, name = "first-delay")]
        pub first_delay: Cell<u32>,

        /// The text to set on the label when scheduled.
        #[property(get, set, name = "label", nullable)]
        pub label_text: RefCell<Option<String>>,

        /// The alignment of the label
        #[property(get, set, name = "overlay-halign", builder(gtk::Align::Center))]
        pub halign: Cell<gtk::Align>,
        #[property(get, set, name = "overlay-valign", builder(gtk::Align::Center))]
        pub valign: Cell<gtk::Align>,

        pub idler: RefCell<Option<glib::SourceId>>,
        pub timer: RefCell<Option<glib::SourceId>>,
        pub delay_timer: RefCell<Option<glib::SourceId>>,
    }

    impl Default for ResizeOverlay {
        fn default() -> Self {
            Self {
                label: TemplateChild::default(),
                duration: Cell::new(750),
                first_delay: Cell::new(250),
                label_text: RefCell::new(None),
                halign: Cell::new(gtk::Align::Center),
                valign: Cell::new(gtk::Align::Center),
                idler: RefCell::new(None),
                timer: RefCell::new(None),
                delay_timer: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResizeOverlay {
        const NAME: &'static str = "GhosttyResizeOverlay";
        type Type = super::ResizeOverlay;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ResizeOverlay {
        fn constructed(&self) {
            self.parent_constructed();
            // The template sets `first-delay`; start the initial delay timer.
            let delay = self.first_delay.get();
            if delay > 0 {
                let weak = self.obj().downgrade();
                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(delay)),
                    move || {
                        if let Some(this) = weak.upgrade() {
                            this.imp().delay_timer.replace(None);
                        }
                    },
                );
                self.delay_timer.replace(Some(id));
            }
        }

        fn dispose(&self) {
            if let Some(v) = self.idler.take() {
                v.remove();
            }
            if let Some(v) = self.timer.take() {
                v.remove();
            }
            if let Some(v) = self.delay_timer.take() {
                v.remove();
            }
            self.dispose_template();
        }
    }

    impl WidgetImpl for ResizeOverlay {}
    impl BinImpl for ResizeOverlay {}
}
