//! Port of apprt/gtk/winproto/wayland.zig: Wayland protocol implementation.
//!
//! The GDK display's `wl_display` is shared through `gdk4-wayland`'s
//! `wayland_crate` feature (a `wayland_client::Connection` over GDK's foreign
//! display), so no `unsafe` is required here. We use a private event queue on
//! that connection for the protocol objects we own (KDE blur / server
//! decoration / slide, xdg-activation). GTK's own queue dispatching reads the
//! socket; events destined for our queue are picked up by
//! [`Shared::dispatch_pending`] (blocking roundtrips at init, a short poll
//! while an activation token request is in flight).
//!
//! # gtk4-layer-shell
//! The quick terminal uses the `gtk4-layer-shell` library. Upstream documents
//! that libgtk4-layer-shell must be linked/loaded *before* libwayland-client
//! (otherwise it cannot interpose the protocol). With Rust the link order is
//! not under our control: the final binary should link `gtk4-layer-shell`
//! first (or run with `LD_PRELOAD=libgtk4-layer-shell.so`). Without it,
//! `layer_shell::is_supported()` is false and the quick terminal is disabled.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use gdk4_wayland::WaylandDisplay;
use gdk4_wayland::prelude::*;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{self as layer_shell, LayerShell};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur;
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager;
use wayland_protocols_plasma::server_decoration::client::org_kde_kwin_server_decoration::OrgKdeKwinServerDecoration;
use wayland_protocols_plasma::server_decoration::client::org_kde_kwin_server_decoration_manager::{
    self, OrgKdeKwinServerDecorationManager,
};
use wayland_protocols_plasma::slide::client::org_kde_kwin_slide::OrgKdeKwinSlide;
use wayland_protocols_plasma::slide::client::org_kde_kwin_slide_manager::OrgKdeKwinSlideManager;

use super::{Error, Result, WindowHost};
use ghostty_config::config::Config;
use ghostty_config::types_b::{
    QuickTerminalDimensions, QuickTerminalKeyboardInteractivity, QuickTerminalLayer,
    QuickTerminalPosition, WindowDecoration,
};
use ghostty_input::Mods;

/// `org_kde_kwin_server_decoration_manager.mode` values.
const DECO_NONE: u32 = 0;
const DECO_CLIENT: u32 = 1;
const DECO_SERVER: u32 = 2;

/// `org_kde_kwin_slide.location` values.
const SLIDE_TOP: u32 = 1;
const SLIDE_LEFT: u32 = 2;
const SLIDE_RIGHT: u32 = 3;
const SLIDE_BOTTOM: u32 = 4;

/// Application-wide Wayland globals (Zig: `App.Context`).
#[derive(Default)]
struct State {
    kde_blur_manager: Option<OrgKdeKwinBlurManager>,
    // FIXME (upstream): replace with `zxdg_decoration_v1` once GTK merges
    // https://gitlab.gnome.org/GNOME/gtk/-/merge_requests/6398
    kde_decoration_manager: Option<OrgKdeKwinServerDecorationManager>,
    kde_slide_manager: Option<OrgKdeKwinSlideManager>,
    default_deco_mode: Option<u32>,
    xdg_activation: Option<XdgActivationV1>,
    /// Whether the xdg_wm_dialog_v1 protocol is present. If so,
    /// gtk4-layer-shell < 1.0.4 may crash creating a quick terminal.
    /// See https://github.com/wmww/gtk4-layer-shell/issues/50
    xdg_wm_dialog_present: bool,
    /// Activation tokens requested but not yet `done`.
    pending_tokens: usize,
    /// The registry name of each global we bound (interface -> name), used
    /// to unset the global when the compositor removes it.
    bound_names: std::collections::HashMap<&'static str, u32>,
}

impl State {
    /// Bind a newly advertised global if it is one we support
    /// (Zig `registryListener` `.global`).
    fn registry_global(
        &mut self,
        registry: &WlRegistry,
        qh: &QueueHandle<State>,
        name: u32,
        interface: &str,
    ) {
        log::debug!("found global {interface}");

        // We don't actually do anything with this other than checking
        // for its existence, so we process this separately.
        if interface == "xdg_wm_dialog_v1" {
            self.xdg_wm_dialog_present = true;
            return;
        }

        macro_rules! try_bind {
            ($field:ident, $t:ty) => {
                if interface == <$t as Proxy>::interface().name {
                    log::debug!("matched {}", interface);
                    self.$field = Some(registry.bind::<$t, _, _>(name, 1, qh, ()));
                    self.bound_names
                        .insert(<$t as Proxy>::interface().name, name);
                }
            };
        }
        try_bind!(kde_blur_manager, OrgKdeKwinBlurManager);
        try_bind!(kde_decoration_manager, OrgKdeKwinServerDecorationManager);
        try_bind!(kde_slide_manager, OrgKdeKwinSlideManager);
        try_bind!(xdg_activation, XdgActivationV1);
    }

    /// A global is suddenly no longer available: destroy and unset it as the
    /// protocol mandates (Zig `registryListener` `.global_remove`).
    fn registry_global_remove(&mut self, name: u32) {
        macro_rules! try_remove {
            ($field:ident, $t:ty) => {
                let iface = <$t as Proxy>::interface().name;
                if self.bound_names.get(iface) == Some(&name) {
                    self.bound_names.remove(iface);
                    // Dropping the proxy releases it client-side.
                    self.$field = None;
                }
            };
        }
        try_remove!(kde_blur_manager, OrgKdeKwinBlurManager);
        try_remove!(kde_decoration_manager, OrgKdeKwinServerDecorationManager);
        try_remove!(kde_slide_manager, OrgKdeKwinSlideManager);
        if self.bound_names.get("xdg_activation_v1") == Some(&name) {
            self.bound_names.remove("xdg_activation_v1");
            if let Some(a) = self.xdg_activation.take() {
                a.destroy();
            }
        }
    }
}

struct Shared {
    conn: Connection,
    qh: QueueHandle<State>,
    queue: RefCell<EventQueue<State>>,
    state: RefCell<State>,
}

impl Shared {
    fn dispatch_pending(&self) {
        let mut queue = self.queue.borrow_mut();
        let mut state = self.state.borrow_mut();
        if let Err(e) = queue.dispatch_pending(&mut *state) {
            log::warn!("wayland dispatch error: {e}");
        }
    }

    fn flush(&self) {
        if let Err(e) = self.conn.flush() {
            log::warn!("wayland flush error: {e}");
        }
    }
}

/// Interface names of the globals we bind.
const BOUND_INTERFACES: [&str; 4] = [
    "org_kde_kwin_blur_manager",
    "org_kde_kwin_server_decoration_manager",
    "org_kde_kwin_slide_manager",
    "xdg_activation_v1",
];

fn bind<I: Proxy + 'static>(
    globals: &wayland_client::globals::GlobalList,
    qh: &QueueHandle<State>,
) -> Option<I>
where
    State: Dispatch<I, ()>,
{
    match globals.bind::<I, _, _>(qh, 1..=1, ()) {
        Ok(v) => Some(v),
        Err(wayland_client::globals::BindError::NotPresent) => None,
        Err(e) => {
            log::warn!("error binding interface {} error={e}", I::interface().name);
            None
        }
    }
}

/// Wayland application-wide state.
pub struct App {
    shared: Rc<Shared>,
}

impl App {
    /// Returns `Ok(None)` if the display isn't Wayland.
    pub fn init(
        gdk_display: &gdk::Display,
        _app_id: &str,
        _config: &Config,
    ) -> Result<Option<App>> {
        let Some(wl_display) = gdk_display.downcast_ref::<WaylandDisplay>() else {
            return Ok(None);
        };
        let display = wl_display.wl_display().ok_or(Error::NoWaylandDisplay)?;
        let backend = display.backend().upgrade().ok_or(Error::NoWaylandDisplay)?;
        let conn = Connection::from_backend(backend);

        // Get our display registry so we can get all the available interfaces
        // and bind to what we need.
        let (globals, mut queue) = registry_queue_init::<State>(&conn)
            .map_err(|e| Error::Wayland(format!("registry: {e}")))?;
        let qh = queue.handle();

        let mut state = State::default();
        globals.contents().with_list(|list| {
            for g in list {
                log::debug!("found global {}", g.interface);
                if g.interface == "xdg_wm_dialog_v1" {
                    state.xdg_wm_dialog_present = true;
                }
            }
        });

        state.kde_blur_manager = bind(&globals, &qh);
        state.kde_decoration_manager = bind(&globals, &qh);
        state.kde_slide_manager = bind(&globals, &qh);
        state.xdg_activation = bind(&globals, &qh);
        globals.contents().with_list(|list| {
            for g in list {
                let bound = match g.interface.as_str() {
                    "org_kde_kwin_blur_manager" => state.kde_blur_manager.is_some(),
                    "org_kde_kwin_server_decoration_manager" => {
                        state.kde_decoration_manager.is_some()
                    }
                    "org_kde_kwin_slide_manager" => state.kde_slide_manager.is_some(),
                    "xdg_activation_v1" => state.xdg_activation.is_some(),
                    _ => false,
                };
                if bound && let Some(name) = BOUND_INTERFACES.iter().find(|n| **n == g.interface) {
                    state.bound_names.insert(name, g.name);
                }
            }
        });

        // Do another round-trip to get the default decoration mode
        queue
            .roundtrip(&mut state)
            .map_err(|e| Error::Wayland(format!("roundtrip: {e}")))?;

        Ok(Some(App {
            shared: Rc::new(Shared {
                conn,
                qh,
                queue: RefCell::new(queue),
                state: RefCell::new(state),
            }),
        }))
    }

    #[allow(clippy::unused_self)]
    pub fn event_mods(&self, _: Option<&gdk::Device>, _: gdk::ModifierType) -> Option<Mods> {
        None
    }

    pub fn supports_quick_terminal(&self) -> bool {
        if !layer_shell::is_supported() {
            log::warn!(
                "your compositor does not support the wlr-layer-shell protocol; disabling quick terminal"
            );
            return false;
        }

        let version = (
            layer_shell::major_version(),
            layer_shell::minor_version(),
            layer_shell::micro_version(),
        );
        if self.shared.state.borrow().xdg_wm_dialog_present && version < (1, 0, 4) {
            log::warn!(
                "the version of gtk4-layer-shell installed on your system is too old (must be 1.0.4 or newer); disabling quick terminal"
            );
            return false;
        }

        true
    }

    /// Initialize gtk4-layer-shell for the window (must happen before the
    /// window is realized).
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn init_quick_terminal(&mut self, window: &gtk::Window) -> Result<()> {
        window.init_layer_shell();
        Ok(())
    }
}

/// Data attached to an xdg activation token.
struct ActivationData {
    surface: WlSurface,
    /// The live token; cleared when `done` fires.
    token: Mutex<Option<XdgActivationTokenV1>>,
    done: AtomicBool,
}

/// Per-window (wl_surface) state for the Wayland protocol.
pub struct Window {
    host: Rc<dyn WindowHost>,
    shared: Rc<Shared>,
    surface: WlSurface,
    /// Present when the window is blurred.
    blur_token: Option<OrgKdeKwinBlur>,
    /// Controls the decoration mode (client/server/auto) of the window.
    decoration: Option<OrgKdeKwinServerDecoration>,
    /// Slide-in/out animation of the quick terminal (otherwise None).
    slide: Option<OrgKdeKwinSlide>,
    /// Present while the window is requesting attention.
    activation: Option<Arc<ActivationData>>,
    monitor_handler: Option<(gdk::Surface, glib::SignalHandlerId)>,
}

impl Window {
    pub fn init(app: &App, host: Rc<dyn WindowHost>) -> Result<Window> {
        let win = host.gtk_window().ok_or(Error::NotWaylandSurface)?;
        let gdk_surface = super::native_surface(&win).ok_or(Error::NotWaylandSurface)?;
        let wl_surface = gdk_surface
            .downcast_ref::<gdk4_wayland::WaylandSurface>()
            .ok_or(Error::NotWaylandSurface)?
            .wl_surface()
            .ok_or(Error::NotWaylandSurface)?;

        let shared = Rc::clone(&app.shared);

        // Get our decoration object so we can control the CSD vs SSD status.
        let decoration = shared
            .state
            .borrow()
            .kde_decoration_manager
            .as_ref()
            .map(|mgr| mgr.create(&wl_surface, &shared.qh, ()));

        let monitor_handler = if host.is_quick_terminal() {
            let host = Rc::clone(&host);
            let id = gdk_surface.connect_enter_monitor(move |_, monitor| {
                entered_monitor(&*host, monitor);
            });
            Some((gdk_surface.clone(), id))
        } else {
            None
        };

        Ok(Window {
            host,
            shared,
            surface: wl_surface,
            blur_token: None,
            decoration,
            slide: None,
            activation: None,
            monitor_handler,
        })
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn resize_event(&mut self) -> Result<()> {
        Ok(())
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn sync_appearance(&mut self) -> Result<()> {
        self.sync_blur();
        self.sync_decoration();
        if self.host.is_quick_terminal()
            && let Err(e) = self.sync_quick_terminal()
        {
            log::warn!("failed to sync quick terminal appearance={e}");
        }
        self.shared.flush();
        Ok(())
    }

    pub fn client_side_decoration_enabled(&self) -> bool {
        match self.decoration_mode() {
            DECO_CLIENT => true,
            // If we support SSDs, then we should *not* enable CSDs if we prefer
            // SSDs. However, if we do not support SSDs (e.g. GNOME) then we
            // should enable CSDs even if the user prefers SSDs.
            DECO_SERVER => self.shared.state.borrow().kde_decoration_manager.is_none(),
            _ => false,
        }
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn add_subprocess_env(&mut self, _: &mut Vec<(String, String)>) -> Result<()> {
        Ok(())
    }

    pub fn set_urgent(&mut self, urgent: bool) -> Result<()> {
        let Some(activation) = self.shared.state.borrow().xdg_activation.clone() else {
            return Ok(());
        };

        // If there already is a token, destroy and unset it
        if let Some(old) = self.activation.take()
            && let Some(tok) = old.token.lock().ok().and_then(|mut t| t.take())
        {
            tok.destroy();
            if !old.done.load(Ordering::Relaxed) {
                let mut st = self.shared.state.borrow_mut();
                st.pending_tokens = st.pending_tokens.saturating_sub(1);
            }
        }

        if urgent {
            let data = Arc::new(ActivationData {
                surface: self.surface.clone(),
                token: Mutex::new(None),
                done: AtomicBool::new(false),
            });
            let token = activation.get_activation_token(&self.shared.qh, Arc::clone(&data));
            token.set_surface(&self.surface);
            token.commit();
            if let Ok(mut t) = data.token.lock() {
                *t = Some(token);
            }
            self.shared.state.borrow_mut().pending_tokens += 1;
            self.shared.flush();
            self.activation = Some(data);

            // Poll our queue until the compositor answers.
            let shared = Rc::clone(&self.shared);
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                shared.dispatch_pending();
                if shared.state.borrow().pending_tokens == 0 {
                    shared.flush();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
        Ok(())
    }

    /// Update the blur state of the window.
    fn sync_blur(&mut self) {
        let Some(manager) = self.shared.state.borrow().kde_blur_manager.clone() else {
            return;
        };
        let Some(config) = self.host.config() else {
            return;
        };
        let blur = config.background_blur;

        if let Some(tok) = &self.blur_token {
            // Only release token when transitioning from blurred -> not blurred
            if !blur.enabled() {
                manager.unset(&self.surface);
                tok.release();
                self.blur_token = None;
            }
        } else if blur.enabled() {
            // Only acquire token when transitioning from not blurred -> blurred
            let tok = manager.create(&self.surface, &self.shared.qh, ());
            tok.commit();
            self.blur_token = Some(tok);
        }
    }

    fn sync_decoration(&self) {
        let Some(deco) = &self.decoration else {
            return;
        };
        deco.request_mode(self.decoration_mode());
    }

    fn decoration_mode(&self) -> u32 {
        match self.host.window_decoration() {
            WindowDecoration::Auto => self
                .shared
                .state
                .borrow()
                .default_deco_mode
                .unwrap_or(DECO_CLIENT),
            WindowDecoration::Client => DECO_CLIENT,
            WindowDecoration::Server => DECO_SERVER,
            WindowDecoration::None => DECO_NONE,
        }
    }

    fn sync_quick_terminal(&mut self) -> Result<()> {
        let window = self.host.gtk_window().ok_or(Error::NotWaylandSurface)?;
        let Some(config) = self.host.config() else {
            return Ok(());
        };

        window.set_layer(match config.gtk_quick_terminal_layer {
            QuickTerminalLayer::Overlay => layer_shell::Layer::Overlay,
            QuickTerminalLayer::Top => layer_shell::Layer::Top,
            QuickTerminalLayer::Bottom => layer_shell::Layer::Bottom,
            QuickTerminalLayer::Background => layer_shell::Layer::Background,
        });
        window.set_namespace(Some(config.gtk_quick_terminal_namespace.as_str()));

        window.set_keyboard_mode(match config.quick_terminal_keyboard_interactivity {
                QuickTerminalKeyboardInteractivity::None => layer_shell::KeyboardMode::None,
                QuickTerminalKeyboardInteractivity::OnDemand => {
                    if layer_shell::protocol_version() < 4 {
                        log::warn!(
                            "your compositor does not support on-demand keyboard access; falling back to exclusive access"
                        );
                        layer_shell::KeyboardMode::Exclusive
                    } else {
                        layer_shell::KeyboardMode::OnDemand
                    }
                }
                QuickTerminalKeyboardInteractivity::Exclusive => {
                    layer_shell::KeyboardMode::Exclusive
                }
            },
        );

        let anchored_edge = match config.quick_terminal_position {
            QuickTerminalPosition::Left => Some(layer_shell::Edge::Left),
            QuickTerminalPosition::Right => Some(layer_shell::Edge::Right),
            QuickTerminalPosition::Top => Some(layer_shell::Edge::Top),
            QuickTerminalPosition::Bottom => Some(layer_shell::Edge::Bottom),
            QuickTerminalPosition::Center => None,
        };

        for edge in [
            layer_shell::Edge::Left,
            layer_shell::Edge::Right,
            layer_shell::Edge::Top,
            layer_shell::Edge::Bottom,
        ] {
            if anchored_edge == Some(edge) {
                window.set_margin(edge, 0);
                window.set_anchor(edge, true);
                continue;
            }
            // Arbitrary margin - could be made customizable?
            window.set_margin(edge, 20);
            window.set_anchor(edge, false);
        }

        if let Some(slide) = self.slide.take() {
            slide.release();
        }

        self.slide = anchored_edge.and_then(|anchored| {
            let mgr = self.shared.state.borrow().kde_slide_manager.clone()?;
            let slide = mgr.create(&self.surface, &self.shared.qh, ());
            slide.set_location(match anchored {
                layer_shell::Edge::Top => SLIDE_TOP,
                layer_shell::Edge::Bottom => SLIDE_BOTTOM,
                layer_shell::Edge::Left => SLIDE_LEFT,
                _ => SLIDE_RIGHT,
            });
            slide.commit();
            Some(slide)
        });
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some((surface, id)) = self.monitor_handler.take() {
            surface.disconnect(id);
        }
        if let Some(blur) = self.blur_token.take() {
            blur.release();
        }
        if let Some(deco) = self.decoration.take() {
            deco.release();
        }
        if let Some(slide) = self.slide.take() {
            slide.release();
        }
        if let Some(act) = self.activation.take()
            && let Some(tok) = act.token.lock().ok().and_then(|mut t| t.take())
        {
            tok.destroy();
        }
        self.shared.flush();
    }
}

/// Update the size of the quick terminal based on monitor dimensions.
fn entered_monitor(host: &dyn WindowHost, monitor: &gdk::Monitor) {
    let Some(window) = host.gtk_window() else {
        return;
    };
    let Some(config) = host.config() else {
        return;
    };
    let size = monitor.geometry();
    let dims = config.quick_terminal_size.calculate(
        config.quick_terminal_position,
        QuickTerminalDimensions {
            width: size.width() as u32,
            height: size.height() as u32,
        },
    );
    window.set_default_size(dims.width as i32, dims.height as i32);
}

// ---- wayland-client dispatch plumbing ---------------------------------

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_registry::Event;
        match event {
            Event::Global {
                name, interface, ..
            } => state.registry_global(registry, qh, name, &interface),
            Event::GlobalRemove { name } => state.registry_global_remove(name),
            _ => {}
        }
    }
}

macro_rules! no_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(
                _: &mut Self,
                _: &$t,
                _: <$t as Proxy>::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

no_events!(
    OrgKdeKwinBlurManager,
    OrgKdeKwinBlur,
    OrgKdeKwinSlideManager,
    OrgKdeKwinSlide,
    XdgActivationV1,
    OrgKdeKwinServerDecoration
);

impl Dispatch<OrgKdeKwinServerDecorationManager, ()> for State {
    fn event(
        state: &mut Self,
        _: &OrgKdeKwinServerDecorationManager,
        event: org_kde_kwin_server_decoration_manager::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let org_kde_kwin_server_decoration_manager::Event::DefaultMode { mode } = event {
            state.default_deco_mode = Some(mode);
        }
    }
}

impl Dispatch<XdgActivationTokenV1, Arc<ActivationData>> for State {
    fn event(
        state: &mut Self,
        token: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        data: &Arc<ActivationData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_activation_token_v1::Event::Done { token: id } = event else {
            return;
        };
        // Ignore events for tokens we've already dropped.
        let is_current = data
            .token
            .lock()
            .is_ok_and(|t| t.as_ref().is_some_and(|t| t.id() == token.id()));
        if !is_current {
            log::warn!("received event for unknown activation token; ignoring");
            return;
        }
        if let Some(activation) = &state.xdg_activation {
            activation.activate(id, &data.surface);
        }
        token.destroy();
        if let Ok(mut t) = data.token.lock() {
            t.take();
        }
        data.done.store(true, Ordering::Relaxed);
        state.pending_tokens = state.pending_tokens.saturating_sub(1);
    }
}
