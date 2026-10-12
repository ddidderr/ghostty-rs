//! Port of apprt/gtk/class/window.zig (`GhosttyWindow`, an
//! `AdwApplicationWindow` subclass).
//!
//! Properties: `active-surface`, `config`, `debug`, `titlebar-style`,
//! `headerbar-visible`, `quick-terminal`, `tabs-autohide`, `tabs-wide`,
//! `tabs-visible`, `toolbar-style`. Actions (`win.*`): about, close,
//! close-tab(s), new-tab, new-window, prompt-surface-title,
//! prompt-tab-title, prompt-context-tab-title, ring-bell, split-{right,left,
//! up,down}, copy, paste, reset, clear, toggle-command-palette,
//! toggle-inspector.
//!
//! Deviations: the parent surface of `new_tab` & friends is the GTK
//! [`Surface`] object (Zig took the `*CoreSurface` and used
//! `rt_surface.gobj()`); the core surface is looked up via `Surface::core()`.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_apprt::surface::NewSurfaceContext;
use ghostty_config::{
    command::Command as ConfigCommand,
    types_a::WindowSubtitle,
    types_b::{
        GtkTabsLocation,
        GtkTitlebarStyle,
        GtkToolbarStyle,
        WindowDecoration,
        WindowNewTabPosition,
        WindowShowTabBar,
        WindowTheme,
    },
};
use ghostty_input::binding::{Action as BindingAction, CloseTabMode};
use ghostty_os::i18n::tr;
use gtk::{
    CompositeTemplate,
    TemplateChild,
    gio,
    glib::{self, clone},
};

use super::{
    application::Application,
    close_confirmation_dialog::{CloseConfirmationDialog, Target as CloseTarget},
    command_palette::CommandPalette,
    config::Config,
    debug_warning::DebugWarning,
    split_tree::{SplitTree, SurfaceTree, Tree},
    surface::{Surface, SurfaceOverrides},
    tab::Tab,
};
use crate::{
    adw_version,
    gtk_version,
    winproto::{self, WindowHost},
};

/// Which tab to select (Zig: `Window.SelectTab`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectTab {
    Previous,
    Next,
    Last,
    /// 1-indexed.
    N(usize),
}

/// Mirrors the Zig `overrides` struct of `newTabForWindow`.
#[derive(Debug, Clone, Default)]
pub struct TabOverrides {
    pub command: Option<ConfigCommand>,
    pub working_directory: Option<String>,
    pub title: Option<String>,
}

/// GEnum mirror of `configpkg.Config.GtkTitlebarStyle` (used by the
/// `titlebar-style` property and a blueprint closure).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "GhosttyWindowTitlebarStyle")]
#[repr(i32)]
pub enum TitlebarStyle {
    #[default]
    Native,
    Tabs,
}

impl From<GtkTitlebarStyle> for TitlebarStyle {
    fn from(v: GtkTitlebarStyle) -> Self {
        match v {
            GtkTitlebarStyle::Native => TitlebarStyle::Native,
            GtkTitlebarStyle::Tabs => TitlebarStyle::Tabs,
        }
    }
}

/// Host adapter handed to winproto (holds only a weak reference so the
/// winproto state owned by the window does not create a cycle).
struct Host(glib::WeakRef<Window>);

impl WindowHost for Host {
    fn gtk_window(&self) -> Option<gtk::Window> {
        self.0.upgrade().map(glib::object::Cast::upcast)
    }

    fn is_quick_terminal(&self) -> bool {
        self.0.upgrade().is_some_and(|w| w.is_quick_terminal())
    }

    fn window_decoration(&self) -> WindowDecoration {
        self.0
            .upgrade()
            .map_or(WindowDecoration::Auto, |w| w.get_window_decoration())
    }

    fn config(&self) -> Option<std::sync::Arc<ghostty_config::config::Config>> {
        self.0
            .upgrade()
            .and_then(|w| w.get_config())
            .map(|c| c.get())
    }
}

mod imp {
    use std::marker::PhantomData;

    use adw::{prelude::*, subclass::prelude::*};

    use super::{
        Cell,
        CommandPalette,
        CompositeTemplate,
        Config,
        DebugWarning,
        NewSurfaceContext,
        RefCell,
        SplitTree,
        Surface,
        Tab,
        TabOverrides,
        TemplateChild,
        TitlebarStyle,
        Window as WindowObj,
        WindowDecoration,
        glib,
        gtk_version,
    };
    use crate::winproto;

    #[derive(glib::Properties, CompositeTemplate)]
    #[properties(wrapper_type = WindowObj)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/window.ui")]
    pub struct Window {
        /// The active surface is the focus that should be receiving all
        /// surface-targeted actions. This is usually the focused surface,
        /// but may also not be focused if the user has selected a
        /// non-surface widget.
        #[property(name = "active-surface", get = Self::get_active_surface, type = Option<Surface>)]
        #[property(name = "debug", get = Self::get_debug, type = bool)]
        #[property(name = "titlebar-style", get = Self::get_titlebar_style, type = TitlebarStyle, builder(TitlebarStyle::Native))]
        #[property(name = "headerbar-visible", get = Self::get_headerbar_visible, type = bool)]
        #[property(name = "tabs-autohide", get = Self::get_tabs_autohide, type = bool)]
        #[property(name = "tabs-wide", get = Self::get_tabs_wide, type = bool)]
        #[property(name = "tabs-visible", get = Self::get_tabs_visible, type = bool)]
        #[property(name = "toolbar-style", get = Self::get_toolbar_style, type = adw::ToolbarStyle, builder(adw::ToolbarStyle::Raised))]
        _props: PhantomData<()>,

        /// The configuration that this window is using.
        #[property(get, set, nullable)]
        pub config: RefCell<Option<Config>>,

        /// Whether this window is a quick terminal. If it is then it
        /// behaves slightly differently under certain scenarios.
        #[property(name = "quick-terminal", get, set, default = false)]
        pub quick_terminal: Cell<bool>,

        /// The window decoration override. If this is not set then we'll
        /// inherit whatever the config has.
        #[allow(clippy::struct_field_names)]
        pub window_decoration: Cell<Option<WindowDecoration>>,

        /// Binding for our active tab (Zig: `tab_bindings` BindingGroup).
        pub tab_binding: RefCell<Option<glib::Binding>>,

        /// State and logic for windowing protocol for a window.
        pub winproto: RefCell<winproto::Window>,

        /// Whether we've initialized any single surface yet. We gate
        /// default size on this so that we don't resize the window after
        /// surfaces already exist.
        pub surface_init: Cell<bool>,

        /// See tab_overview_open for why we have this.
        pub tab_overview_focus_timer: RefCell<Option<glib::SourceId>>,

        /// A weak reference to a command palette.
        pub command_palette: glib::WeakRef<CommandPalette>,

        /// Tab page that the context menu was opened for (set by
        /// `setup-menu`).
        pub context_menu_page: RefCell<Option<adw::TabPage>>,

        /// Signal handlers connected to surfaces (so we can disconnect them
        /// by "data == self" like the Zig code).
        pub surface_handlers: RefCell<Vec<(glib::WeakRef<Surface>, Vec<glib::SignalHandlerId>)>>,

        /// Signal handlers connected to tabs (close-request).
        pub tab_handlers: RefCell<Vec<(glib::WeakRef<Tab>, glib::SignalHandlerId)>>,

        /// Handlers on split trees (changed), keyed by tab.
        pub tree_handlers: RefCell<Vec<(glib::WeakRef<SplitTree>, glib::SignalHandlerId)>>,

        #[template_child]
        pub tab_overview: TemplateChild<adw::TabOverview>,
        #[template_child]
        pub tab_bar: TemplateChild<adw::TabBar>,
        #[template_child]
        pub tab_view: TemplateChild<adw::TabView>,
        #[template_child]
        pub toolbar: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                _props: PhantomData,
                config: RefCell::new(None),
                quick_terminal: Cell::new(false),
                window_decoration: Cell::new(None),
                tab_binding: RefCell::new(None),
                winproto: RefCell::new(winproto::Window::None(winproto::noop::Window)),
                surface_init: Cell::new(false),
                tab_overview_focus_timer: RefCell::new(None),
                command_palette: glib::WeakRef::new(),
                context_menu_page: RefCell::new(None),
                surface_handlers: RefCell::new(Vec::new()),
                tab_handlers: RefCell::new(Vec::new()),
                tree_handlers: RefCell::new(Vec::new()),
                tab_overview: TemplateChild::default(),
                tab_bar: TemplateChild::default(),
                tab_view: TemplateChild::default(),
                toolbar: TemplateChild::default(),
                toast_overlay: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "GhosttyWindow";
        type Type = WindowObj;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            DebugWarning::ensure_type();
            SplitTree::ensure_type();
            Surface::ensure_type();
            Tab::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // If our configuration is null then we get the configuration
            // from the application.
            let config = {
                if self.config.borrow().is_none() {
                    let app = super::Application::default();
                    self.config.replace(Some(app.get_config()));
                }
                self.config.borrow().as_ref().map(Config::get)
            };

            // Add our dev CSS class if we're in debug mode.
            if cfg!(debug_assertions) {
                obj.add_css_class("devel");
            }

            // The tab binding (certain properties are only synced from the
            // currently active tab) is set up in `tab_view_selected_page`.

            // Set our window icon. We can't set this in the blueprint file
            // because its dependent on the build config.
            obj.set_icon_name(Some(crate::APPLICATION_ID));

            // Initialize our actions
            obj.init_action_map();

            // Start states based on config.
            if let Some(config) = config {
                if config.maximize {
                    obj.maximize();
                }
                if config.fullscreen != ghostty_config::types_a::Fullscreen::False {
                    obj.fullscreen();
                }
                // If we have an explicit title set, we set that immediately
                // so that any applications inspecting the window states see
                // an immediate title set when the window appears.
                if let Some(title) = &config.title {
                    obj.set_title(Some(title));
                }
            }

            // We always sync our appearance at the end because loading our
            // config and such can affect our bindings which are setup
            // initially in init_template.
            obj.sync_appearance();

            // We need to do this so that the title initializes properly,
            // I think because its a dynamic getter.
            obj.notify("active-surface");
        }

        fn dispose(&self) {
            self.command_palette.set(None::<&CommandPalette>);
            self.config.replace(None);
            if let Some(b) = self.tab_binding.take() {
                b.unbind();
            }
            self.dispose_template();
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}

    // ---- property getters ----------------------------------------
    impl Window {
        fn get_active_surface(&self) -> Option<Surface> {
            self.obj().get_active_surface()
        }

        #[allow(clippy::unused_self)]
        fn get_debug(&self) -> bool {
            cfg!(debug_assertions)
        }

        fn cfg(&self) -> Option<std::sync::Arc<ghostty_config::config::Config>> {
            self.config.borrow().as_ref().map(Config::get)
        }

        fn get_headerbar_visible(&self) -> bool {
            let obj = self.obj();

            // Never display the header bar when CSDs are disabled.
            if !self.winproto.borrow().client_side_decoration_enabled() {
                return false;
            }

            // Never display the header bar as a quick terminal.
            if self.quick_terminal.get() {
                return false;
            }

            // If we're fullscreen we never show the header bar.
            if obj.is_fullscreen() {
                return false;
            }

            // The remainder needs a config
            let Some(config) = self.cfg() else {
                return true;
            };

            // *Conditionally* disable the header bar when maximized, and
            // gtk-titlebar-hide-when-maximized is set
            if obj.is_maximized() && config.gtk_titlebar_hide_when_maximized {
                return false;
            }

            match config.gtk_titlebar_style {
                // If the titlebar style is tabs never show the titlebar.
                GtkTitlebarStyleAlias::Tabs => false,
                // If native show the titlebar if configured to do so.
                GtkTitlebarStyleAlias::Native => config.gtk_titlebar,
            }
        }

        fn get_tabs_autohide(&self) -> bool {
            let Some(config) = self.cfg() else {
                return true;
            };
            match config.gtk_titlebar_style {
                // If the titlebar style is tabs we cannot autohide.
                GtkTitlebarStyleAlias::Tabs => false,
                GtkTitlebarStyleAlias::Native => match config.window_show_tab_bar {
                    // Auto we always autohide... obviously.
                    super::WindowShowTabBar::Auto => true,
                    // Always we never autohide because we always show it.
                    super::WindowShowTabBar::Always => false,
                    // Never we autohide because it doesn't actually matter,
                    // since get_tabs_visible will return false.
                    super::WindowShowTabBar::Never => true,
                },
            }
        }

        fn get_tabs_visible(&self) -> bool {
            let Some(config) = self.cfg() else {
                return true;
            };
            match config.gtk_titlebar_style {
                GtkTitlebarStyleAlias::Tabs => {
                    // *Conditionally* disable the tab bar when maximized,
                    // the titlebar style is tabs, and
                    // gtk-titlebar-hide-when-maximized is set.
                    if self.obj().is_maximized() && config.gtk_titlebar_hide_when_maximized {
                        return false;
                    }
                    // If the titlebar style is tabs the tab bar must always
                    // be visible.
                    true
                }
                GtkTitlebarStyleAlias::Native => match config.window_show_tab_bar {
                    super::WindowShowTabBar::Always | super::WindowShowTabBar::Auto => true,
                    super::WindowShowTabBar::Never => false,
                },
            }
        }

        fn get_tabs_wide(&self) -> bool {
            self.cfg().is_none_or(|c| c.gtk_wide_tabs)
        }

        fn get_toolbar_style(&self) -> adw::ToolbarStyle {
            let Some(config) = self.cfg() else {
                return adw::ToolbarStyle::Raised;
            };
            match config.gtk_toolbar_style {
                super::GtkToolbarStyle::Flat => adw::ToolbarStyle::Flat,
                super::GtkToolbarStyle::Raised => adw::ToolbarStyle::Raised,
                super::GtkToolbarStyle::RaisedBorder => adw::ToolbarStyle::RaisedBorder,
            }
        }

        fn get_titlebar_style(&self) -> TitlebarStyle {
            self.cfg()
                .map_or(TitlebarStyle::Native, |c| c.gtk_titlebar_style.into())
        }
    }

    use ghostty_config::types_b::GtkTitlebarStyle as GtkTitlebarStyleAlias;

    // ---- template callbacks -------------------------------------
    //
    // GtkBuilder passes `[sender, args.., template_instance]` for signal
    // handlers (swapped="no"), which is why the instance is the last
    // parameter. Closures (`function`) get the expression arguments only.
    #[gtk::template_callbacks]
    impl Window {
        #[template_callback]
        fn realize(_w: &gtk::Widget, win: &WindowObj) {
            win.on_realize();
        }

        #[template_callback]
        fn new_tab(_b: &adw::SplitButton, win: &WindowObj) {
            win.perform_binding_action(&ghostty_input::binding::Action::NewTab);
        }

        #[template_callback]
        fn overview_create_tab(_o: &adw::TabOverview, win: &WindowObj) -> adw::TabPage {
            let parent = win.get_active_surface();
            win.new_tab_page(
                parent.as_ref(),
                NewSurfaceContext::Tab,
                TabOverrides::default(),
            )
        }

        #[template_callback]
        fn overview_notify_open(
            overview: &adw::TabOverview,
            _pspec: &glib::ParamSpec,
            win: &WindowObj,
        ) {
            win.tab_overview_open(overview);
        }

        #[template_callback]
        fn close_request(_w: &gtk::Window, win: &WindowObj) -> glib::Propagation {
            win.window_close_request()
        }

        #[template_callback]
        fn close_page(
            _v: &adw::TabView,
            page: &adw::TabPage,
            win: &WindowObj,
        ) -> glib::Propagation {
            win.tab_view_close_page(page)
        }

        #[template_callback]
        fn page_attached(_v: &adw::TabView, page: &adw::TabPage, _pos: i32, win: &WindowObj) {
            win.tab_view_page_attached(page);
        }

        #[template_callback]
        fn page_detached(_v: &adw::TabView, page: &adw::TabPage, _pos: i32, win: &WindowObj) {
            win.tab_view_page_detached(page);
        }

        #[template_callback]
        fn setup_tab_menu(_v: &adw::TabView, page: Option<&adw::TabPage>, win: &WindowObj) {
            win.imp().context_menu_page.replace(page.cloned());
        }

        #[template_callback]
        fn tab_create_window(_v: &adw::TabView, _win: &WindowObj) -> adw::TabView {
            // Create a new window without creating a new tab.
            let new_win = WindowObj::new_empty(&super::Application::default());

            // We have to show it otherwise it'll just be hidden.
            new_win.present();

            // Get our tab view
            new_win.tab_view()
        }

        #[template_callback]
        fn notify_n_pages(_v: &adw::TabView, _pspec: &glib::ParamSpec, win: &WindowObj) {
            let imp = win.imp();
            if imp.tab_view.n_pages() == 0 {
                // If we have no pages left then we want to close window.

                // If the tab overview is open, then we don't close the
                // window because its a rather abrupt experience. This also
                // fixes an issue where dragging out the last tab in the tab
                // overview won't cause Ghostty to exit.
                if imp.tab_overview.is_open() {
                    return;
                }

                win.close();
            }
        }

        #[template_callback]
        fn notify_selected_page(_v: &adw::TabView, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.tab_view_selected_page();
        }

        #[template_callback]
        fn notify_config(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.prop_config();
        }

        #[template_callback]
        fn notify_fullscreened(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.sync_appearance();
        }

        #[template_callback]
        fn notify_is_active(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.prop_is_active();
        }

        #[template_callback]
        fn notify_maximized(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.sync_appearance();
        }

        #[template_callback]
        fn notify_menu_active(button: &gtk::MenuButton, _pspec: &glib::ParamSpec, win: &WindowObj) {
            // Debian 12 is stuck on GTK 4.8
            if !gtk_version::at_least(4, 10, 0) {
                return;
            }

            // We only care if we're activating. If we're activating then
            // we need to check the validity of our menu items.
            if !button.is_active() {
                return;
            }

            win.sync_actions();
        }

        #[template_callback]
        fn notify_quick_terminal(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            win.prop_quick_terminal();
        }

        #[template_callback]
        fn notify_scale_factor(_w: &WindowObj, _pspec: &glib::ParamSpec, win: &WindowObj) {
            // On some platforms (namely X11) we need to refresh our
            // appearance when the scale factor changes.
            if let Err(err) = win.imp().winproto.borrow_mut().sync_appearance() {
                log::warn!("failed to sync appearance after scale factor has been updated={err}");
            }
        }

        #[template_callback(function)]
        fn titlebar_style_is_tabs(value: TitlebarStyle) -> bool {
            value == TitlebarStyle::Tabs
        }

        #[template_callback(function)]
        fn computed_subtitle(config: Option<Config>, pwd: Option<String>) -> Option<String> {
            let config = config?.get();
            match config.window_subtitle {
                super::WindowSubtitle::False => None,
                super::WindowSubtitle::WorkingDirectory => pwd,
            }
        }
    }
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    /// Create a new window for `app`. If `title` is set it is applied
    /// immediately (`overrides.title`). The window starts with no tabs; use
    /// [`Window::new_tab`] / [`Window::new_tab_for_window`].
    pub fn new(app: &Application, title: Option<&str>) -> Self {
        crate::register_resources();
        let win: Self = glib::Object::builder().property("application", app).build();
        if let Some(title) = title {
            // If the overrides have a title set, we set that immediately
            // so that any applications inspecting the window states see an
            // immediate title set when the window appears, rather than
            // waiting possibly a few event loop ticks for it to sync from
            // the surface.
            win.set_title(Some(title));
        }
        win
    }

    /// Create a window flagged as a quick terminal (Zig:
    /// `newInstance(Window, .{ .application = app, .quick-terminal = true })`).
    pub fn new_quick_terminal(app: &Application) -> Self {
        crate::register_resources();
        glib::Object::builder()
            .property("application", app)
            .property("quick-terminal", true)
            .build()
    }

    fn new_empty(app: &Application) -> Self {
        Self::new(app, None)
    }

    // ---- accessors -------------------------------------------------

    /// Winproto backend for this window.
    pub fn winproto(&self) -> std::cell::RefMut<'_, winproto::Window> {
        self.imp().winproto.borrow_mut()
    }

    /// Whether this terminal is a quick terminal or not.
    pub fn is_quick_terminal(&self) -> bool {
        self.imp().quick_terminal.get()
    }

    /// Get the currently active surface. See the "active-surface" property.
    pub fn get_active_surface(&self) -> Option<Surface> {
        self.get_selected_tab()?.get_active_surface()
    }

    /// Returns the configuration for this window.
    pub fn get_config(&self) -> Option<Config> {
        self.imp().config.borrow().clone()
    }

    /// Get the tab view for this window.
    pub fn tab_view(&self) -> adw::TabView {
        self.imp().tab_view.get()
    }

    /// Alias for [`Window::tab_view`] (Zig: `getTabView`).
    pub fn get_tab_view(&self) -> adw::TabView {
        self.tab_view()
    }

    /// Get the current window decoration value for this window.
    pub fn get_window_decoration(&self) -> WindowDecoration {
        let imp = self.imp();
        if let Some(v) = imp.window_decoration.get() {
            return v;
        }
        if let Some(c) = imp.config.borrow().as_ref() {
            return c.get().window_decoration;
        }
        WindowDecoration::Auto
    }

    /// Toggle the window decorations for this window.
    pub fn toggle_window_decorations(&self) {
        let imp = self.imp();

        if imp.window_decoration.get().is_some() {
            // Unset any previously set window decoration settings
            self.set_window_decoration(None);
            return;
        }

        let Some(config) = self.get_config() else {
            return;
        };
        self.set_window_decoration(Some(match config.get().window_decoration {
            // Use auto when the decoration is initially none
            WindowDecoration::None => WindowDecoration::Auto,
            // Anything non-none to none
            WindowDecoration::Auto | WindowDecoration::Client | WindowDecoration::Server => {
                WindowDecoration::None
            }
        }));
    }

    /// Set the window decoration override for this window. If this is None,
    /// then we'll revert back to the configuration's default.
    fn set_window_decoration(&self, new: Option<WindowDecoration>) {
        self.imp().window_decoration.set(new);
        self.sync_appearance();
    }

    fn get_selected_tab(&self) -> Option<Tab> {
        let page = self.imp().tab_view.selected_page()?;
        let child = page.child();
        debug_assert!(child.is::<Tab>());
        child.downcast::<Tab>().ok()
    }

    /// Returns true if this window needs confirmation before quitting.
    pub fn get_needs_confirm_quit(&self) -> bool {
        let tab_view = &self.imp().tab_view;
        for i in 0..tab_view.n_pages() {
            let page = tab_view.nth_page(i);
            let Ok(tab) = page.child().downcast::<Tab>() else {
                log::warn!("unexpected non-Tab child in tab view");
                continue;
            };
            if tab.get_needs_confirm_quit() {
                return true;
            }
        }
        false
    }

    fn is_fullscreen_(&self) -> bool {
        gtk::prelude::GtkWindowExt::is_fullscreen(self)
    }

    fn is_maximized_(&self) -> bool {
        gtk::prelude::GtkWindowExt::is_maximized(self)
    }

    // ---- tabs --------------------------------------------------------

    /// Create a new tab with the given parent. The tab will be inserted at
    /// the position dictated by the `window-new-tab-position` config. The
    /// new tab will be selected.
    pub fn new_tab(&self, parent: Option<&Surface>) {
        self.new_tab_page(parent, NewSurfaceContext::Tab, TabOverrides::default());
    }

    /// Create the first tab of a new window (inherits the parent's initial
    /// size hints).
    pub fn new_tab_for_window(&self, parent: Option<&Surface>, overrides: TabOverrides) {
        self.new_tab_page(parent, NewSurfaceContext::Window, overrides);
    }

    fn new_tab_page(
        &self,
        parent: Option<&Surface>,
        context: NewSurfaceContext,
        overrides: TabOverrides,
    ) -> adw::TabPage {
        let imp = self.imp();
        let tab_view = &imp.tab_view;

        // Create our new tab object
        let config = imp.config.borrow().clone();
        let tab = Tab::new(
            config.as_ref(),
            SurfaceOverrides {
                command: overrides.command,
                working_directory: overrides.working_directory,
                title: overrides.title,
            },
        );

        if let Some(p) = parent {
            // For a new window's first tab, inherit the parent's initial
            // size hints.
            if context == NewSurfaceContext::Window {
                self.surface_init(p);
            }
            tab.set_parent_with_context(p, context);
        }

        // Get the position that we should insert the new tab at.
        let Some(config) = config else {
            // If we don't have a config we just append it at the end. This
            // should never happen.
            return tab_view.append(&tab);
        };
        let position = match config.get().window_new_tab_position {
            WindowNewTabPosition::Current => match tab_view.selected_page() {
                Some(selected) => tab_view.page_position(&selected) + 1,
                None => tab_view.n_pages(),
            },
            WindowNewTabPosition::End => tab_view.n_pages(),
        };

        // Add the page and select it
        let page = tab_view.insert(&tab, position);
        tab_view.set_selected_page(&page);

        // Create some property bindings
        tab.bind_property("title", &page, "title")
            .sync_create()
            .build();
        tab.bind_property("tooltip", &page, "tooltip")
            .sync_create()
            .build();

        // Bind signals
        let split_tree = tab.get_split_tree();
        let id = split_tree.connect_closure(
            "changed",
            false,
            glib::closure_local!(
                #[weak(rename_to = win)]
                self,
                move |_st: &SplitTree, old: Option<SurfaceTree>, new: Option<SurfaceTree>| {
                    win.tab_split_tree_changed(
                        old.as_ref().map(|t| &t.0),
                        new.as_ref().map(|t| &t.0),
                    );
                }
            ),
        );
        imp.tree_handlers
            .borrow_mut()
            .push((split_tree.downgrade(), id));

        // Run an initial notification for the surface tree so we can setup
        // initial state.
        self.tab_split_tree_changed(None, split_tree.get_tree().as_ref());

        page
    }

    /// Select the tab as requested. Returns true if the tab selection
    /// changed.
    pub fn select_tab(&self, n: SelectTab) -> bool {
        let tab_view = &self.imp().tab_view;

        // Get our current tab numeric position
        let Some(selected) = tab_view.selected_page() else {
            return false;
        };
        let current = tab_view.page_position(&selected);

        // Get our total
        let total = tab_view.n_pages();

        let goto: i32 = match n {
            SelectTab::Previous => {
                if current > 0 {
                    current - 1
                } else {
                    total - 1
                }
            }
            SelectTab::Next => {
                if current < total - 1 {
                    current + 1
                } else {
                    0
                }
            }
            SelectTab::Last => total - 1,
            SelectTab::N(v) => {
                // 1-indexed
                if v == 0 {
                    return false;
                }
                let Ok(n_int) = i32::try_from(v) else {
                    return false;
                };
                (n_int - 1).min(total - 1)
            }
        };
        debug_assert!(goto >= 0);
        debug_assert!(goto < total);

        // If our target is the same as our current then we do nothing.
        if goto == current {
            return false;
        }

        // Add the page and select it
        let page = tab_view.nth_page(goto);
        tab_view.set_selected_page(&page);

        true
    }

    /// Move the tab containing the given surface by the given amount.
    /// Returns if this affected any tab positioning.
    pub fn move_tab(&self, surface: &Surface, amount: isize) -> bool {
        let tab_view = &self.imp().tab_view;

        // If we have one tab we never move.
        let total = tab_view.n_pages();
        if total == 1 {
            return false;
        }

        // Get the tab that contains the given surface.
        let Some(tab) = get_ancestor::<Tab>(surface) else {
            return false;
        };

        // Get the page position that contains the tab.
        let page = tab_view.page(&tab);
        let pos = tab_view.page_position(&page);

        // Move it
        let initial = pos + amount as i32;
        let max = total - 1;
        let desired_pos = if initial < 0 {
            max + initial + 1
        } else if initial > max {
            initial - max - 1
        } else {
            initial
        };
        debug_assert!(desired_pos >= 0);
        debug_assert!(desired_pos < total);

        tab_view.reorder_page(&page, desired_pos)
    }

    pub fn toggle_tab_overview(&self) {
        let overview = &self.imp().tab_overview;
        overview.set_open(!overview.is_open());
    }

    /// Toggle the visible property.
    pub fn toggle_visibility(&self) {
        self.set_visible(!self.is_visible());
    }

    // ---- appearance ---------------------------------------------------

    /// Updates various appearance properties. This should always be safe to
    /// call multiple times. This should be called whenever a change happens
    /// that might affect how the window appears (config change, fullscreen,
    /// etc.).
    fn sync_appearance(&self) {
        let imp = self.imp();

        // Toggle style classes based on whether we're using CSDs or SSDs.
        //
        // These classes are defined in the gtk.Window documentation:
        // https://docs.gtk.org/gtk4/class.Window.html#css-nodes.
        {
            // Reset all style classes first
            for class in ["ssd", "csd", "solid-csd", "no-border-radius"] {
                self.remove_css_class(class);
            }

            let csd_enabled = imp.winproto.borrow().client_side_decoration_enabled();
            self.set_decorated(csd_enabled);

            if csd_enabled {
                let display = WidgetExt::display(self);

                // We do the exact same check GTK is doing internally and
                // toggle either the `csd` or `solid-csd` style, based on
                // whether the user's window manager is deemed
                // _non-compositing_.
                //
                // In practice this only impacts users of traditional X11
                // window managers (e.g. i3, dwm, awesomewm, etc.) and not
                // X11 desktop environments or Wayland compositors/DEs.
                if display.is_rgba() && display.is_composited() {
                    self.add_css_class("csd");
                } else {
                    self.add_css_class("solid-csd");
                }
            } else {
                self.add_css_class("ssd");
                // Fix any artifacting that may occur in window corners.
                self.add_css_class("no-border-radius");
            }
        }

        // Trigger all our dynamic properties that depend on the config.
        for key in [
            "headerbar-visible",
            "tabs-autohide",
            "tabs-visible",
            "tabs-wide",
            "toolbar-style",
            "titlebar-style",
        ] {
            self.notify(key);
        }

        // Remainder uses the config
        let Some(config) = self.get_config() else {
            return;
        };
        let config = config.get();

        // Only add a solid background if we're opaque.
        self.toggle_css_class("background", config.background_opacity >= 1.0);

        // Apply class to color headerbar if window-theme is set to `ghostty`
        // and GTK version is before 4.16. The conditional is because above
        // 4.16 we use GTK CSS color variables.
        self.toggle_css_class(
            "window-theme-ghostty",
            !gtk_version::at_least(4, 16, 0) && config.window_theme == WindowTheme::Ghostty,
        );

        // Move the tab bar to the proper location.
        imp.toolbar.remove(&*imp.tab_bar);
        match config.gtk_tabs_location {
            GtkTabsLocation::Top => imp.toolbar.add_top_bar(&*imp.tab_bar),
            GtkTabsLocation::Bottom => imp.toolbar.add_bottom_bar(&*imp.tab_bar),
        }

        // Do our window-protocol specific appearance sync.
        if let Err(err) = imp.winproto.borrow_mut().sync_appearance() {
            log::warn!("failed to sync winproto appearance error={err}");
        }
    }

    /// Sync the state of any actions on this window.
    fn sync_actions(&self) {
        let has_selection = self
            .get_active_surface()
            .and_then(|s| s.core())
            .is_some_and(|c| c.has_selection());

        let Some(action) = self
            .lookup_action("copy")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        else {
            return;
        };
        action.set_enabled(has_selection);
    }

    fn toggle_css_class(&self, class: &str, value: bool) {
        if value {
            self.add_css_class(class);
        } else {
            self.remove_css_class(class);
        }
    }

    /// Perform a binding action on the window's active surface.
    fn perform_binding_action(&self, action: &BindingAction) {
        let Some(core) = self.get_active_surface().and_then(|s| s.core()) else {
            return;
        };
        if let Err(err) = core.perform_binding_action(action) {
            log::warn!("error performing binding action error={err}");
        }
    }

    /// Queue a simple text-based toast. All text-based toasts share the
    /// same timeout for consistency.
    fn add_toast(&self, title: &str) {
        let toast = adw::Toast::new(title);
        toast.set_timeout(3);
        self.imp().toast_overlay.add_toast(toast);
    }

    // ---- surface handlers ------------------------------------------------

    fn connect_surface_handlers(&self, tree: &Tree) {
        let imp = self.imp();
        for entry in tree.iterator() {
            let surface = entry.view;

            // Before adding any new signal handlers, disconnect any that we
            // may have added before. Otherwise we may get multiple handlers
            // for the same signal.
            self.disconnect_surface(surface);

            let mut ids = Vec::new();

            ids.push(surface.connect_closure(
                "present-request",
                false,
                glib::closure_local!(
                    #[weak(rename_to = win)]
                    self,
                    move |surface: &Surface| win.surface_present_request(surface)
                ),
            ));
            ids.push(surface.connect_closure(
                "clipboard-write",
                false,
                glib::closure_local!(
                    #[weak(rename_to = win)]
                    self,
                    move |_surface: &Surface, clipboard: glib::Value, text: &str| {
                        win.surface_clipboard_write(&clipboard, text);
                    }
                ),
            ));
            ids.push(surface.connect_closure(
                "menu",
                false,
                glib::closure_local!(
                    #[weak(rename_to = win)]
                    self,
                    move |_surface: &Surface| win.sync_actions()
                ),
            ));
            ids.push(surface.connect_closure(
                "toggle-fullscreen",
                false,
                glib::closure_local!(
                    #[weak(rename_to = win)]
                    self,
                    move |_surface: &Surface| {
                        if win.is_fullscreen_() {
                            win.unfullscreen();
                        } else {
                            win.fullscreen();
                        }
                        // We react to the changes in the fullscreened notify
                    }
                ),
            ));
            ids.push(surface.connect_closure(
                "toggle-maximize",
                false,
                glib::closure_local!(
                    #[weak(rename_to = win)]
                    self,
                    move |_surface: &Surface| {
                        if win.is_maximized_() {
                            win.unmaximize();
                        } else {
                            win.maximize();
                        }
                        // We react to the changes in the maximized notify
                    }
                ),
            ));

            // If we've never had a surface initialize yet, then we register
            // this signal. Its theoretically possible to launch multiple
            // surfaces before init so we could register this on multiple
            // and that is not a problem because we'll check the flag again
            // in each handler.
            if !imp.surface_init.get() {
                ids.push(surface.connect_closure(
                    "init",
                    false,
                    glib::closure_local!(
                        #[weak(rename_to = win)]
                        self,
                        move |surface: &Surface| win.surface_init(surface)
                    ),
                ));
            }

            imp.surface_handlers
                .borrow_mut()
                .push((surface.downgrade(), ids));
        }
    }

    /// Disconnect all handlers we added to `surface`.
    fn disconnect_surface(&self, surface: &Surface) {
        let mut handlers = self.imp().surface_handlers.borrow_mut();
        let mut i = 0;
        while i < handlers.len() {
            let same = handlers[i].0.upgrade().is_none_or(|s| &s == surface);
            if same {
                let (weak, ids) = handlers.swap_remove(i);
                if let Some(s) = weak.upgrade() {
                    for id in ids {
                        s.disconnect(id);
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Disconnect all the surface handlers for the given tree. This should
    /// be called whenever a tree is no longer present in the window, e.g.
    /// when a tab is detached or the tree changes.
    fn disconnect_surface_handlers(&self, tree: &Tree) {
        for entry in tree.iterator() {
            self.disconnect_surface(entry.view);
        }
    }

    fn tab_split_tree_changed(&self, old: Option<&Tree>, new: Option<&Tree>) {
        if let Some(tree) = old {
            self.disconnect_surface_handlers(tree);
        }
        if let Some(tree) = new {
            self.connect_surface_handlers(tree);
        }
    }

    // ---- notify / signal handlers ------------------------------------

    fn prop_config(&self) {
        if let Some(config) = self.get_config()
            && config.get().app_notifications.config_reload
        {
            self.add_toast(&tr("Reloaded the configuration"));
        }
        self.sync_appearance();
    }

    fn prop_is_active(&self) {
        // Hide quick-terminal if set to autohide
        if self.is_quick_terminal()
            && let Some(cfg) = self.get_config()
            && cfg.get().quick_terminal_autohide
            && !self.is_active()
        {
            self.toggle_visibility();
        }

        // Don't change urgency if we're not the active window.
        if !self.is_active() {
            return;
        }

        if let Err(err) = self.winproto().set_urgent(false) {
            log::warn!("winproto failed to reset urgency={err}");
        }
    }

    fn prop_quick_terminal(&self) {
        let imp = self.imp();
        if imp.surface_init.get() {
            log::warn!(
                "quick terminal property can't be changed after surfaces have been initialized"
            );
            return;
        }

        if imp.quick_terminal.get() {
            // Initialize the quick terminal at the app-layer
            if let Err(err) = Application::default()
                .winproto_mut()
                .init_quick_terminal(self.upcast_ref())
            {
                log::warn!("failed to initialize quick terminal error={err}");
            }
        }
    }

    fn on_realize(&self) {
        let app = Application::default();

        // Initialize our window protocol logic
        let host: Rc<dyn WindowHost> = Rc::new(Host(self.downgrade()));
        match winproto::Window::init(&app.winproto(), host) {
            Ok(wp) => {
                self.imp().winproto.replace(wp);
            }
            Err(err) => {
                log::warn!("failed to initialize window protocol error={err}");
                return;
            }
        }

        // We need to setup resize notifications on our surface, which is
        // only available after the window had been realized.
        if let Some(gdk_surface) = gtk::prelude::NativeExt::surface(self) {
            for prop in ["width", "height"] {
                gdk_surface.connect_notify_local(
                    Some(prop),
                    clone!(
                        #[weak(rename_to = win)]
                        self,
                        move |_, _| {
                            // X11 needs to fix blurring on resize, but
                            // winproto implementations could do anything.
                            if let Err(err) = win.imp().winproto.borrow_mut().resize_event() {
                                log::warn!("winproto resize event failed error={err}");
                            }
                        }
                    ),
                );
            }
        }

        // When we are realized we always setup our appearance since this
        // calls some winproto functions.
        self.sync_appearance();
    }

    fn tab_overview_open(&self, overview: &adw::TabOverview) {
        // We only care about when the tab overview is closed.
        if overview.is_open() {
            return;
        }

        // On tab overview close, focus is sometimes lost. This is an
        // upstream issue in libadwaita[1]. When this is resolved we can put
        // a runtime version check here to avoid this workaround.
        //
        // Our workaround is to start a timer after 500ms to refocus the
        // currently selected tab. We choose 500ms because the adw animation
        // is 400ms.
        //
        // [1]: https://gitlab.gnome.org/GNOME/libadwaita/-/issues/670

        // If we have an old timer remove it
        let imp = self.imp();
        if let Some(timer) = imp.tab_overview_focus_timer.take() {
            timer.remove();
        }

        // Restart our timer
        let id = glib::timeout_add_local(
            Duration::from_millis(500),
            clone!(
                #[weak(rename_to = win)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    // Always note our timer is removed
                    win.imp().tab_overview_focus_timer.replace(None);

                    // Get our currently active surface which should respect
                    // the newly selected tab. Grab focus.
                    if let Some(surface) = win.get_active_surface() {
                        surface.grab_focus();
                    }

                    // Remove the timer
                    glib::ControlFlow::Break
                }
            ),
        );
        imp.tab_overview_focus_timer.replace(Some(id));
    }

    fn window_close_request(&self) -> glib::Propagation {
        if self.get_needs_confirm_quit() {
            // Show a confirmation dialog
            let dialog = CloseConfirmationDialog::new(CloseTarget::Window);
            dialog.connect_close_request(clone!(
                #[weak(rename_to = win)]
                self,
                move |_| win.destroy()
            ));

            // Show it
            dialog.present(Some(self));
            return glib::Propagation::Stop;
        }

        self.destroy();
        glib::Propagation::Proceed
    }

    fn tab_view_close_page(&self, page: &adw::TabPage) -> glib::Propagation {
        let tab_view = &self.imp().tab_view;
        let child = page.child();
        let Ok(tab) = child.clone().downcast::<Tab>() else {
            return glib::Propagation::Proceed;
        };

        // If the tab says it doesn't need confirmation then we go ahead and
        // close immediately.
        if !tab.get_needs_confirm_quit() {
            tab_view.close_page_finish(page, true);
            return glib::Propagation::Stop;
        }

        // Show a confirmation dialog
        let dialog = CloseConfirmationDialog::new(CloseTarget::Tab);
        dialog.connect_close_request(clone!(
            #[strong]
            page,
            move |_| finish_close_page(&page, true)
        ));
        dialog.connect_cancel(clone!(
            #[strong]
            page,
            move |_| finish_close_page(&page, false)
        ));

        // Show it
        dialog.present(Some(&child));
        glib::Propagation::Stop
    }

    fn tab_view_selected_page(&self) {
        let imp = self.imp();

        // Always reset our binding source in case we have no pages.
        if let Some(b) = imp.tab_binding.take() {
            b.unbind();
        }

        // Get our current page which MUST be a Tab object.
        let Some(page) = imp.tab_view.selected_page() else {
            return;
        };
        let child = page.child();
        debug_assert!(child.is::<Tab>());

        // Setup our binding group. This ensures things like the title are
        // synced from the active tab.
        imp.tab_binding
            .replace(Some(child.bind_property("title", self, "title").build()));

        // If the tab was previously marked as needing attention (e.g. due
        // to a bell character), we now unmark that
        page.set_needs_attention(false);
    }

    fn tab_view_page_attached(&self, page: &adw::TabPage) {
        // Get the attached page which must be a Tab object.
        let Ok(tab) = page.child().downcast::<Tab>() else {
            return;
        };

        // Attach listeners for the tab.
        let id = tab.connect_close_request(clone!(
            #[weak(rename_to = win)]
            self,
            move |tab| win.tab_close_request(tab)
        ));
        self.imp()
            .tab_handlers
            .borrow_mut()
            .push((tab.downgrade(), id));

        // Attach listeners for the surface.
        //
        // Interesting behavior here that was previously undocumented but
        // I'm going to make it explicit here: we accept all the signals
        // here (like toggle-fullscreen) regardless of whether the surface
        // or tab is focused. At the time of writing this we have no API
        // that could really trigger these that way but its theoretically
        // possible.
        //
        // What is DEFINITELY possible is something like OSC52 triggering a
        // clipboard-write signal on an unfocused tab/surface. We
        // definitely want to show the user a notification about that but
        // our notification right now is a toast that doesn't make it clear
        // WHO used the clipboard.
        if let Some(tree) = tab.get_surface_tree() {
            self.connect_surface_handlers(&tree);
        }
    }

    fn tab_view_page_detached(&self, page: &adw::TabPage) {
        // We need to get the tab to disconnect the signals.
        let Ok(tab) = page.child().downcast::<Tab>() else {
            return;
        };
        {
            let mut handlers = self.imp().tab_handlers.borrow_mut();
            let mut i = 0;
            while i < handlers.len() {
                if handlers[i].0.upgrade().is_none_or(|t| t == tab) {
                    let (weak, id) = handlers.swap_remove(i);
                    if let Some(t) = weak.upgrade() {
                        t.disconnect(id);
                    }
                } else {
                    i += 1;
                }
            }
        }

        // Remove the tree handlers
        if let Some(tree) = tab.get_surface_tree() {
            self.disconnect_surface_handlers(&tree);
        }
    }

    fn tab_close_request(&self, tab: &Tab) {
        let tab_view = &self.imp().tab_view;
        let page = tab_view.page(tab);
        // TODO: connect close page handler to tab to check for confirmation
        tab_view.close_page(&page);
    }

    fn surface_clipboard_write(&self, clipboard: &glib::Value, text: &str) {
        // We only toast for the standard clipboard.
        let is_standard = glib::EnumValue::from_value(clipboard)
            .is_some_and(|(_, v)| v.value() == 0 || v.nick() == "standard");
        if !is_standard {
            return;
        }

        // We only toast if configured to
        let Some(config) = self.get_config() else {
            return;
        };
        if !config.get().app_notifications.clipboard_copy {
            return;
        }

        if text.is_empty() {
            self.add_toast(&tr("Cleared clipboard"));
        } else {
            self.add_toast(&tr("Copied to clipboard"));
        }
    }

    fn surface_present_request(&self, surface: &Surface) {
        // Verify that this surface is actually in this window.
        match get_ancestor::<Window>(surface) {
            None => {
                log::warn!("present request called for non-existent surface");
                return;
            }
            Some(w) if &w != self => {
                log::warn!("present request called for surface in different window");
                return;
            }
            Some(_) => {}
        }

        // Get the tab for this surface.
        let Some(tab) = get_ancestor::<Tab>(surface) else {
            log::warn!("present request surface not found");
            return;
        };

        // Get the page that contains this tab
        let tab_view = &self.imp().tab_view;
        let page = tab_view.page(&tab);
        tab_view.set_selected_page(&page);

        // Grab focus
        surface.grab_focus();

        // Bring the window to the front.
        self.present();
    }

    fn surface_init(&self, surface: &Surface) {
        let imp = self.imp();

        // Make sure we init only once
        if imp.surface_init.get() {
            return;
        }
        imp.surface_init.set(true);

        // Setup our default and minimum size.
        if let Some(size) = surface.get_default_size() {
            self.set_default_size(size.width as i32, size.height as i32);
        }
        if let Some(size) = surface.get_min_size() {
            self.set_size_request(size.width as i32, size.height as i32);
        }
    }

    // ---- actions ---------------------------------------------------------

    fn init_action_map(&self) {
        let simple = |name: &str, f: fn(&Window)| {
            gio::ActionEntry::builder(name)
                .activate(move |win: &Window, _, _| f(win))
                .build()
        };
        let binding = |name: &str, action: BindingAction| {
            gio::ActionEntry::builder(name)
                .activate(move |win: &Window, _, _| win.perform_binding_action(&action))
                .build()
        };

        self.add_action_entries([
            simple("about", Window::action_about),
            gio::ActionEntry::builder("close")
                .activate(|win: &Window, _, _| win.close())
                .build(),
            gio::ActionEntry::builder("close-tab")
                .parameter_type(Some(glib::VariantTy::STRING))
                .activate(|win: &Window, _, param| win.action_close_tab(param))
                .build(),
            binding("new-tab", BindingAction::NewTab),
            binding("new-window", BindingAction::NewWindow),
            binding("prompt-surface-title", BindingAction::PromptSurfaceTitle),
            binding("prompt-tab-title", BindingAction::PromptTabTitle),
            simple(
                "prompt-context-tab-title",
                Window::action_prompt_context_tab_title,
            ),
            simple("ring-bell", Window::action_ring_bell),
            binding(
                "split-right",
                BindingAction::NewSplit(ghostty_input::binding::SplitDirection::Right),
            ),
            binding(
                "split-left",
                BindingAction::NewSplit(ghostty_input::binding::SplitDirection::Left),
            ),
            binding(
                "split-up",
                BindingAction::NewSplit(ghostty_input::binding::SplitDirection::Up),
            ),
            binding(
                "split-down",
                BindingAction::NewSplit(ghostty_input::binding::SplitDirection::Down),
            ),
            binding(
                "copy",
                BindingAction::CopyToClipboard(ghostty_input::binding::CopyToClipboard::Mixed),
            ),
            binding("paste", BindingAction::PasteFromClipboard),
            binding("reset", BindingAction::Reset),
            binding("clear", BindingAction::ClearScreen),
            // TODO: accept the surface that toggled the command palette
            simple("toggle-command-palette", Window::toggle_command_palette),
            simple("toggle-inspector", Window::toggle_inspector),
        ]);
    }

    fn action_about(&self) {
        let name = "Ghostty";
        let icon = "com.mitchellh.ghostty";
        let website = "https://ghostty.org";

        // libadwaita >= 1.5 is assumed, so AdwAboutDialog is always used
        // (the Zig GtkAboutDialog fallback is unreachable here).
        let _ = adw_version::supports_dialogs();
        let dialog = adw::AboutDialog::builder()
            .application_name(name)
            .developer_name(tr("Ghostty Developers"))
            .application_icon(icon)
            .version(env!("CARGO_PKG_VERSION"))
            .issue_url("https://github.com/ghostty-org/ghostty/issues")
            .website(website)
            .build();
        dialog.present(Some(self));
    }

    fn action_close_tab(&self, param: Option<&glib::Variant>) {
        let Some(param) = param else {
            log::warn!("win.close-tab called without a parameter");
            return;
        };
        let Some(s) = param.str() else {
            log::warn!("invalid mode provided to win.close-tab");
            return;
        };
        let Some(mode) = CloseTabMode::from_name(s) else {
            log::warn!("invalid mode provided to win.close-tab: {s}");
            return;
        };
        self.perform_binding_action(&BindingAction::CloseTab(mode));
    }

    fn action_prompt_context_tab_title(&self) {
        let Some(page) = self.imp().context_menu_page.borrow().clone() else {
            return;
        };
        let Ok(tab) = page.child().downcast::<Tab>() else {
            return;
        };
        tab.prompt_tab_title();
    }

    fn action_ring_bell(&self) {
        let Some(config) = self.get_config() else {
            return;
        };
        let config = config.get();

        if config.bell_features.system {
            match gtk::prelude::NativeExt::surface(self) {
                Some(native) => native.beep(),
                None => log::warn!("unable to get native surface from window"),
            }
        }

        // Dont set urgency if the window is already active.
        if config.bell_features.attention && !self.is_active() {
            // Request user attention
            if let Err(err) = self.winproto().set_urgent(true) {
                log::warn!("winproto failed to set urgency={err}");
            }
        }
    }

    /// Toggle the command palette.
    ///
    /// TODO: accept the surface that toggled the command palette as a
    /// parameter
    fn toggle_command_palette(&self) {
        let imp = self.imp();

        // Get a reference to a command palette. First check the weak
        // reference that we save to see if we already have one stored. If
        // we don't then create a new one.
        let palette = if let Some(p) = imp.command_palette.upgrade() {
            p
        } else {
            // Create a fresh command palette.
            let palette = CommandPalette::new();

            // Synchronize our config to the command palette's config.
            self.bind_property("config", &palette, "config")
                .sync_create()
                .build();

            // Listen to the trigger signal to know if the user selected an
            // option in the command palette.
            palette.connect_trigger(clone!(
                #[weak(rename_to = win)]
                self,
                move |_, action| {
                    // If the activation actually has an action, perform it.
                    win.perform_binding_action(action);
                }
            ));

            // Save a weak reference to the command palette. We use a weak
            // reference to avoid reference counting cycles that might cause
            // problems later.
            imp.command_palette.set(Some(&palette));
            palette
        };

        // Tell the command palette to toggle itself. If the dialog gets
        // presented (instead of hidden) it will be modal over our window.
        palette.toggle(self);
    }

    /// Toggle the Ghostty inspector for the active surface.
    fn toggle_inspector(&self) {
        let Some(surface) = self.get_active_surface() else {
            return;
        };
        surface.control_inspector(ghostty_apprt::action::Inspector::Toggle);
    }
}

/// Find the first ancestor of `widget` of type `T` (Zig: `ext.getAncestor`).
fn get_ancestor<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Option<T> {
    widget
        .ancestor(T::static_type())
        .and_then(|w| w.downcast::<T>().ok())
}

fn finish_close_page(page: &adw::TabPage, confirm: bool) {
    let Some(tab_view) = get_ancestor::<adw::TabView>(&page.child()) else {
        log::warn!("close confirmation called for non-existent page");
        return;
    };
    tab_view.close_page_finish(page, confirm);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titlebar_style_conversion() {
        assert_eq!(
            TitlebarStyle::from(GtkTitlebarStyle::Native),
            TitlebarStyle::Native
        );
        assert_eq!(
            TitlebarStyle::from(GtkTitlebarStyle::Tabs),
            TitlebarStyle::Tabs
        );
    }

    #[test]
    fn select_tab_n_zero_is_noop_variant() {
        assert_eq!(SelectTab::N(0), SelectTab::N(0));
        assert_ne!(SelectTab::Next, SelectTab::Previous);
    }
}
