//! Port of apprt/gtk/class/key_state_overlay.zig.
//!
//! An overlay that displays the current key table stack and pending key
//! sequence. This helps users understand what key bindings are active and
//! what keys they've pressed in a multi-key sequence.
//!
//! Deviation: Zig's `ext.StringList` boxed type is replaced by `GStrv`
//! (`Vec<String>`) for the `tables` and `sequence` properties.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

glib::wrapper! {
    pub struct KeyStateOverlay(ObjectSubclass<imp::KeyStateOverlay>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for KeyStateOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStateOverlay {
    #[must_use]
    pub fn new() -> Self {
        glib::Object::new()
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::KeyStateOverlay)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/key-state-overlay.ui")]
    pub struct KeyStateOverlay {
        /// The key table stack.
        #[property(get, set = Self::set_tables)]
        pub tables: RefCell<Vec<String>>,

        /// The key sequence.
        #[property(get, set = Self::set_sequence)]
        pub sequence: RefCell<Vec<String>>,

        #[property(get = Self::has_tables, type = bool, name = "has-tables")]
        _has_tables: (),
        #[property(get = Self::has_sequence, type = bool, name = "has-sequence")]
        _has_sequence: (),

        /// Target vertical alignment for the overlay.
        #[property(get, set, name = "valign-target", builder(gtk::Align::End))]
        pub valign_target: Cell<gtk::Align>,
    }

    impl Default for KeyStateOverlay {
        fn default() -> Self {
            Self {
                tables: RefCell::default(),
                sequence: RefCell::default(),
                _has_tables: (),
                _has_sequence: (),
                valign_target: Cell::new(gtk::Align::End),
            }
        }
    }

    impl KeyStateOverlay {
        fn set_tables(&self, v: Vec<String>) {
            self.tables.replace(v);
            // `tables` itself is notified by the derive; has-tables is ours.
            self.obj().notify_has_tables();
        }

        fn set_sequence(&self, v: Vec<String>) {
            self.sequence.replace(v);
            self.obj().notify_has_sequence();
        }

        fn has_tables(&self) -> bool {
            !self.tables.borrow().is_empty()
        }

        fn has_sequence(&self) -> bool {
            !self.sequence.borrow().is_empty()
        }
    }

    #[gtk::template_callbacks]
    impl KeyStateOverlay {
        #[template_callback(function)]
        fn show_chevron(has_tables: bool, has_sequence: bool) -> bool {
            has_tables && has_sequence
        }

        #[template_callback(function)]
        fn has_state(has_tables: bool, has_sequence: bool) -> bool {
            has_tables || has_sequence
        }

        #[template_callback(function)]
        #[allow(clippy::needless_pass_by_value)]
        fn tables_text(tables: Vec<String>) -> Option<String> {
            if tables.is_empty() {
                return None;
            }
            Some(tables.join(" > "))
        }

        #[template_callback(function)]
        #[allow(clippy::needless_pass_by_value)]
        fn sequence_text(sequence: Vec<String>) -> Option<String> {
            if sequence.is_empty() {
                return None;
            }
            Some(sequence.join(" "))
        }

        #[template_callback]
        fn on_drag_end(gesture: &gtk::GestureDrag, _offset_x: f64, offset_y: f64) {
            // Key state overlay only moves between top-center and
            // bottom-center. Horizontal alignment is always center.
            let Some(widget) = gesture.widget() else {
                return;
            };
            let Some(this) = widget.downcast_ref::<super::KeyStateOverlay>() else {
                return;
            };
            let Some(parent) = widget.parent() else {
                return;
            };

            let parent_height = f64::from(parent.height());
            let self_height = f64::from(widget.height());

            let self_y = if this.valign_target() == gtk::Align::Start {
                0.0
            } else {
                parent_height - self_height
            };
            let new_y = self_y + offset_y + (self_height / 2.0);

            let new_valign = if new_y > parent_height / 2.0 {
                gtk::Align::End
            } else {
                gtk::Align::Start
            };

            if new_valign != this.valign_target() {
                this.set_valign_target(new_valign);
                widget.queue_resize();
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for KeyStateOverlay {
        const NAME: &'static str = "GhosttyKeyStateOverlay";
        type Type = super::KeyStateOverlay;
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
    impl ObjectImpl for KeyStateOverlay {
        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for KeyStateOverlay {}
    impl BinImpl for KeyStateOverlay {}
}
