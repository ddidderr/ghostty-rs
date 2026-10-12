//! `GhosttyApplication` (ZIG/src/apprt/gtk/class/application.zig): the
//! primary entrypoint of the GTK runtime. Wraps an `AdwApplication`, owns the
//! config, windowing-protocol state, CSS providers and global shortcuts, and
//! implements the apprt action switch ([`Application::perform_action`]).
//!
//! Public API used by the other classes:
//! * [`Application::default`] -> the running application
//! * [`Application::core`] / [`Application::rt`] -> `Rc<dyn CoreApp>` / `Rc<dyn RtApp>`
//! * [`Application::get_config`] -> `class::config::Config` object (property `config`)
//! * [`Application::core_config`] -> `Arc<ghostty_config::config::Config>`
//! * [`Application::winproto`] / [`Application::winproto_mut`]
//! * [`Application::saved_language`], [`Application::wakeup`], [`Application::quit`]

#![allow(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::type_complexity,
    clippy::should_implement_trait
)]

use std::{
    cell::{Cell, OnceCell, Ref, RefCell, RefMut},
    rc::Rc,
    sync::Arc,
};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_apprt::{
    action::{self, Action, Target},
    runtime_api::{AppMessage, CoreApp, RtApp, SurfaceId},
    structs::ColorScheme,
};
use ghostty_config::{
    cli_args::ConfigField,
    command::Command,
    config::Config as CoreConfig,
    types_b::{GtkSingleInstance, WindowTheme},
};
use ghostty_input::binding::Action as BindingAction;
use gtk::{gdk, gio, glib};

use crate::{
    class::{
        close_confirmation_dialog::{self, CloseConfirmationDialog},
        config::Config,
        config_errors_dialog::ConfigErrorsDialog,
        global_shortcuts::GlobalShortcuts,
        split_tree::SplitTree,
        surface::{Size, Surface},
        tab::Tab,
        window::{SelectTab, TabOverrides, Window},
    },
    css::CssProviders,
    winproto,
};

/// Errors from [`Application::run_loop`].
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("unable to acquire the default main context")]
    ContextAcquireFailed,
    #[error("unable to register the application")]
    ApplicationRegisterFailed,
    #[error("core app tick failed: {0}")]
    Tick(String),
}

/// State of the quit-after-last-window-closed timer.
#[derive(Debug, Default)]
pub enum QuitTimer {
    #[default]
    Off,
    Active(glib::SourceId),
    Expired,
}

/// Overrides for a new window (Zig: `newWindow` overrides struct).
#[derive(Debug, Default, Clone)]
pub struct WindowOverrides {
    pub command: Option<Command>,
    pub working_directory: Option<String>,
    pub title: Option<String>,
}

// ---------------------------------------------------------------------
// Process level helpers

/// Set an environment variable.
#[allow(unsafe_code)]
fn set_env(key: &str, value: &str) {
    // SAFETY: only called during application construction on the main
    // thread, before GTK is initialized and before we spawn any child
    // processes; no other thread in this crate reads the environment.
    unsafe { std::env::set_var(key, value) }
}

/// Funnel GLib/GObject/GTK log messages into the `log` crate rather than
/// having them dumped directly to stderr.
fn glib_log_writer(level: glib::LogLevel, fields: &[glib::LogField<'_>]) -> glib::LogWriterOutput {
    let mut message = None;
    let mut domain = None;
    for f in fields {
        match f.key() {
            "MESSAGE" => message = f.value_str(),
            "GLIB_DOMAIN" => domain = f.value_str(),
            _ => {}
        }
    }
    let Some(message) = message else {
        return glib::LogWriterOutput::Unhandled;
    };
    let domain = domain.unwrap_or("«unknown»");
    match level {
        glib::LogLevel::Error => log::error!(target: "glib", "ERROR: {domain}: {message}"),
        glib::LogLevel::Critical => log::error!(target: "glib", "CRITICAL: {domain}: {message}"),
        glib::LogLevel::Warning => log::warn!(target: "glib", "WARNING: {domain}: {message}"),
        glib::LogLevel::Message => log::info!(target: "glib", "MESSAGE: {domain}: {message}"),
        glib::LogLevel::Info => log::info!(target: "glib", "INFO: {domain}: {message}"),
        glib::LogLevel::Debug => log::debug!(target: "glib", "DEBUG: {domain}: {message}"),
    }
    glib::LogWriterOutput::Handled
}

fn runtime_at_least(major: u32, minor: u32, micro: u32) -> bool {
    (
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
    ) >= (major, minor, micro)
}

/// Sets various GTK-related environment variables as necessary given the
/// runtime environment or configuration. Must be called BEFORE GTK
/// initialization.
fn set_gtk_env(config: &CoreConfig) {
    // gdk_debug flags
    let mut opengl = config.gtk_opengl_debug;
    let mut gl_disable_gles = false;
    let mut gl_no_fractional = false;
    let mut vulkan_disable = false;

    // gdk_disable flags
    let mut gles_api = false;
    // current gtk implementation for color management is not good enough.
    // see: https://bugs.kde.org/show_bug.cgi?id=495647
    let mut color_mgmt = true;
    let mut vulkan = false;

    'environment: {
        if runtime_at_least(4, 18, 0) {
            color_mgmt = false;
        }

        if runtime_at_least(4, 16, 0) {
            // From gtk 4.16, GDK_DEBUG is split into GDK_DEBUG and GDK_DISABLE.
            gles_api = true;
            vulkan = true;
            break 'environment;
        }
        if runtime_at_least(4, 14, 0) {
            // We need to export GDK_DEBUG to run on Wayland after GTK 4.14.
            // Upstream issue: https://gitlab.gnome.org/GNOME/gtk/-/issues/6589
            gl_disable_gles = true;
            vulkan_disable = true;
            if !runtime_at_least(4, 17, 5) {
                // Removed at GTK v4.17.5
                gl_no_fractional = true;
            }
            break 'environment;
        }
        // Versions prior to 4.14 are a bit of an unknown for Ghostty.
        vulkan_disable = true;
    }
    // `gtk-opengl-debug` dumps logs directly to stderr.
    opengl = opengl && std::io::IsTerminal::is_terminal(&std::io::stderr());

    let join = |flags: &[(&str, bool)]| {
        flags
            .iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",")
    };

    let gdk_debug = join(&[
        ("opengl", opengl),
        ("gl-disable-gles", gl_disable_gles),
        ("gl-no-fractional", gl_no_fractional),
        ("vulkan-disable", vulkan_disable),
    ]);
    log::warn!("setting GDK_DEBUG={gdk_debug}");
    set_env("GDK_DEBUG", &gdk_debug);

    let gdk_disable = join(&[
        ("gles-api", gles_api),
        ("color-mgmt", color_mgmt),
        ("vulkan", vulkan),
    ]);
    log::warn!("setting GDK_DISABLE={gdk_disable}");
    set_env("GDK_DISABLE", &gdk_disable);
}

/// Port of Ghostty's `os/locale.zig` `ensureLocale` (called from
/// `global.zig` at startup): set the process locale from the environment and,
/// if that fails (usually an invalid `LANG`), retry with `LANG` unset, then fall
/// back to `en_US.UTF-8` (updating `LANG` on success). Idempotent. This must
/// run before GTK initializes and before the child environment is captured.
#[allow(unsafe_code)]
pub fn init_locale() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: all strings are valid NUL-terminated C strings; this runs at
        // startup before other threads read the environment or locale.
        unsafe {
            fn show(p: *const libc::c_char) -> String {
                // SAFETY: setlocale returns a valid C string when non-null.
                unsafe { std::ffi::CStr::from_ptr(p) }
                    .to_string_lossy()
                    .into_owned()
            }

            // Set the locale to whatever is set in env vars.
            let r = libc::setlocale(libc::LC_ALL, c"".as_ptr());
            if !r.is_null() {
                log::info!("setlocale from env result={}", show(r));
                return;
            }

            // setlocale failed, probably because LANG is invalid. Try again
            // without LANG to use the system default.
            if std::env::var_os("LANG").is_some_and(|v| !v.is_empty()) {
                // Both so that LANG is either empty or unset completely.
                libc::setenv(c"LANG".as_ptr(), c"".as_ptr(), 1);
                libc::unsetenv(c"LANG".as_ptr());

                let r = libc::setlocale(libc::LC_ALL, c"".as_ptr());
                if !r.is_null() {
                    let v = show(r);
                    log::info!("setlocale after unset lang result={v}");

                    // An unsupported locale yields the "C" fallback; we
                    // prefer moving on to en_US.UTF-8 in that case.
                    if v != "C" {
                        return;
                    }
                }
            }

            // Failure again... fallback to en_US.UTF-8
            log::warn!(
                "setlocale failed with LANG and system default. Falling back to en_US.UTF-8"
            );
            let r = libc::setlocale(libc::LC_ALL, c"en_US.UTF-8".as_ptr());
            if r.is_null() {
                log::warn!("setlocale failed even with the fallback, uncertain results");
            } else {
                libc::setenv(c"LANG".as_ptr(), c"en_US.UTF-8".as_ptr(), 1);
                log::info!("setlocale default result={}", show(r));
            }
        }
    });
}

// ---------------------------------------------------------------------
// RtApp

/// The `RtApp` handed to core. Stateless; forwards to the default
/// application. Core must only call it from the main thread.
#[derive(Debug, Default)]
pub struct AppRt;

impl RtApp for AppRt {
    fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool {
        Application::default().perform_action(target, action)
    }
}

mod imp {
    use adw::{prelude::*, subclass::prelude::*};
    use gtk::gio;

    use super::{
        AppMessage,
        Cell,
        Config,
        ConfigErrorsDialog,
        CoreApp,
        CssProviders,
        GlobalShortcuts,
        OnceCell,
        QuitTimer,
        Rc,
        RefCell,
        RtApp,
        glib,
        winproto,
    };

    #[derive(Default)]
    pub struct Application {
        /// The libghostty App instance.
        pub core_app: OnceCell<Rc<dyn CoreApp>>,
        pub rt_app: OnceCell<Rc<dyn RtApp>>,

        /// The configuration for the application.
        pub config: RefCell<Option<Config>>,

        /// State and logic for the underlying windowing protocol.
        pub winproto: RefCell<Option<winproto::App>>,

        /// The global shortcut logic.
        pub global_shortcuts: OnceCell<GlobalShortcuts>,

        /// Set to true so long as we request a window exactly once. This
        /// prevents quitting the app before we've shown one window.
        pub requested_window: Cell<bool>,

        /// Set to false internally when the event loop should exit.
        pub running: Cell<bool>,

        pub quit_timer: RefCell<QuitTimer>,

        /// The config errors dialog (weak: it can close on its own).
        pub config_errors_dialog: glib::WeakRef<ConfigErrorsDialog>,

        pub signal_thread_started: Cell<bool>,

        /// CSS providers (runtime + custom).
        pub css: RefCell<Option<CssProviders>>,

        /// Copy of the LANG variable provided by the system, if any.
        pub saved_language: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "GhosttyApplication";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPS: std::sync::OnceLock<Vec<glib::ParamSpec>> = std::sync::OnceLock::new();
            PROPS.get_or_init(|| {
                vec![
                    glib::ParamSpecObject::builder::<Config>("config")
                        .read_only()
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "config" => self.config.borrow().to_value(),
                name => unimplemented!("unknown property {name}"),
            }
        }

        fn dispose(&self) {
            if let Some(diag) = self.config_errors_dialog.upgrade() {
                diag.close();
            }
        }
    }

    impl ApplicationImpl for Application {
        fn startup(&self) {
            log::debug!("startup");
            self.parent_startup();
            let obj = self.obj();

            // Set ourselves as the default application.
            gio::Application::set_default(obj.upcast_ref::<gio::Application>());

            // Setup our style manager (light/dark mode)
            obj.startup_style_manager();
            // Setup some signal handlers
            obj.startup_signals();
            // Setup our action map
            obj.startup_action_map();
            // Setup our global shortcuts
            obj.startup_global_shortcuts();

            // If we have any config diagnostics from loading, then we show
            // the diagnostics dialog. We show this one as a general modal
            // (not to any specific window) because we don't even know if the
            // window will load.
            obj.show_config_errors_dialog();
        }

        fn activate(&self) {
            log::debug!("activate");
            // Queue a new window
            if let Some(core) = self.core_app.get() {
                core.push_message(AppMessage::NewWindow);
            }
            self.parent_activate();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Application {
    /// Get the default (running) application.
    ///
    /// Panics if there is no default application or it is not a
    /// GhosttyApplication.
    pub fn default() -> Self {
        gio::Application::default()
            .and_then(|a| a.downcast::<Application>().ok())
            .expect("no default GhosttyApplication")
    }

    /// Creates a new Application instance. This does a lot more work than a
    /// typical class instantiation, because we expect that this is the main
    /// program entrypoint. Every error is mapped to some degraded state
    /// where we can at least show a window with an error.
    pub fn new(core_app: Rc<dyn CoreApp>) -> Self {
        init_locale();

        // Capture GLib/GObject/GTK log messages.
        glib::log_set_writer_func(glib_log_writer);

        // Load our configuration.
        let mut config = match CoreConfig::load() {
            Ok(c) => c,
            Err(err) => {
                // If we fail to load the configuration, then we should log
                // the error in the diagnostics so it can be shown to the user.
                let mut def = CoreConfig::new_default();
                def.add_diagnostic(format!("error loading user configuration: {err}"));
                def
            }
        };

        let saved_language = std::env::var("LANG").ok();
        if let Some(language) = &config.language {
            set_env("LANG", language);
        }

        // Set gettext global domain to be our app so that our unqualified
        // translations map to our translations.
        if let Err(err) = ghostty_os::i18n::init_global_domain() {
            // Failures shouldn't stop application startup.
            log::warn!("i18n initialization failed error={err:?}");
        }

        // Setup our GTK init env vars
        set_gtk_env(&config);
        if let Err(err) = gtk::init() {
            log::error!("failed to initialize GTK: {err}");
            std::process::exit(1);
        }
        if let Err(err) = adw::init() {
            log::error!("failed to initialize libadwaita: {err}");
        }

        // Log our GTK versions (libadwaita requires GTK to be initialized
        // first in the Rust bindings).
        crate::gtk_version::log_version();
        crate::adw_version::log_version();

        let single_instance = match config.gtk_single_instance {
            GtkSingleInstance::True => true,
            GtkSingleInstance::False => false,
            // This should have been resolved to true/false during config loading.
            GtkSingleInstance::Detect => {
                log::warn!("gtk-single-instance=detect was not resolved, assuming true");
                true
            }
        };

        // Setup the flags for our application.
        let mut flags = gio::ApplicationFlags::default();
        if !single_instance {
            flags |= gio::ApplicationFlags::NON_UNIQUE;
        }

        // Our app ID determines uniqueness and maps to our desktop file.
        let app_id: String = match &config.class {
            Some(class) if gio::Application::id_is_valid(class) => class.clone(),
            Some(_) => {
                log::warn!("invalid 'class' in config, ignoring");
                crate::APPLICATION_ID.to_owned()
            }
            None => crate::APPLICATION_ID.to_owned(),
        };

        let Some(display) = gdk::Display::default() else {
            // I'm unsure of any scenario where this happens.
            log::warn!("gdk display is null, exiting");
            std::process::exit(1);
        };

        // Setup our windowing protocol logic
        let wp = match winproto::App::init(&display, &app_id, &config) {
            Ok(wp) => wp,
            Err(err) => {
                // If we fail to detect or setup the windowing protocol
                // specifics, we fallback to a noop implementation.
                log::warn!("error initializing windowing protocol err={err}");
                winproto::App::None(winproto::noop::App)
            }
        };
        log::debug!("windowing protocol={:?}", wp.protocol());

        log::debug!("creating GTK application id={app_id} single-instance={single_instance}");

        // Wrap our configuration in a GObject.
        let _ = &mut config;
        let config_obj = Config::new(&config);

        // Internally, GTK ensures that only one instance of this provider
        // exists in the provider list for the display.
        let css = CssProviders::new(&display);

        let obj: Self = glib::Object::builder()
            .property("application-id", app_id)
            .property("flags", flags)
            // Force the resource path to a known value so it doesn't depend
            // on the app id and force it to load in compiled resources.
            .property("resource-base-path", crate::RESOURCE_PREFIX)
            .build();

        let imp = obj.imp();
        let _ = imp.core_app.set(core_app);
        let _ = imp.rt_app.set(Rc::new(AppRt));
        *imp.config.borrow_mut() = Some(config_obj);
        *imp.winproto.borrow_mut() = Some(wp);
        *imp.css.borrow_mut() = Some(css);
        let _ = imp.global_shortcuts.set(GlobalShortcuts::new());
        *imp.saved_language.borrow_mut() = saved_language;

        // Signals
        obj.connect_notify_local(Some("config"), |this, _| this.prop_config());

        // Trigger initial config changes
        obj.notify("config");

        obj
    }

    /// Force deinitialize the application (Zig: `deinit`). Applications are
    /// never fully unreferenced so this ensures our state is released.
    pub fn deinit(&self) {
        let imp = self.imp();
        *imp.css.borrow_mut() = None;
        *imp.winproto.borrow_mut() = None;
        *imp.config.borrow_mut() = None;
        if let Some(gs) = imp.global_shortcuts.get() {
            gs.set_connection(None);
        }
        *imp.saved_language.borrow_mut() = None;
    }

    // -----------------------------------------------------------------
    // Accessors

    /// The core app. Not reference counted in Zig; here a cheap `Rc` clone.
    pub fn core(&self) -> Rc<dyn CoreApp> {
        self.imp().core_app.get().expect("core app set").clone()
    }

    /// The `RtApp` implementation to hand to core.
    pub fn rt(&self) -> Rc<dyn RtApp> {
        self.imp().rt_app.get().expect("rt app set").clone()
    }

    /// The configuration object (property `config`). Increments its refcount.
    pub fn get_config(&self) -> Config {
        self.imp()
            .config
            .borrow()
            .clone()
            .expect("application config")
    }

    /// The configuration, as a shared core config.
    pub fn core_config(&self) -> Arc<CoreConfig> {
        self.get_config().get()
    }

    /// The windowing protocol state. Panics if called after `deinit`.
    pub fn winproto(&self) -> Ref<'_, winproto::App> {
        Ref::map(self.imp().winproto.borrow(), |v| {
            v.as_ref().expect("winproto available")
        })
    }

    /// Mutable windowing protocol state (needed by `init_quick_terminal`).
    pub fn winproto_mut(&self) -> RefMut<'_, winproto::App> {
        RefMut::map(self.imp().winproto.borrow_mut(), |v| {
            v.as_mut().expect("winproto available")
        })
    }

    /// The original `LANG` Ghostty was launched with (None if unset).
    pub fn saved_language(&self) -> Option<String> {
        self.imp().saved_language.borrow().clone()
    }

    /// All live surfaces (Zig: `core_app.surfaces.items` mapped to GObjects).
    pub fn surfaces(&self) -> Vec<Surface> {
        self.core()
            .surface_ids()
            .into_iter()
            .filter_map(Surface::from_id)
            .collect()
    }

    /// Wake the main loop (Zig: `wakeup`). Thread safe.
    pub fn wakeup(&self) {
        glib::MainContext::default().wakeup();
    }

    /// Mark that a window was requested at least once.
    pub fn mark_requested_window(&self) {
        self.imp().requested_window.set(true);
    }

    // -----------------------------------------------------------------
    // Run loop

    /// Run the application. Replacement for `gio::Application::run` because
    /// we want tighter control over the event loop to integrate with core.
    pub fn run_loop(&self) -> Result<(), RunError> {
        // Based on the actual `gio.Application.run` implementation.
        let ctx = glib::MainContext::default();
        let _guard = ctx.acquire().map_err(|_| RunError::ContextAcquireFailed)?;

        let result = self.run_loop_inner(&ctx);

        // The final cleanup that is always required at the end of running.
        self.stop_quit_timer();
        gio::Settings::sync();
        // Clear out the event loop, don't block.
        while ctx.iteration(false) {}
        result
    }

    fn run_loop_inner(&self, ctx: &glib::MainContext) -> Result<(), RunError> {
        let gapp = self.upcast_ref::<gio::Application>();

        // Register the application
        if let Err(err) = gapp.register(gio::Cancellable::NONE) {
            log::warn!("error registering application: {}", err.message());
            return Err(RunError::ApplicationRegisterFailed);
        }

        // This just calls the `activate` signal but its part of the normal
        // startup routine, but only if the config allows it.
        if self.core_config().initial_window {
            gapp.activate();
        }

        // If we are NOT the primary instance, then we never want to run.
        if gapp.is_remote() {
            log::debug!("application is remote, exiting run loop after activation");
            return Ok(());
        }

        // Tell systemd that we are ready.
        ghostty_os::systemd::notify::ready();

        // Developer aid: replay synthesized input (GHOSTTY_DEBUG_INPUT_SCRIPT).
        crate::debug_input::start(self);

        log::debug!("entering runloop");
        let imp = self.imp();
        imp.running.set(true);
        let core = self.core();
        let rt = self.rt();
        let mut result = Ok(());
        while imp.running.get() {
            ctx.iteration(true);

            // Tick the core Ghostty terminal app
            if let Err(err) = core.tick(&*rt) {
                log::error!("core tick failed: {err}");
                result = Err(RunError::Tick(err.to_string()));
                break;
            }

            // Check if we must quit based on the current state.
            let must_quit = {
                let config = self.core_config();
                if !config.quit_after_last_window_closed {
                    // If we are configured to always stay running, don't quit.
                    false
                } else if matches!(*imp.quit_timer.borrow(), QuitTimer::Expired) {
                    // If the quit timer has expired, quit.
                    log::debug!("must_quit due to quit timer expired");
                    true
                } else if config.quit_after_last_window_closed_delay.is_none()
                    && imp.requested_window.get()
                    && self.windows().is_empty()
                {
                    // No windows attached to our app. Only if we don't have
                    // the closed delay set.
                    log::debug!("must_quit due to no app windows");
                    true
                } else {
                    false
                }
            };

            if must_quit {
                // All must quit scenarios do not need confirmation.
                self.quit_now();
            }
        }
        log::debug!("exiting runloop");
        result
    }

    /// Quit the application. This starts the process to stop the run loop.
    /// It will not exit the process.
    pub fn quit(&self) {
        let imp = self.imp();

        // If our run loop has already exited then we are done.
        if !imp.running.get() {
            return;
        }

        // If our core app doesn't need to confirm quit then we can exit now.
        if !self.core().needs_confirm_quit() {
            self.quit_now();
            return;
        }

        // Get the parent for our dialog
        let parent = gtk::Window::list_toplevels().into_iter().find(|w| {
            w.downcast_ref::<gtk::Window>()
                .is_some_and(gtk::Window::is_active)
        });

        // Show a confirmation dialog
        let dialog = CloseConfirmationDialog::new(close_confirmation_dialog::Target::App);
        let this = self.downgrade();
        dialog.connect_close_request(move |_| {
            if let Some(this) = this.upgrade() {
                this.quit_now();
            }
        });

        // Show it
        dialog.present(parent.as_ref());
    }

    fn quit_now(&self) {
        // Get all our windows and destroy them, forcing them to free. We
        // only want to destroy our windows (they own every other type of
        // window) so this triggers a proper shutdown sequence.
        for w in gtk::Window::list_toplevels() {
            if let Ok(window) = w.downcast::<Window>() {
                window.destroy();
            }
        }

        // Trigger our runloop exit.
        self.imp().running.set(false);
    }

    // -----------------------------------------------------------------
    // Quit timer

    /// Called when there are no more open surfaces.
    fn start_quit_timer(&self) {
        let config = self.core_config();

        // Cancel any previous timer.
        self.stop_quit_timer();

        // This is a no-op unless we are configured to quit after last window closed.
        if !config.quit_after_last_window_closed {
            return;
        }

        // If a delay is configured, set a timeout function to quit after the delay.
        let imp = self.imp();
        if let Some(v) = &config.quit_after_last_window_closed_delay {
            let this = self.downgrade();
            let id = glib::timeout_add_local(
                std::time::Duration::from_millis(u64::from(v.as_milliseconds())),
                move || {
                    if let Some(this) = this.upgrade() {
                        *this.imp().quit_timer.borrow_mut() = QuitTimer::Expired;
                    }
                    glib::ControlFlow::Break
                },
            );
            *imp.quit_timer.borrow_mut() = QuitTimer::Active(id);
        } else {
            // If no delay is configured, treat it as expired.
            *imp.quit_timer.borrow_mut() = QuitTimer::Expired;
        }
    }

    /// Called when a new surface gets opened.
    fn stop_quit_timer(&self) {
        let old = std::mem::take(&mut *self.imp().quit_timer.borrow_mut());
        if let QuitTimer::Active(id) = old {
            id.remove();
        }
    }

    // -----------------------------------------------------------------
    // Config / CSS / accelerators

    /// Set the configuration for this application. If the config has errors
    /// this may show the config errors dialog.
    fn set_config(&self, config: &Config) {
        *self.imp().config.borrow_mut() = Some(config.clone());
        self.notify("config");

        // Show our errors if we have any
        self.show_config_errors_dialog();
    }

    fn prop_config(&self) {
        // Sync our accelerators for menu items.
        self.sync_action_accelerators();

        // Load our runtime and custom CSS.
        let config = self.core_config();
        if let Some(css) = self.imp().css.borrow_mut().as_mut() {
            css.load_all(&config);
        }

        // Keep global shortcuts in sync with the config (property binding).
        if let Some(gs) = self.imp().global_shortcuts.get() {
            gs.set_config(self.imp().config.borrow().as_ref());
        }
    }

    fn sync_action_accelerators(&self) {
        use ghostty_input::binding::action::{
            CloseTabMode,
            CopyToClipboard,
            InspectorMode,
            SplitDirection,
        };
        let a = |name: &str, action: BindingAction| self.sync_action_accelerator(name, &action);
        a("app.quit", BindingAction::Quit);
        a("app.open-config", BindingAction::OpenConfig);
        a("app.reload-config", BindingAction::ReloadConfig);
        a(
            "win.toggle-inspector",
            BindingAction::Inspector(InspectorMode::Toggle),
        );
        a("app.show-gtk-inspector", BindingAction::ShowGtkInspector);
        a(
            "win.toggle-command-palette",
            BindingAction::ToggleCommandPalette,
        );
        a("win.close", BindingAction::CloseWindow);
        a("win.new-window", BindingAction::NewWindow);
        a("win.new-tab", BindingAction::NewTab);
        a(
            "win.close-tab::this",
            BindingAction::CloseTab(CloseTabMode::This),
        );
        a(
            "tab.close::this",
            BindingAction::CloseTab(CloseTabMode::This),
        );
        a(
            "win.split-right",
            BindingAction::NewSplit(SplitDirection::Right),
        );
        a(
            "win.split-down",
            BindingAction::NewSplit(SplitDirection::Down),
        );
        a(
            "win.split-left",
            BindingAction::NewSplit(SplitDirection::Left),
        );
        a("win.split-up", BindingAction::NewSplit(SplitDirection::Up));
        a(
            "win.copy",
            BindingAction::CopyToClipboard(CopyToClipboard::Mixed),
        );
        a("win.paste", BindingAction::PasteFromClipboard);
        a("win.reset", BindingAction::Reset);
        a("win.clear", BindingAction::ClearScreen);
        a("win.prompt-title", BindingAction::PromptSurfaceTitle);
        a(
            "split-tree.new-split::left",
            BindingAction::NewSplit(SplitDirection::Left),
        );
        a(
            "split-tree.new-split::right",
            BindingAction::NewSplit(SplitDirection::Right),
        );
        a(
            "split-tree.new-split::up",
            BindingAction::NewSplit(SplitDirection::Up),
        );
        a(
            "split-tree.new-split::down",
            BindingAction::NewSplit(SplitDirection::Down),
        );
    }

    fn sync_action_accelerator(&self, gtk_action: &str, action: &BindingAction) {
        // Reset it initially
        self.set_accels_for_action(gtk_action, &[]);

        let config = self.core_config();
        let Some(trigger) = config.keybind.set.get_trigger(action) else {
            return;
        };
        let Some(accel) = crate::key::accel_from_trigger(trigger) else {
            return;
        };
        self.set_accels_for_action(gtk_action, &[accel.as_str()]);
    }

    // -----------------------------------------------------------------
    // Startup

    /// Setup the style manager on startup: initial light/dark mode based on
    /// the configuration, plus change listeners.
    fn startup_style_manager(&self) {
        let config = self.core_config();

        // Setup our initial light/dark
        let style = self.style_manager();
        style.set_color_scheme(match config.window_theme {
            WindowTheme::Auto | WindowTheme::Ghostty => {
                let lum = config.background.to_terminal_rgb().perceived_luminance();
                if lum > 0.5 {
                    adw::ColorScheme::PreferLight
                } else {
                    adw::ColorScheme::PreferDark
                }
            }
            WindowTheme::System => adw::ColorScheme::PreferLight,
            WindowTheme::Dark => adw::ColorScheme::ForceDark,
            WindowTheme::Light => adw::ColorScheme::ForceLight,
        });

        // Setup color change notifications
        let this = self.downgrade();
        style.connect_dark_notify(move |style| {
            if let Some(this) = this.upgrade() {
                this.handle_style_manager_dark(style);
            }
        });

        // Do an initial color scheme sync. This is idempotent and does
        // nothing if our current theme matches what libghostty has.
        self.handle_style_manager_dark(&style);
    }

    fn handle_style_manager_dark(&self, style: &adw::StyleManager) {
        let scheme = if style.is_dark() {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        log::debug!("style manager changed scheme={scheme:?}");

        let core = self.core();
        core.color_scheme_event(&*self.rt(), scheme);
        for id in core.surface_ids() {
            let Some(surface) = Surface::from_id(id) else {
                continue;
            };
            let Some(core_surface) = surface.core() else {
                continue;
            };
            if let Err(err) = core_surface.color_scheme_callback(scheme) {
                log::warn!("unable to tell surface about color scheme change err={err}");
            }
        }

        if let Some(css) = self.imp().css.borrow_mut().as_mut() {
            css.set_color_scheme(scheme == ColorScheme::Dark);
        }
    }

    /// Setup signal handlers (SIGUSR2 reloads the config).
    fn startup_signals(&self) {
        let imp = self.imp();
        if imp.signal_thread_started.replace(true) {
            return;
        }
        // glib-rs doesn't bind `g_unix_signal_add`, so a helper thread waits
        // for SIGUSR2 and hops to the main context.
        let mut signals = match signal_hook::iterator::Signals::new([libc::SIGUSR2]) {
            Ok(s) => s,
            Err(err) => {
                log::warn!("unable to register SIGUSR2 handler: {err}");
                return;
            }
        };
        std::thread::spawn(move || {
            for _ in signals.forever() {
                glib::MainContext::default().invoke(|| {
                    log::info!("received SIGUSR2, reloading configuration");
                    Application::default()
                        .reload_config(Target::App, action::ReloadConfig::default());
                });
            }
        });
    }

    /// Setup our action map.
    fn startup_action_map(&self) {
        let add = |name: &str,
                   param: Option<&glib::VariantTy>,
                   f: Box<dyn Fn(&Self, Option<&glib::Variant>)>| {
            let action = gio::SimpleAction::new(name, param);
            let this = self.downgrade();
            action.connect_activate(move |_, p| {
                if let Some(this) = this.upgrade() {
                    f(&this, p);
                }
            });
            self.add_action(&action);
        };

        let as_ty = glib::VariantTy::new("as").expect("valid variant type");
        add("new-window", None, Box::new(Self::action_new_window));
        add(
            "new-window-command",
            Some(as_ty),
            Box::new(Self::action_new_window),
        );
        add(
            "open-config",
            None,
            Box::new(|this, _| this.action_open_config()),
        );
        add(
            "present-surface",
            Some(glib::VariantTy::UINT64),
            Box::new(Self::action_present_surface),
        );
        add(
            "quit",
            None,
            Box::new(|this, _| {
                this.perform_action(Target::App, Action::Quit);
            }),
        );
        add(
            "reload-config",
            None,
            Box::new(|this, _| {
                this.perform_action(
                    Target::App,
                    Action::ReloadConfig(action::ReloadConfig::default()),
                );
            }),
        );
    }

    /// Setup our global shortcuts.
    fn startup_global_shortcuts(&self) {
        let Some(gs) = self.imp().global_shortcuts.get() else {
            return;
        };

        // On startup, our dbus connection should be available.
        gs.set_connection(self.dbus_connection().as_ref());

        // Keep the shortcut config in sync with the app.
        gs.set_config(self.imp().config.borrow().as_ref());

        // Setup the signal handler for global shortcut triggers
        let this = self.downgrade();
        gs.connect_trigger(move |_, action| {
            let Some(this) = this.upgrade() else { return };
            if let Err(err) = this.core().perform_all_action(&*this.rt(), action) {
                log::warn!("failed to perform action={err}");
            }
        });
    }

    /// Show the config errors dialog if the config on our application has
    /// diagnostics.
    fn show_config_errors_dialog(&self) {
        let imp = self.imp();
        let config = self.get_config();

        // If we already have a dialog, just update the config.
        if let Some(diag) = imp.config_errors_dialog.upgrade() {
            diag.set_property("config", &config);
            if config.has_diagnostics() {
                diag.present(None::<&gtk::Widget>);
            } else {
                diag.close();
            }
            return;
        }

        // No diagnostics, do nothing.
        if !config.has_diagnostics() {
            return;
        }

        // No dialog yet, initialize a new one.
        let dialog = ConfigErrorsDialog::new(&config);
        imp.config_errors_dialog.set(Some(&dialog));

        // Connect to the reload signal so we know to reload our config.
        let this = self.downgrade();
        dialog.connect_reload_config(move |_| {
            let Some(this) = this.upgrade() else { return };
            // We clear our dialog reference because it's going to close
            // after response handling and we don't want to reuse it.
            this.imp()
                .config_errors_dialog
                .set(None::<&ConfigErrorsDialog>);
            // Reload our config as if the app reloaded.
            this.reload_config(Target::App, action::ReloadConfig::default());
        });

        dialog.present(None::<&gtk::Widget>);
    }

    // -----------------------------------------------------------------
    // GActions

    /// `app.new-window` and `app.new-window-command`.
    fn action_new_window(&self, parameter: Option<&glib::Variant>) {
        log::debug!("received new window action");

        let mut working_directory: Option<String> = None;
        let mut title: Option<String> = None;
        let mut command: Option<Command> = None;
        let mut args: Vec<String> = Vec::new();

        'overrides: {
            // were we given a parameter?
            let Some(parameter) = parameter else {
                break 'overrides;
            };

            // ensure that the supplied parameter is an array of strings
            let Some(values) = parameter.get::<Vec<String>>() else {
                log::warn!("parameter is of type '{}', not 'as'", parameter.type_());
                break 'overrides;
            };

            let mut e_seen = false;
            for (i, s) in values.iter().enumerate() {
                log::debug!("new-window argument: {i} {s}");

                if e_seen {
                    args.push(s.clone());
                    continue;
                }
                if s == "-e" {
                    e_seen = true;
                    continue;
                }
                if let Some(v) = s.strip_prefix("--command=") {
                    let mut cmd = Command::default();
                    if let Err(err) = cmd.parse_cli(Some(v)) {
                        log::warn!("unable to parse command: {err:?}");
                        continue;
                    }
                    command = Some(cmd);
                    continue;
                }
                if let Some(v) = s.strip_prefix("--working-directory=") {
                    working_directory = Some(v.trim().to_owned());
                    continue;
                }
                if let Some(v) = s.strip_prefix("--title=") {
                    title = Some(v.trim().to_owned());
                }
            }
        }

        if !args.is_empty() {
            command = Some(Command::Direct(args));
        }

        self.new_window(
            None,
            WindowOverrides {
                command,
                working_directory,
                title,
            },
        );
    }

    fn action_open_config(&self) {
        self.core().push_message(AppMessage::OpenConfig);
    }

    fn action_present_surface(&self, parameter: Option<&glib::Variant>) {
        let Some(parameter) = parameter else { return };

        // Make sure that we've received a u64 from the system.
        let Some(id) = parameter.get::<u64>() else {
            return;
        };

        // A value of zero means that there was no target surface for the
        // notification so we don't focus any surface. Send the id through
        // the core mailbox so core can validate it (a notification may
        // outlive a Ghostty instance).
        if id == 0 {
            return;
        }
        self.core().push_message(AppMessage::PresentSurface(id));
    }

    // -----------------------------------------------------------------
    // Action implementations

    /// apprt API to perform an action. Returns whether it was performed.
    pub fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool {
        // Resolve the surface target (Zig deref'd `*CoreSurface` directly).
        let surface: Option<Surface> = match target {
            Target::App => None,
            Target::Surface(id) => {
                let s = Surface::from_id(id);
                if s.is_none() {
                    log::warn!("action for unknown surface id={id}");
                }
                s
            }
        };
        let is_surface_target = matches!(target, Target::Surface(_));
        // `surface` as the Zig `target` union.
        let tgt = Tgt {
            is_surface: is_surface_target,
            surface,
        };

        match action {
            Action::CloseTab(mode) => tgt.with_surface_bool(|s| {
                let value = match mode {
                    action::CloseTabMode::This => "this",
                    action::CloseTabMode::Other => "other",
                    action::CloseTabMode::Right => "right",
                };
                s.activate_action("tab.close", Some(&value.to_variant()))
                    .is_ok()
            }),
            Action::CloseWindow => {
                tgt.with_surface_bool(|s| s.activate_action("win.close", None).is_ok())
            }
            Action::CopyTitleToClipboard => tgt.with_surface_bool(Surface::copy_title_to_clipboard),
            Action::ConfigChange(change) => {
                // Wrap our config in a GObject (shares the Arc).
                let config_obj = Config::from_arc(change.config);
                match &tgt.surface {
                    Some(s) => s.set_config(&config_obj),
                    None => self.set_config(&config_obj),
                }
                true
            }
            Action::DesktopNotification(n) => {
                self.desktop_notification(&tgt, &n);
                true
            }
            Action::EqualizeSplits => {
                if !tgt.is_surface {
                    log::warn!("equalize splits to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| s.activate_action("split-tree.equalize", None).is_ok())
            }
            Action::GotoSplit(to) => tgt.with_surface_bool(|s| Self::goto_split(s, to)),
            Action::GotoWindow(dir) => Self::goto_window(dir),
            Action::GotoTab(tab) => tgt.with_surface_bool(|s| {
                let Some(window) = ancestor::<Window>(s) else {
                    log::warn!("surface is not in a window, ignoring goto_tab");
                    return false;
                };
                window.select_tab(match tab {
                    action::GotoTab::Previous => SelectTab::Previous,
                    action::GotoTab::Next => SelectTab::Next,
                    action::GotoTab::Last => SelectTab::Last,
                    action::GotoTab::Index(n) if n >= 0 => SelectTab::N(n as usize),
                    action::GotoTab::Index(_) => return false,
                })
            }),
            Action::InitialSize(v) => tgt.with_surface_bool(|s| {
                s.set_default_size(Size {
                    width: v.width,
                    height: v.height,
                });
                true
            }),
            Action::Inspector(v) => tgt.with_surface_bool(|s| s.control_inspector(v)),
            Action::KeySequence(v) => {
                if !tgt.is_surface {
                    log::warn!("key_sequence action to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    s.key_sequence_action(&v);
                    true
                })
            }
            Action::KeyTable(v) => {
                if !tgt.is_surface {
                    log::warn!("key_table action to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    s.key_table_action(&v);
                    true
                })
            }
            Action::MouseOverLink(v) => {
                tgt.with_surface_void("mouse over link", |s| {
                    s.set_mouse_hover_url(if v.url.is_empty() { None } else { Some(&v.url) });
                });
                true
            }
            Action::MouseShape(shape) => {
                tgt.with_surface_void("mouse shape", |s| s.set_mouse_shape(shape));
                true
            }
            Action::MouseVisibility(v) => {
                tgt.with_surface_void("mouse visibility", |s| {
                    s.set_mouse_hidden(matches!(v, action::MouseVisibility::Hidden));
                });
                true
            }
            Action::MoveTab(v) => tgt.with_surface_bool(|s| {
                let Some(window) = ancestor::<Window>(s) else {
                    log::warn!("surface is not in a window, ignoring move_tab");
                    return false;
                };
                window.move_tab(s, v.amount)
            }),
            Action::NewSplit(dir) => {
                if !tgt.is_surface {
                    log::warn!("new split to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    let name = match dir {
                        action::SplitDirection::Right => "right",
                        action::SplitDirection::Down => "down",
                        action::SplitDirection::Left => "left",
                        action::SplitDirection::Up => "up",
                    };
                    s.activate_action("split-tree.new-split", Some(&name.to_variant()))
                        .is_ok()
                })
            }
            Action::NewTab => {
                if !tgt.is_surface {
                    log::warn!("new tab to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    // Get the window ancestor of the surface. Surfaces
                    // shouldn't be aware they might be in windows but at the
                    // app level we can do this.
                    let Some(window) = ancestor::<Window>(s) else {
                        log::warn!("surface is not in a window, ignoring new_tab");
                        return false;
                    };
                    window.new_tab(Some(s));
                    true
                })
            }
            Action::NewWindow => {
                self.new_window(tgt.surface.as_ref(), WindowOverrides::default());
                true
            }
            Action::OpenConfig => self.open_config(),
            Action::OpenUrl(v) => {
                self.open_url(&v);
                true
            }
            Action::Pwd(v) => {
                tgt.with_surface_void("pwd", |s| s.set_pwd(Some(&v.pwd)));
                true
            }
            Action::PresentTerminal => tgt.with_surface_bool(|s| {
                s.present();
                true
            }),
            Action::ProgressReport(v) => tgt.with_surface_bool(|s| {
                s.set_progress_report(v);
                true
            }),
            Action::PromptTitle(v) => match v {
                action::PromptTitle::Surface => tgt.with_surface_bool(|s| {
                    s.prompt_title();
                    true
                }),
                action::PromptTitle::Tab => tgt.with_surface_bool(|s| {
                    let Some(tab) = ancestor::<Tab>(s) else {
                        log::warn!("surface is not in a tab, ignoring prompt_tab_title");
                        return false;
                    };
                    tab.prompt_tab_title();
                    true
                }),
            },
            Action::Quit => {
                self.quit();
                true
            }
            Action::QuitTimer(mode) => {
                match mode {
                    action::QuitTimer::Start => self.start_quit_timer(),
                    action::QuitTimer::Stop => self.stop_quit_timer(),
                }
                true
            }
            Action::ReloadConfig(opts) => {
                self.reload_config(target, opts);
                true
            }
            Action::Render => {
                if let Some(s) = &tgt.surface {
                    s.redraw();
                }
                true
            }
            Action::ResizeSplit(v) => {
                if !tgt.is_surface {
                    log::warn!("resize_split to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    let Some(tree) = ancestor::<SplitTree>(s) else {
                        log::warn!("surface is not in a split tree, ignoring resize_split");
                        return false;
                    };
                    // If the tree has no splits (only one leaf), this action
                    // is not performable. This allows the key event to pass
                    // through to the terminal.
                    if !tree.get_is_split() {
                        return false;
                    }
                    let dir = match v.direction {
                        action::ResizeSplitDirection::Up => {
                            ghostty_datastruct::split_tree::Direction::Up
                        }
                        action::ResizeSplitDirection::Down => {
                            ghostty_datastruct::split_tree::Direction::Down
                        }
                        action::ResizeSplitDirection::Left => {
                            ghostty_datastruct::split_tree::Direction::Left
                        }
                        action::ResizeSplitDirection::Right => {
                            ghostty_datastruct::split_tree::Direction::Right
                        }
                    };
                    match tree.resize(dir, v.amount) {
                        Ok(b) => b,
                        Err(err) => {
                            log::warn!("unable to resize split: {err:?}");
                            false
                        }
                    }
                })
            }
            Action::RingBell => {
                if let Some(s) = &tgt.surface {
                    s.set_bell_ringing(true);
                }
                true
            }
            Action::Scrollbar(v) => {
                if let Some(s) = &tgt.surface {
                    s.set_scrollbar(v);
                }
                true
            }
            Action::SetTitle(v) => {
                tgt.with_surface_void("set_title", |s| s.set_title(Some(&v.title)));
                true
            }
            Action::SetTabTitle(v) => {
                if !tgt.is_surface {
                    log::warn!("set_tab_title to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    let Some(tab) = ancestor::<Tab>(s) else {
                        log::warn!("surface is not in a tab, ignoring set_tab_title");
                        return false;
                    };
                    tab.set_title_override(if v.title.is_empty() {
                        None
                    } else {
                        Some(&v.title)
                    });
                    true
                })
            }
            Action::ShowChildExited(v) => tgt.with_surface_bool(|s| s.child_exited(&v)),
            Action::ShowGtkInspector => {
                gtk::Window::set_interactive_debugging(true);
                true
            }
            Action::SizeLimit(v) => tgt.with_surface_bool(|s| {
                // Note: we ignore the max size currently because we have
                // no mechanism to enforce it.
                s.set_min_size(Size {
                    width: v.min_width,
                    height: v.min_height,
                });
                true
            }),
            Action::ToggleMaximize => {
                if let Some(s) = &tgt.surface {
                    s.toggle_maximize();
                }
                true
            }
            Action::ToggleFullscreen(_) => {
                if let Some(s) = &tgt.surface {
                    s.toggle_fullscreen();
                }
                true
            }
            Action::ToggleQuickTerminal => self.toggle_quick_terminal(),
            Action::ToggleTabOverview => tgt.with_surface_bool(|s| {
                let Some(window) = ancestor::<Window>(s) else {
                    log::warn!("surface is not in a window, ignoring toggle_tab_overview");
                    return false;
                };
                window.toggle_tab_overview();
                true
            }),
            Action::ToggleWindowDecorations => tgt.with_surface_bool(|s| {
                let Some(window) = ancestor::<Window>(s) else {
                    log::warn!("surface is not in a window, ignoring toggle_window_decorations");
                    return false;
                };
                window.toggle_window_decorations();
                true
            }),
            Action::ToggleCommandPalette => tgt.with_surface_bool(Surface::toggle_command_palette),
            Action::ToggleSplitZoom => {
                if !tgt.is_surface {
                    log::warn!("toggle_split_zoom to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(|s| {
                    let Some(tree) = ancestor::<SplitTree>(s) else {
                        log::warn!("surface is not in a split tree, ignoring toggle_split_zoom");
                        return false;
                    };
                    // If the tree has no splits (only one leaf), this action
                    // is not performable.
                    if !tree.get_is_split() {
                        return false;
                    }
                    s.activate_action("split-tree.zoom", None).is_ok()
                })
            }
            Action::ShowOnScreenKeyboard => {
                if !tgt.is_surface {
                    log::warn!("show_on_screen_keyboard to app is unexpected");
                    return false;
                }
                tgt.with_surface_bool(Surface::show_on_screen_keyboard)
            }
            Action::CommandFinished(v) => tgt.with_surface_bool(|s| s.command_finished(&v)),
            Action::Readonly(v) => tgt.with_surface_bool(|s| s.set_readonly(v)),
            Action::StartSearch(v) => {
                if let Some(s) = &tgt.surface {
                    s.set_search_active(true, &v.needle);
                }
                true
            }
            Action::EndSearch => {
                if let Some(s) = &tgt.surface {
                    s.set_search_active(false, "");
                }
                true
            }
            Action::SearchTotal(v) => {
                if let Some(s) = &tgt.surface {
                    s.set_search_total(v.total);
                }
                true
            }
            Action::SearchSelected(v) => {
                if let Some(s) = &tgt.surface {
                    s.set_search_selected(v.selected);
                }
                true
            }

            // Unimplemented
            Action::SecureInput(_)
            | Action::CloseAllWindows
            | Action::FloatWindow(_)
            | Action::ToggleVisibility
            | Action::ToggleBackgroundOpacity
            | Action::CellSize(_)
            | Action::RenderInspector
            | Action::RendererHealth(_)
            | Action::ColorChange(_)
            | Action::ResetWindowSize
            | Action::CheckForUpdates
            | Action::Undo
            | Action::Redo => {
                log::warn!("unimplemented action={:?}", action.key());
                false
            }
        }
    }

    fn desktop_notification(&self, tgt: &Tgt, n: &action::DesktopNotification) {
        if let Some(s) = &tgt.surface {
            s.send_desktop_notification(&n.title, &n.body);
            return;
        }

        // Set a default title if we don't already have one
        let title = if n.title.is_empty() {
            "Ghostty"
        } else {
            n.title.as_str()
        };

        let notification = gio::Notification::new(title);
        notification.set_body(Some(&n.body));
        notification.set_icon(&gio::ThemedIcon::new("com.mitchellh.ghostty"));
        notification
            .set_default_action_and_target_value("app.present-surface", Some(&0u64.to_variant()));

        // We set the notification ID to the body content. If the content is
        // the same, this notification may replace a previous notification.
        self.send_notification(Some(&n.body), &notification);
    }

    fn goto_split(surface: &Surface, to: action::GotoSplit) -> bool {
        use ghostty_datastruct::split_tree::{Direction, Goto};
        // Design note: we can't use widget actions here because we need to
        // know whether there is a goto target for returning the proper
        // perform result (boolean).
        let Some(tree) = ancestor::<SplitTree>(surface) else {
            log::warn!("surface is not in a split tree, ignoring goto_split");
            return false;
        };
        tree.goto(match to {
            action::GotoSplit::Previous => Goto::PreviousWrapped,
            action::GotoSplit::Next => Goto::NextWrapped,
            action::GotoSplit::Up => Goto::Spatial(Direction::Up),
            action::GotoSplit::Down => Goto::Spatial(Direction::Down),
            action::GotoSplit::Left => Goto::Spatial(Direction::Left),
            action::GotoSplit::Right => Goto::Spatial(Direction::Right),
        })
    }

    fn goto_window(direction: action::GotoWindow) -> bool {
        let list: Vec<gtk::Window> = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .collect();
        if list.is_empty() {
            return false;
        }

        // The window we're starting from is typically our active window.
        let start = list.iter().position(gtk::Window::is_active).unwrap_or(0);

        // Go forward or backwards in the list until we find a valid window
        // that is visible, wrapping around once.
        let n = list.len();
        for step in 0..n {
            let idx = match direction {
                action::GotoWindow::Next => (start + step) % n,
                action::GotoWindow::Previous => (start + n - step) % n,
            };
            if Self::goto_window_maybe(&list[idx]) {
                return true;
            }
        }
        false
    }

    fn goto_window_maybe(gtk_window: &gtk::Window) -> bool {
        // If it is already active skip it.
        if gtk_window.is_active() {
            return false;
        }
        // If it is hidden, skip it.
        if !gtk_window.is_visible() {
            return false;
        }
        // If it isn't a Ghostty window, skip it.
        let Some(window) = gtk_window.downcast_ref::<Window>() else {
            return false;
        };

        // Focus our active surface
        let Some(surface) = window.get_active_surface() else {
            return false;
        };
        gtk_window.present();
        surface.grab_focus();
        true
    }

    /// Create a new window (Zig: `Action.newWindow`).
    pub fn new_window(&self, parent: Option<&Surface>, overrides: WindowOverrides) {
        // Note that we've requested a window at least once. This is used to
        // trigger quit on no windows.
        self.mark_requested_window();

        let win = Window::new(self, overrides.title.as_deref());
        self.init_and_show_window(&win, parent, &overrides);
    }

    fn init_and_show_window(
        &self,
        win: &Window,
        parent: Option<&Surface>,
        overrides: &WindowOverrides,
    ) {
        // Setup a binding so that whenever our config changes so does the
        // window. There's never a time when the window config should be out
        // of sync with the application config.
        self.bind_property("config", win, "config").build();

        // Create a new tab with window context (first tab in new window)
        win.new_tab_for_window(
            parent,
            TabOverrides {
                command: overrides.command.clone(),
                working_directory: overrides.working_directory.clone(),
                title: overrides.title.clone(),
            },
        );

        // Estimate the initial window size before presenting so the window
        // manager can position it correctly.
        if let Some(surface) = win.get_active_surface() {
            surface.estimate_initial_size();
            if let Some(size) = surface.get_default_size() {
                win.set_default_size(size.width as i32, size.height as i32);
            }
        }

        // Show the window
        win.present();
    }

    fn open_config(&self) -> bool {
        // Get the config file path
        let path = match ghostty_config::edit::open_path() {
            Ok(p) => p,
            Err(err) => {
                log::warn!("error getting config file path: {err}");
                return false;
            }
        };

        // Open it using open_url. "path" isn't actually a URL but at the
        // time of writing that works just fine for GTK.
        self.open_url(&action::OpenUrl {
            kind: action::OpenUrlKind::Text,
            url: path,
        });
        true
    }

    fn open_url(&self, value: &action::OpenUrl) {
        // TODO: use https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html

        // Fallback to the minimal cross-platform way of opening a URL.
        let kind = match value.kind {
            action::OpenUrlKind::Unknown => ghostty_os::open::OpenUrlKind::Unknown,
            action::OpenUrlKind::Text => ghostty_os::open::OpenUrlKind::Text,
            action::OpenUrlKind::Html => ghostty_os::open::OpenUrlKind::Html,
        };
        if let Err(err) = ghostty_os::open::open(kind, &value.url) {
            log::warn!("unable to open url: {err}");
        }
    }

    /// Reload the configuration for the application and propagate it across
    /// the entire application and all terminals.
    fn reload_config(&self, target: Target<SurfaceId>, opts: action::ReloadConfig) {
        // Tell systemd that reloading has started; when we exit this
        // function tell it that reloading has finished.
        ghostty_os::systemd::notify::reloading();
        let _ready = scopeguard_ready();

        // Get our config object.
        let config: Arc<CoreConfig> = if opts.soft {
            // Soft-reloading applies conditional logic to the existing
            // loaded config so we return that as-is.
            self.core_config()
        } else {
            // Hard reload, load a new config completely.
            match CoreConfig::load() {
                Ok(c) => Arc::new(c),
                Err(err) => {
                    log::warn!("error reloading config: {err}");
                    return;
                }
            }
        };

        // Update the proper target. This will trigger a `config_change`
        // apprt action which will propagate the config properly to our
        // property system.
        match target {
            Target::App => {
                if let Err(err) = self.core().update_config(&*self.rt(), &config) {
                    log::warn!("error reloading config: {err}");
                }
            }
            Target::Surface(id) => {
                let Some(core) = Surface::from_id(id).and_then(|s| s.core()) else {
                    return;
                };
                if let Err(err) = core.update_config(&config) {
                    log::warn!("error reloading config: {err}");
                }
            }
        }
    }

    fn toggle_quick_terminal(&self) -> bool {
        // If we already have a quick terminal window, we just toggle the
        // visibility of it.
        if let Some(win) = Self::quick_terminal_window() {
            win.toggle_visibility();
            return true;
        }

        // If we don't support quick terminals then we do nothing.
        if !self.winproto().supports_quick_terminal() {
            return false;
        }

        // Create our new window as a quick terminal
        crate::register_resources();
        let win: Window = glib::Object::builder()
            .property("application", self)
            .property("quick-terminal", true)
            .build();
        debug_assert!(win.is_quick_terminal());
        self.init_and_show_window(&win, None, &WindowOverrides::default());
        true
    }

    fn quick_terminal_window() -> Option<Window> {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<Window>().ok())
            .find(Window::is_quick_terminal)
    }
}

/// The Zig `apprt.Target` as seen by action handlers.
struct Tgt {
    is_surface: bool,
    surface: Option<Surface>,
}

impl Tgt {
    /// Run `f` for surface targets; app targets (or unknown surfaces) are
    /// `false` (Zig: `.app => return false`).
    fn with_surface_bool(&self, f: impl FnOnce(&Surface) -> bool) -> bool {
        match &self.surface {
            Some(s) => f(s),
            None => false,
        }
    }

    /// Like Zig's void handlers that log a warning for app targets.
    fn with_surface_void(&self, what: &str, f: impl FnOnce(&Surface)) {
        match &self.surface {
            Some(s) => f(s),
            None if !self.is_surface => log::warn!("{what} to app is unexpected"),
            None => {}
        }
    }
}

/// Find an ancestor widget of type `T` (Zig: `ext.getAncestor`).
fn ancestor<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Option<T> {
    widget.ancestor(T::static_type()).and_downcast::<T>()
}

/// Guard that tells systemd we're ready again on drop.
struct ReadyGuard;
impl Drop for ReadyGuard {
    fn drop(&mut self) {
        ghostty_os::systemd::notify::ready();
    }
}
fn scopeguard_ready() -> ReadyGuard {
    ReadyGuard
}
