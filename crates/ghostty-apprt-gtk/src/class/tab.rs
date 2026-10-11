//! Port of apprt/gtk/class/tab.zig: `GhosttyTab`, the content of one tab
//! page (a split tree plus tab-level title/actions).

#![allow(clippy::wildcard_imports)]

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use ghostty_apprt::surface::NewSurfaceContext;
use gtk::gio;
use gtk::glib::{self, clone, subclass::Signal};

use super::application::Application;
use super::config::Config;
use super::split_tree::{SplitTree, SurfaceTree};
use super::surface::{Surface, SurfaceOverrides};
use super::title_dialog::{Target as TitleTarget, TitleDialog};

glib::wrapper! {
    pub struct Tab(ObjectSubclass<imp::Tab>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Tab {
    /// Create a new tab. If `config` is None the application config is used.
    /// An initial surface is created in the split tree.
    pub fn new(config: Option<&Config>, overrides: SurfaceOverrides) -> Self {
        let tab: Self = glib::Object::new();
        let imp = tab.imp();

        // If our configuration is null then we get the configuration from
        // the application.
        let config = config
            .cloned()
            .unwrap_or_else(|| Application::default().get_config());
        imp.config.replace(Some(config));
        tab.notify("config");

        // Create our initial surface in the split tree.
        if let Err(e) = imp.split_tree.new_split(
            ghostty_datastruct::split_tree::Direction::Right,
            None,
            overrides,
        ) {
            // TODO: show a nicer "no surfaces" state. Incredibly unlikely.
            panic!("oom creating initial split: {e}");
        }

        tab
    }

    /// Set the parent of this tab page. This only affects the first surface
    /// ever created for a tab. If a surface was already created this does
    /// nothing.
    pub fn set_parent(&self, parent: &Surface) {
        self.set_parent_with_context(parent, NewSurfaceContext::Tab);
    }

    pub fn set_parent_with_context(&self, parent: &Surface, context: NewSurfaceContext) {
        if let Some(surface) = self.get_active_surface() {
            surface.set_parent(parent, context);
        }
    }

    /// Overridden title. Generally shown over the title unless unset.
    pub fn set_title_override(&self, title: Option<&str>) {
        self.imp().title_override.replace(title.map(str::to_owned));
        self.notify("title-override");
    }

    pub fn get_title_override(&self) -> Option<String> {
        self.imp().title_override.borrow().clone()
    }

    pub fn get_title(&self) -> Option<String> {
        self.imp().title.borrow().clone()
    }

    pub fn prompt_tab_title(&self) {
        let imp = self.imp();
        let initial = imp
            .title_override
            .borrow()
            .clone()
            .or_else(|| imp.title.borrow().clone());
        let dialog = TitleDialog::new(TitleTarget::Tab, initial.as_deref());
        dialog.connect_set(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, title| this.set_title_override(if title.is_empty() {
                None
            } else {
                Some(title)
            })
        ));
        dialog.present(self);
    }

    /// Get the currently active surface. See the "active-surface" property.
    pub fn get_active_surface(&self) -> Option<Surface> {
        self.get_split_tree().get_active_surface()
    }

    /// Get the surface tree of this tab.
    pub fn get_surface_tree(&self) -> Option<super::split_tree::Tree> {
        self.get_split_tree().get_tree()
    }

    /// Get the split tree widget that is in this tab.
    pub fn get_split_tree(&self) -> SplitTree {
        self.imp().split_tree.get()
    }

    /// Get the config this tab was created with.
    pub fn get_config(&self) -> Option<Config> {
        self.imp().config.borrow().clone()
    }

    /// Returns true if this tab needs confirmation before quitting based on
    /// the various Ghostty configurations.
    pub fn get_needs_confirm_quit(&self) -> bool {
        self.get_split_tree().get_needs_confirm_quit()
    }

    /// Connect to the `close-request` signal, emitted whenever the tab would
    /// like to be closed.
    pub fn connect_close_request<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("close-request", false, move |args| {
            if let Some(Ok(this)) = args.first().map(glib::Value::get::<Tab>) {
                f(&this);
            }
            None
        })
    }

    /// Get the tab view holding this tab, if any.
    fn get_tab_view(&self) -> Option<adw::TabView> {
        self.ancestor(adw::TabView::static_type())
            .and_then(|w| w.downcast().ok())
    }

    /// Get the tab page holding this tab, if any.
    fn get_tab_page(&self) -> Option<adw::TabPage> {
        Some(self.get_tab_view()?.page(self))
    }

    fn init_action_map(&self) {
        let group = gio::SimpleActionGroup::new();

        let close = gio::SimpleAction::new("close", Some(glib::VariantTy::STRING));
        close.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, param| this.action_close(param.and_then(|p| p.str()))
        ));
        group.add_action(&close);

        let ring_bell = gio::SimpleAction::new("ring-bell", None);
        ring_bell.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| {
                // If the page is selected already we don't mark it as
                // needing attention. We only want to mark unfocused pages.
                // This will then clear when the page is selected.
                let Some(page) = this.get_tab_page() else {
                    return;
                };
                if page.is_selected() {
                    return;
                }
                page.set_needs_attention(true);
            }
        ));
        group.add_action(&ring_bell);

        let next = gio::SimpleAction::new("next-page", None);
        next.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| {
                if let Some(v) = this.get_tab_view() {
                    v.select_next_page();
                }
            }
        ));
        group.add_action(&next);

        let prev = gio::SimpleAction::new("previous-page", None);
        prev.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| {
                if let Some(v) = this.get_tab_view() {
                    v.select_previous_page();
                }
            }
        ));
        group.add_action(&prev);

        let prompt = gio::SimpleAction::new("prompt-tab-title", None);
        prompt.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| this.prompt_tab_title()
        ));
        group.add_action(&prompt);

        self.insert_action_group("tab", Some(&group));
    }

    fn action_close(&self, mode: Option<&str>) {
        let Some(mode) = mode else {
            log::warn!("tab.close-tab called without a parameter");
            return;
        };
        let Some(tab_view) = self.get_tab_view() else {
            return;
        };
        let page = tab_view.page(self);

        // Delegate to our parent to handle this, since this will emit a
        // close-page signal that the parent can intercept.
        match mode {
            "this" => tab_view.close_page(&page),
            "other" => tab_view.close_other_pages(&page),
            "right" => tab_view.close_pages_after(&page),
            // Need to be defensive here since actions can be triggered
            // externally.
            _ => log::warn!("invalid mode provided to tab.close-tab: {mode}"),
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/tab.ui")]
    pub struct Tab {
        /// The configuration that this tab is using.
        pub config: RefCell<Option<Config>>,

        /// The title of this tab. This is usually bound to the active surface.
        pub title: RefCell<Option<String>>,

        /// The manually overridden title from `prompt_tab_title`.
        pub title_override: RefCell<Option<String>>,

        /// The tooltip of this tab. This is usually bound to the active surface.
        pub tooltip: RefCell<Option<String>>,

        #[template_child]
        pub split_tree: TemplateChild<SplitTree>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Tab {
        const NAME: &'static str = "GhosttyTab";
        type Type = super::Tab;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            SplitTree::ensure_type();
            Surface::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl Tab {
        #[template_callback]
        fn notify_active_surface(split: &SplitTree, _pspec: glib::ParamSpec) {
            if let Some(tab) = split.ancestor(super::Tab::static_type()) {
                tab.notify("active-surface");
            }
        }

        #[template_callback]
        fn notify_tree(split: &SplitTree, _pspec: glib::ParamSpec) {
            let Some(tab) = split
                .ancestor(super::Tab::static_type())
                .and_then(|w| w.downcast::<super::Tab>().ok())
            else {
                return;
            };
            tab.notify("surface-tree");

            // If our tree is empty we close the tab.
            if tab.get_surface_tree().is_none_or(|t| t.is_empty()) {
                tab.emit_by_name::<()>("close-request", &[]);
            }
        }

        #[template_callback(function)]
        fn computed_title(
            config: Option<Config>,
            terminal: Option<String>,
            surface_override: Option<String>,
            tab_override: Option<String>,
            zoomed: bool,
            bell_ringing: bool,
        ) -> String {
            // Our plain title is the manually tab overridden title if it
            // exists, otherwise the overridden title if it exists,
            // otherwise the terminal title if it exists, otherwise a
            // default string.
            let config_title = config.as_ref().and_then(|c| c.get().title.clone());
            let plain = tab_override
                .or(surface_override)
                .or(terminal)
                .or(config_title)
                .unwrap_or_else(|| "Ghostty".to_owned());

            // We don't need a config in every case, but if we don't have one
            // let's just assume something went terribly wrong and use our
            // default title.
            let Some(config) = config else {
                log::warn!("config unavailable for computed title, likely bug");
                return plain;
            };
            let config = config.get();

            let mut out = String::new();

            // If our bell is ringing, then we prefix the bell icon.
            if bell_ringing && config.bell_features.title {
                out.push_str("🔔 ");
            }

            // If we're zoomed, prefix with the magnifying glass emoji.
            if zoomed {
                out.push_str("🔍 ");
            }

            out.push_str(&plain);
            out
        }
    }

    impl ObjectImpl for Tab {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPS: std::sync::OnceLock<Vec<glib::ParamSpec>> = std::sync::OnceLock::new();
            PROPS.get_or_init(|| {
                vec![
                    glib::ParamSpecObject::builder::<Surface>("active-surface")
                        .read_only()
                        .build(),
                    glib::ParamSpecObject::builder::<Config>("config").build(),
                    glib::ParamSpecObject::builder::<SplitTree>("split-tree")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoxed::builder::<SurfaceTree>("surface-tree")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("tooltip").build(),
                    glib::ParamSpecString::builder("title").build(),
                    glib::ParamSpecString::builder("title-override").build(),
                ]
            })
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<Signal>> = std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                // Emitted whenever the tab would like to be closed.
                vec![Signal::builder("close-request").build()]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();
            match pspec.name() {
                "active-surface" => obj.get_active_surface().to_value(),
                "config" => self.config.borrow().to_value(),
                "split-tree" => self.split_tree.get().to_value(),
                "surface-tree" => obj.get_surface_tree().map(SurfaceTree).to_value(),
                "tooltip" => self.tooltip.borrow().to_value(),
                "title" => self.title.borrow().to_value(),
                "title-override" => self.title_override.borrow().to_value(),
                _ => unreachable!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "config" => {
                    self.config.replace(value.get().expect("Config"));
                }
                "tooltip" => {
                    self.tooltip.replace(value.get().expect("string"));
                }
                "title" => {
                    self.title.replace(value.get().expect("string"));
                }
                "title-override" => {
                    self.title_override.replace(value.get().expect("string"));
                }
                _ => unreachable!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.obj().init_action_map();
        }

        fn dispose(&self) {
            self.config.replace(None);
            self.dispose_template();
        }
    }

    impl WidgetImpl for Tab {}
    impl BoxImpl for Tab {}
}
