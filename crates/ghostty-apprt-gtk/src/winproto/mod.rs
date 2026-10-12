//! Port of apprt/gtk/winproto.zig: abstraction over the windowing protocol
//! (X11 / Wayland / none) in use by the GDK display.
//!
//! * [`App`] is application-wide protocol state (one per application).
//! * [`Window`] is per-toplevel protocol state.
//!
//! Zig's tagged unions over `noop`/`x11`/`wayland` become Rust enums. The Zig
//! code held a `*ApprtWindow` pointer; here the window GObject implements
//! [`WindowHost`] (a small, weak-referencing adapter), see below.

use std::{rc::Rc, sync::Arc};

use ghostty_config::{config::Config, types_b::WindowDecoration};
use ghostty_input::Mods;
use gtk::{gdk, prelude::*};

pub mod noop;
pub mod wayland;
pub mod x11;

/// Errors from the winproto backends.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("window surface is not an X11 surface")]
    NotX11Surface,
    #[error("window surface is not a Wayland surface")]
    NotWaylandSurface,
    #[error("no Wayland display/surface available")]
    NoWaylandDisplay,
    #[error("error initializing the XKB extension")]
    XkbInitialization,
    #[error("X11 request failed")]
    RequestFailed,
    #[error("X11 property not found")]
    PropertyNotFound,
    #[error("X11 property type mismatch")]
    PropertyTypeMismatch,
    #[error("X11 property format mismatch")]
    PropertyFormatMismatch,
    #[error("wayland: {0}")]
    Wayland(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What the winproto backends need from the Ghostty window GObject
/// (Zig: `ApprtWindow`). Implemented by `class::window` using a weak
/// reference to the window (so winproto state, which the window owns, does not
/// create a reference cycle).
pub trait WindowHost {
    /// `apprt_window.as(gtk.Window)` (also a `gtk::Native`).
    fn gtk_window(&self) -> Option<gtk::Window>;
    /// `apprt_window.isQuickTerminal()`.
    fn is_quick_terminal(&self) -> bool;
    /// `apprt_window.getWindowDecoration()`.
    fn window_decoration(&self) -> WindowDecoration;
    /// `apprt_window.getConfig()?.get()`.
    fn config(&self) -> Option<Arc<Config>>;
}

/// Which protocol is active (Zig: `winproto.Protocol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    None,
    Wayland,
    X11,
}

/// App-state for the underlying windowing protocol.
pub enum App {
    None(noop::App),
    Wayland(wayland::App),
    X11(x11::App),
}

impl App {
    /// Probe wayland, then x11, falling back to `None` (noop).
    pub fn init(gdk_display: &gdk::Display, app_id: &str, config: &Config) -> Result<App> {
        if let Some(v) = wayland::App::init(gdk_display, app_id, config)? {
            return Ok(App::Wayland(v));
        }
        if let Some(v) = x11::App::init(gdk_display, app_id, config)? {
            return Ok(App::X11(v));
        }
        Ok(App::None(noop::App))
    }

    pub fn protocol(&self) -> Protocol {
        match self {
            App::None(_) => Protocol::None,
            App::Wayland(_) => Protocol::Wayland,
            App::X11(_) => Protocol::X11,
        }
    }

    /// Modifier state for a key event, falling back to the translation of
    /// the GDK modifiers (`key.translateMods`).
    pub fn event_mods(&self, device: Option<&gdk::Device>, gtk_mods: gdk::ModifierType) -> Mods {
        let v = match self {
            App::None(v) => v.event_mods(device, gtk_mods),
            App::Wayland(v) => v.event_mods(device, gtk_mods),
            App::X11(v) => v.event_mods(device, gtk_mods),
        };
        v.unwrap_or_else(|| crate::key::translate_mods(gtk_mods))
    }

    pub fn supports_quick_terminal(&self) -> bool {
        match self {
            App::None(v) => v.supports_quick_terminal(),
            App::Wayland(v) => v.supports_quick_terminal(),
            App::X11(v) => v.supports_quick_terminal(),
        }
    }

    /// Set up necessary support for the quick terminal that must occur
    /// *before* the window-level winproto object is created (and before the
    /// window is realized). Only has an effect on Wayland, where
    /// gtk4-layer-shell is initialized for the window.
    pub fn init_quick_terminal(&mut self, window: &gtk::Window) -> Result<()> {
        match self {
            App::None(v) => v.init_quick_terminal(window),
            App::Wayland(v) => v.init_quick_terminal(window),
            App::X11(v) => v.init_quick_terminal(window),
        }
    }
}

/// Per-Window state for the underlying windowing protocol.
pub enum Window {
    None(noop::Window),
    Wayland(wayland::Window),
    X11(x11::Window),
}

impl Window {
    /// Create the per-window state. The window must be realized (it needs its
    /// `gdk::Surface`).
    pub fn init(app: &App, host: Rc<dyn WindowHost>) -> Result<Window> {
        Ok(match app {
            App::None(a) => Window::None(noop::Window::init(a, host)?),
            App::Wayland(a) => Window::Wayland(wayland::Window::init(a, host)?),
            App::X11(a) => Window::X11(x11::Window::init(a, host)?),
        })
    }

    pub fn resize_event(&mut self) -> Result<()> {
        match self {
            Window::None(v) => v.resize_event(),
            Window::Wayland(v) => v.resize_event(),
            Window::X11(v) => v.resize_event(),
        }
    }

    pub fn sync_appearance(&mut self) -> Result<()> {
        match self {
            Window::None(v) => v.sync_appearance(),
            Window::Wayland(v) => v.sync_appearance(),
            Window::X11(v) => v.sync_appearance(),
        }
    }

    /// True if CSD is currently enabled for this window (actual state, not
    /// desired state).
    pub fn client_side_decoration_enabled(&self) -> bool {
        match self {
            Window::None(v) => v.client_side_decoration_enabled(),
            Window::Wayland(v) => v.client_side_decoration_enabled(),
            Window::X11(v) => v.client_side_decoration_enabled(),
        }
    }

    pub fn add_subprocess_env(&mut self, env: &mut Vec<(String, String)>) -> Result<()> {
        match self {
            Window::None(v) => v.add_subprocess_env(env),
            Window::Wayland(v) => v.add_subprocess_env(env),
            Window::X11(v) => v.add_subprocess_env(env),
        }
    }

    pub fn set_urgent(&mut self, urgent: bool) -> Result<()> {
        match self {
            Window::None(v) => v.set_urgent(urgent),
            Window::Wayland(v) => v.set_urgent(urgent),
            Window::X11(v) => v.set_urgent(urgent),
        }
    }
}

/// Helper: widget scale factor etc. used by backends.
pub(crate) fn native_surface(w: &gtk::Window) -> Option<gdk::Surface> {
    w.upcast_ref::<gtk::Native>().surface()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_defaults() {
        let a = noop::App;
        assert!(!a.supports_quick_terminal());
        assert!(a.event_mods(None, gdk::ModifierType::empty()).is_none());
    }
}
