//! Port of apprt/gtk/winproto/noop.zig: used when neither X11 nor Wayland is
//! detected.

use std::rc::Rc;

use gtk::gdk;

use super::{Result, WindowHost};
use ghostty_input::Mods;

#[derive(Debug, Default, Clone, Copy)]
pub struct App;

impl App {
    pub fn event_mods(&self, _: Option<&gdk::Device>, _: gdk::ModifierType) -> Option<Mods> {
        None
    }

    pub fn supports_quick_terminal(&self) -> bool {
        false
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn init_quick_terminal(&mut self, _: &gtk::Window) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Window;

impl Window {
    #[allow(clippy::unnecessary_wraps)]
    pub fn init(_: &App, _: Rc<dyn WindowHost>) -> Result<Window> {
        Ok(Window)
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn resize_event(&mut self) -> Result<()> {
        Ok(())
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn sync_appearance(&mut self) -> Result<()> {
        Ok(())
    }

    /// Always true: this should be the actual present state of the window.
    #[allow(clippy::unused_self)]
    pub fn client_side_decoration_enabled(&self) -> bool {
        true
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn add_subprocess_env(&mut self, _: &mut Vec<(String, String)>) -> Result<()> {
        Ok(())
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn set_urgent(&mut self, _: bool) -> Result<()> {
        Ok(())
    }
}
