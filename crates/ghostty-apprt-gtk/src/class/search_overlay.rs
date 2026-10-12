//! Port of apprt/gtk/class/search_overlay.zig.
//!
//! The terminal search bar overlay. Emits `stop-search`, `search-changed`
//! (with the entry text, may be `None`), `next-match` and `previous-match`.

use std::sync::OnceLock;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::subclass::Signal};

glib::wrapper! {
    pub struct SearchOverlay(ObjectSubclass<imp::SearchOverlay>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SearchOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchOverlay {
    #[must_use]
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Grab focus on the search entry and select all text.
    pub fn grab_focus_entry(&self) {
        let entry = &self.imp().search_entry;
        entry.grab_focus();
        // -1 is distance from the end, causing the entire text to be selected.
        entry.select_region(0, -1);
    }

    /// Set contents of search and emit `search-changed`.
    pub fn set_search_contents(&self, content: &str) {
        self.imp().search_entry.set_text(content);
        self.emit_by_name::<()>("search-changed", &[&Some(content.to_owned())]);
    }

    /// Set the total number of search matches.
    pub fn set_search_total(&self, total: Option<usize>) {
        let imp = self.imp();
        let had_total = imp.search_total.get().is_some();
        if imp.search_total.get() == total {
            return;
        }
        imp.search_total.set(total);
        self.notify_search_total();
        if had_total != total.is_some() {
            self.notify_has_search_total();
        }
    }

    /// Set the currently selected match index.
    pub fn set_search_selected(&self, selected: Option<usize>) {
        let imp = self.imp();
        let had_selected = imp.search_selected.get().is_some();
        if imp.search_selected.get() == selected {
            return;
        }
        imp.search_selected.set(selected);
        self.notify_search_selected();
        if had_selected != selected.is_some() {
            self.notify_has_search_selected();
        }
    }

    pub fn connect_stop_search<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "stop-search",
            false,
            glib::closure_local!(move |this: &Self| f(this)),
        )
    }

    pub fn connect_search_changed<F: Fn(&Self, Option<String>) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "search-changed",
            false,
            glib::closure_local!(move |this: &Self, text: Option<String>| f(this, text)),
        )
    }

    pub fn connect_next_match<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "next-match",
            false,
            glib::closure_local!(move |this: &Self| f(this)),
        )
    }

    pub fn connect_previous_match<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "previous-match",
            false,
            glib::closure_local!(move |this: &Self| f(this)),
        )
    }
}

mod imp {
    use std::cell::Cell;

    #[allow(clippy::wildcard_imports)]
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::SearchOverlay)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/search-overlay.ui")]
    pub struct SearchOverlay {
        /// The search entry widget.
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,

        /// True when a search is active, meaning we should show the overlay.
        #[property(get, set = Self::set_active)]
        pub active: Cell<bool>,

        /// Total number of search matches (None means unknown/none).
        pub search_total: Cell<Option<usize>>,

        /// Currently selected match index (None means none selected).
        pub search_selected: Cell<Option<usize>>,

        #[property(get = Self::get_search_total, type = u64, name = "search-total")]
        _p_total: (),
        #[property(get = Self::get_has_search_total, type = bool, name = "has-search-total")]
        _p_has_total: (),
        #[property(get = Self::get_search_selected, type = u64, name = "search-selected")]
        _p_selected: (),
        #[property(get = Self::get_has_search_selected, type = bool, name = "has-search-selected")]
        _p_has_selected: (),

        /// Target horizontal alignment for the overlay.
        #[property(get, set, name = "halign-target", builder(gtk::Align::End))]
        pub halign_target: Cell<gtk::Align>,

        /// Target vertical alignment for the overlay.
        #[property(get, set, name = "valign-target", builder(gtk::Align::Start))]
        pub valign_target: Cell<gtk::Align>,
    }

    impl Default for SearchOverlay {
        fn default() -> Self {
            Self {
                search_entry: TemplateChild::default(),
                active: Cell::new(false),
                search_total: Cell::new(None),
                search_selected: Cell::new(None),
                _p_total: (),
                _p_has_total: (),
                _p_selected: (),
                _p_has_selected: (),
                halign_target: Cell::new(gtk::Align::End),
                valign_target: Cell::new(gtk::Align::Start),
            }
        }
    }

    impl SearchOverlay {
        // Set active status, and update search on activation
        fn set_active(&self, active: bool) {
            if !self.active.get() && active {
                let text = self.search_entry.text().to_string();
                self.obj()
                    .emit_by_name::<()>("search-changed", &[&Some(text)]);
            }
            self.active.set(active);
        }

        fn get_search_total(&self) -> u64 {
            self.search_total.get().unwrap_or(0) as u64
        }

        fn get_has_search_total(&self) -> bool {
            self.search_total.get().is_some()
        }

        fn get_search_selected(&self) -> u64 {
            self.search_selected.get().unwrap_or(0) as u64
        }

        fn get_has_search_selected(&self) -> bool {
            self.search_selected.get().is_some()
        }
    }

    /// Find the overlay that contains `w` (the Zig callbacks receive it as
    /// user data).
    fn overlay_of(w: &impl IsA<gtk::Widget>) -> Option<super::SearchOverlay> {
        w.ancestor(super::SearchOverlay::static_type())
            .and_downcast()
    }

    #[gtk::template_callbacks]
    impl SearchOverlay {
        #[template_callback(function)]
        fn match_label_closure(
            has_selected: bool,
            selected: u64,
            has_total: bool,
            total: u64,
        ) -> String {
            if !has_total || total == 0 {
                return "0/0".to_owned();
            }
            format!("{}/{}", if has_selected { selected + 1 } else { 0 }, total)
        }

        #[template_callback]
        fn search_changed(entry: &gtk::SearchEntry) {
            let Some(this) = overlay_of(entry) else {
                return;
            };
            let text = entry.text().to_string();
            this.emit_by_name::<()>("search-changed", &[&Some(text)]);
        }

        // NOTE: shared with multiple widgets in the template.
        #[template_callback]
        fn stop_search(w: &gtk::Widget) {
            if let Some(this) = overlay_of(w) {
                this.emit_by_name::<()>("stop-search", &[]);
            }
        }

        #[template_callback]
        fn next_match(w: &gtk::Widget) {
            if let Some(this) = overlay_of(w) {
                this.emit_by_name::<()>("next-match", &[]);
            }
        }

        #[template_callback]
        fn previous_match(w: &gtk::Widget) {
            if let Some(this) = overlay_of(w) {
                this.emit_by_name::<()>("previous-match", &[]);
            }
        }

        #[template_callback]
        fn search_entry_key_pressed(
            ctrl: &gtk::EventControllerKey,
            keyval: gdk::Key,
            _keycode: u32,
            mods: gdk::ModifierType,
        ) -> bool {
            let Some(this) = ctrl.widget().and_then(|w| overlay_of(&w)) else {
                return false;
            };
            if keyval == gdk::Key::Return || keyval == gdk::Key::KP_Enter {
                if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                    this.emit_by_name::<()>("previous-match", &[]);
                } else {
                    this.emit_by_name::<()>("next-match", &[]);
                }
                return true;
            }
            false
        }

        #[template_callback]
        fn on_drag_end(gesture: &gtk::GestureDrag, offset_x: f64, offset_y: f64) {
            // On drag end, move halign/valign if we crossed the midpoint on
            // either axis. This lets the overlay be moved to different
            // corners of the parent container.
            let Some(widget) = gesture.widget() else {
                return;
            };
            let Some(this) = widget.downcast_ref::<super::SearchOverlay>() else {
                return;
            };
            let Some(parent) = widget.parent() else {
                return;
            };

            let parent_width = f64::from(parent.width());
            let parent_height = f64::from(parent.height());
            let self_width = f64::from(widget.width());
            let self_height = f64::from(widget.height());

            let self_x = if this.halign_target() == gtk::Align::Start {
                0.0
            } else {
                parent_width - self_width
            };
            let self_y = if this.valign_target() == gtk::Align::Start {
                0.0
            } else {
                parent_height - self_height
            };

            let new_x = self_x + offset_x + (self_width / 2.0);
            let new_y = self_y + offset_y + (self_height / 2.0);

            let new_halign = if new_x > parent_width / 2.0 {
                gtk::Align::End
            } else {
                gtk::Align::Start
            };
            let new_valign = if new_y > parent_height / 2.0 {
                gtk::Align::End
            } else {
                gtk::Align::Start
            };

            let mut changed = false;
            if new_halign != this.halign_target() {
                this.set_halign_target(new_halign);
                changed = true;
            }
            if new_valign != this.valign_target() {
                this.set_valign_target(new_valign);
                changed = true;
            }
            if changed {
                widget.queue_resize();
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchOverlay {
        const NAME: &'static str = "GhosttySearchOverlay";
        type Type = super::SearchOverlay;
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
    impl ObjectImpl for SearchOverlay {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted when the search is stopped (e.g., Escape pressed).
                    Signal::builder("stop-search").build(),
                    // Emitted when the search text changes.
                    Signal::builder("search-changed")
                        .param_types([Option::<String>::static_type()])
                        .build(),
                    Signal::builder("next-match").build(),
                    Signal::builder("previous-match").build(),
                ]
            })
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for SearchOverlay {}
    impl BinImpl for SearchOverlay {}
}
