//! Port of apprt/gtk/winproto/x11.zig: X11 window protocol implementation.
//!
//! Uses Xlib (through the `x11` crate, sharing GDK's `Display*`) exactly like
//! the Zig code; all raw FFI is confined to small `unsafe` blocks with SAFETY
//! comments.
#![allow(unsafe_code)]

use std::ffi::{c_int, c_long, c_uchar, c_uint, c_ulong, c_void};
use std::rc::Rc;

use gdk4_x11::x11::xlib;
use gdk4_x11::{X11Display, X11Surface};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use super::{Error, Result, WindowHost};
use ghostty_input::Mods;

// XKB bits we need that the `x11` crate doesn't expose (XKBlib.h / XKB.h).
const XKB_MAJOR_VERSION: c_int = 1;
const XKB_MINOR_VERSION: c_int = 0;
const XKB_USE_CORE_KBD: c_uint = 0x0100;
const XKB_STATE_NOTIFY: c_int = 2;
const QUEUED_AFTER_READING: c_int = 1;
const XKB_MODIFIER_STATE_MASK: c_ulong = 1 << 0;

#[link(name = "X11")]
unsafe extern "C" {
    fn XkbQueryExtension(
        dpy: *mut c_void,
        opcode_rtrn: *mut c_int,
        event_rtrn: *mut c_int,
        error_rtrn: *mut c_int,
        major_in_out: *mut c_int,
        minor_in_out: *mut c_int,
    ) -> c_int;

    fn XkbSelectEventDetails(
        dpy: *mut c_void,
        device_spec: c_uint,
        event_type: c_uint,
        bits_to_change: c_ulong,
        values_for_bits: c_ulong,
    ) -> c_int;
}

/// Leading part of `XkbStateNotifyEvent` (XKBlib.h) up to `lookup_mods`.
#[repr(C)]
#[allow(dead_code)]
struct XkbStateNotifyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut xlib::Display,
    time: c_ulong,
    xkb_type: c_int,
    device: c_int,
    changed: c_uint,
    group: c_int,
    base_group: c_int,
    latched_group: c_int,
    locked_group: c_int,
    mods: c_uint,
    base_mods: c_uint,
    latched_mods: c_uint,
    locked_mods: c_uint,
    compat_state: c_int,
    grab_mods: c_uchar,
    compat_grab_mods: c_uchar,
    lookup_mods: c_uchar,
}

#[derive(Debug, Clone, Copy)]
struct Atoms {
    kde_blur: xlib::Atom,
    motif_wm_hints: xlib::Atom,
}

impl Atoms {
    fn new(display: &X11Display) -> Atoms {
        Atoms {
            kde_blur: gdk4_x11::x11_get_xatom_by_name_for_display(
                display,
                "_KDE_NET_WM_BLUR_BEHIND_REGION",
            ),
            motif_wm_hints: gdk4_x11::x11_get_xatom_by_name_for_display(display, "_MOTIF_WM_HINTS"),
        }
    }
}

/// X11 application-wide state.
pub struct App {
    xl: Rc<xlib::Xlib>,
    display: *mut xlib::Display,
    base_event_code: c_int,
    atoms: Atoms,
}

impl App {
    /// Returns `Ok(None)` if the display isn't X11.
    pub fn init(
        gdk_display: &gdk::Display,
        app_id: &str,
        config: &ghostty_config::config::Config,
    ) -> Result<Option<App>> {
        let Some(x11_display) = gdk_display.downcast_ref::<X11Display>() else {
            return Ok(None);
        };

        let xl = xlib::Xlib::open().map_err(|e| {
            log::error!("failed to load libX11: {e}");
            Error::XkbInitialization
        })?;

        // SAFETY: the display is a live X11 display; the returned pointer is
        // owned by GDK and valid for the lifetime of the display (the whole
        // application).
        let xlib_display = unsafe { x11_display.xdisplay() };

        let program_name = match &config.x11_instance_name {
            Some(pn) => pn.clone(),
            None if cfg!(debug_assertions) => "ghostty-debug".to_string(),
            None => "ghostty".to_string(),
        };

        // Set the X11 window class property (WM_CLASS); the instance name is
        // derived from g_set_prgname. Results in e.g.
        //   WM_CLASS(STRING) = "ghostty", "com.mitchellh.ghostty"
        glib::set_prgname(Some(program_name.as_str()));
        x11_display.set_program_class(app_id);

        log::debug!("Xkb.init: initializing Xkb");
        let mut opcode: c_int = 0;
        let mut base_event_code: c_int = 0;
        let mut base_error_code: c_int = 0;
        let mut major = XKB_MAJOR_VERSION;
        let mut minor = XKB_MINOR_VERSION;
        // SAFETY: valid display pointer, valid out-pointers to locals.
        let ok = unsafe {
            XkbQueryExtension(
                xlib_display.cast(),
                &raw mut opcode,
                &raw mut base_event_code,
                &raw mut base_error_code,
                &raw mut major,
                &raw mut minor,
            )
        };
        if ok == 0 {
            log::error!(
                "Fatal: error initializing Xkb extension: error executing XkbQueryExtension"
            );
            return Err(Error::XkbInitialization);
        }

        // SAFETY: valid display pointer.
        let ok = unsafe {
            XkbSelectEventDetails(
                xlib_display.cast(),
                XKB_USE_CORE_KBD,
                XKB_STATE_NOTIFY as c_uint,
                XKB_MODIFIER_STATE_MASK,
                XKB_MODIFIER_STATE_MASK,
            )
        };
        if ok == 0 {
            log::error!(
                "Fatal: error initializing Xkb extension: error executing XkbSelectEventDetails"
            );
            return Err(Error::XkbInitialization);
        }

        Ok(Some(App {
            xl: Rc::new(xl),
            display: xlib_display,
            base_event_code,
            atoms: Atoms::new(x11_display),
        }))
    }

    /// Checks for an immediate pending XKB state update event and returns the
    /// keyboard state based on it. Necessary because the standard GTK X11 API
    /// does not include the currently pressed key in the modifier snapshot
    /// (paraphrased from Firefox/Gecko widget/gtk/nsGtkKeyUtils.cpp).
    ///
    /// Returns `None` if there is no such event; callers fall back to the GDK
    /// modifier state.
    pub fn event_mods(&self, _: Option<&gdk::Device>, _: gdk::ModifierType) -> Option<Mods> {
        // SAFETY: `display` is valid for the application's lifetime; XEvent is
        // plain-old-data, zero-initialised, and filled by XPeekEvent.
        let lookup_mods = unsafe {
            if (self.xl.XEventsQueued)(self.display, QUEUED_AFTER_READING) == 0 {
                return None;
            }
            let mut next: xlib::XEvent = std::mem::zeroed();
            (self.xl.XPeekEvent)(self.display, &raw mut next);
            if next.type_ != self.base_event_code {
                return None;
            }
            // XkbStateNotifyEvent is a prefix-compatible view of the XEvent
            // union (which is at least as large and aligned as a long).
            let ev = &*std::ptr::from_ref(&next).cast::<XkbStateNotifyEvent>();
            if ev.xkb_type != XKB_STATE_NOTIFY {
                return None;
            }
            ev.lookup_mods
        };

        let lookup = c_uint::from(lookup_mods);
        let mut mods = Mods::default();
        log::debug!("X11: found extra XkbStateNotify event w/lookup_mods: {lookup_mods:b}");
        if lookup & xlib::ShiftMask != 0 {
            mods.shift = true;
        }
        if lookup & xlib::ControlMask != 0 {
            mods.ctrl = true;
        }
        if lookup & xlib::Mod1Mask != 0 {
            mods.alt = true;
        }
        if lookup & xlib::Mod4Mask != 0 {
            mods.super_ = true;
        }
        if lookup & xlib::LockMask != 0 {
            mods.caps_lock = true;
        }
        Some(mods)
    }

    #[allow(clippy::unused_self)]
    pub fn supports_quick_terminal(&self) -> bool {
        log::warn!("quick terminal is not yet supported on X11");
        false
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn init_quick_terminal(&mut self, _: &gtk::Window) -> Result<()> {
        Ok(())
    }
}

/// `_KDE_NET_WM_BLUR_BEHIND_REGION` payload: x, y, width, height (32-bit
/// format properties are arrays of `long`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
struct Region {
    x: c_long,
    y: c_long,
    width: c_long,
    height: c_long,
}

/// See Xm/MwmUtil.h, packaged with the Motif Window Manager. All fields are
/// `long`-sized in the 32-bit format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
struct MotifWmHints {
    flags: c_ulong,
    functions: c_ulong,
    decorations: c_ulong,
    input_mode: c_long,
    status: c_ulong,
}

impl MotifWmHints {
    const FLAG_DECORATIONS: c_ulong = 1 << 1;
    const DECORATIONS_ALL: c_ulong = 1 << 0;
}

/// X11 per-window state.
pub struct Window {
    xl: Rc<xlib::Xlib>,
    display: *mut xlib::Display,
    atoms: Atoms,
    host: Rc<dyn WindowHost>,
    x11_surface: X11Surface,

    blur_region: Region,

    // Cache last applied values to avoid redundant X11 property updates, which
    // cause visual glitches with some WMs (ghostty-org/ghostty#8075).
    last_applied_blur_region: Option<Region>,
    last_applied_decoration_hints: Option<MotifWmHints>,
}

impl Window {
    pub fn init(app: &App, host: Rc<dyn WindowHost>) -> Result<Window> {
        let win = host.gtk_window().ok_or(Error::NotX11Surface)?;
        let surface = super::native_surface(&win).ok_or(Error::NotX11Surface)?;
        let x11_surface = surface
            .downcast::<X11Surface>()
            .map_err(|_| Error::NotX11Surface)?;
        Ok(Window {
            xl: Rc::clone(&app.xl),
            display: app.display,
            atoms: app.atoms,
            host,
            x11_surface,
            blur_region: Region::default(),
            last_applied_blur_region: None,
            last_applied_decoration_hints: None,
        })
    }

    pub fn resize_event(&mut self) -> Result<()> {
        // The blur region must update with window resizes
        self.sync_blur()
    }

    pub fn sync_appearance(&mut self) -> Result<()> {
        // The user could have toggled between CSDs and SSDs, therefore we need
        // to recalculate the blur region offset.
        if let Some(win) = self.host.gtk_window() {
            let (mut x, mut y) = win.upcast_ref::<gtk::Native>().surface_transform();
            // Transform surface coordinates to device coordinates.
            let scale = f64::from(win.scale_factor());
            x *= scale;
            y *= scale;
            self.blur_region = Region {
                x: x as c_long,
                y: y as c_long,
                ..Region::default()
            };
        }
        if let Err(err) = self.sync_blur() {
            log::error!("failed to synchronize blur={err}");
        }
        if let Err(err) = self.sync_decorations() {
            log::error!("failed to synchronize decorations={err}");
        }
        Ok(())
    }

    pub fn client_side_decoration_enabled(&self) -> bool {
        use ghostty_config::types_b::WindowDecoration as D;
        match self.host.window_decoration() {
            D::Auto | D::Client => true,
            D::Server | D::None => false,
        }
    }

    fn sync_blur(&mut self) -> Result<()> {
        // FIXME (upstream): doesn't factor in rounded corners on Adwaita.
        let Some(win) = self.host.gtk_window() else {
            return Ok(());
        };
        let Some(config) = self.host.config() else {
            return Ok(());
        };

        // When blur is disabled, remove the property if it was previously set
        let blur = config.background_blur;
        if !blur.enabled() {
            if self.last_applied_blur_region.is_some() {
                self.delete_property(self.atoms.kde_blur)?;
                self.last_applied_blur_region = None;
            }
            return Ok(());
        }

        // Transform surface coordinates to device coordinates.
        let scale = win.scale_factor();
        self.blur_region.width = c_long::from(win.width() * scale);
        self.blur_region.height = c_long::from(win.height() * scale);

        // Only update X11 properties when the blur region actually changes
        if self.last_applied_blur_region == Some(self.blur_region) {
            return Ok(());
        }

        log::debug!(
            "set blur={blur:?}, window xid={}, region={:?}",
            self.x11_surface.xid(),
            self.blur_region
        );

        let region = self.blur_region;
        self.change_property(self.atoms.kde_blur, xlib::XA_CARDINAL, &region)?;
        self.last_applied_blur_region = Some(region);
        Ok(())
    }

    fn sync_decorations(&mut self) -> Result<()> {
        use ghostty_config::types_b::WindowDecoration as D;

        let mut hints = match self.get_window_property::<MotifWmHints>(
            self.atoms.motif_wm_hints,
            self.atoms.motif_wm_hints,
        ) {
            Ok(h) => h,
            // already default-initialised, so this is fine
            Err(Error::PropertyNotFound) => MotifWmHints::default(),
            Err(e) => return Err(e),
        };

        hints.flags |= MotifWmHints::FLAG_DECORATIONS;
        if matches!(self.host.window_decoration(), D::Server) {
            hints.decorations |= MotifWmHints::DECORATIONS_ALL;
        } else {
            hints.decorations &= !MotifWmHints::DECORATIONS_ALL;
        }

        // Only update decoration hints when they actually change
        if self.last_applied_decoration_hints == Some(hints) {
            return Ok(());
        }

        self.change_property(self.atoms.motif_wm_hints, self.atoms.motif_wm_hints, &hints)?;
        self.last_applied_decoration_hints = Some(hints);
        Ok(())
    }

    pub fn add_subprocess_env(&mut self, env: &mut Vec<(String, String)>) -> Result<()> {
        env.push(("WINDOWID".to_string(), self.x11_surface.xid().to_string()));
        Ok(())
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn set_urgent(&mut self, urgent: bool) -> Result<()> {
        self.x11_surface.set_urgency_hint(urgent);
        Ok(())
    }

    /// `XGetWindowProperty` for a 32-format property that is an array of
    /// `long`s whose layout matches `T` (`T` is a `repr(C)` struct of longs).
    fn get_window_property<T: Copy>(&self, name: xlib::Atom, typ: xlib::Atom) -> Result<T> {
        let mut actual_type: xlib::Atom = 0;
        let mut actual_format: c_int = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut prop: *mut c_uchar = std::ptr::null_mut();

        // SAFETY: valid display and window id; all out-pointers refer to locals.
        let code = unsafe {
            (self.xl.XGetWindowProperty)(
                self.display,
                self.x11_surface.xid(),
                name,
                0,
                c_long::MAX,
                xlib::False,
                typ,
                &raw mut actual_type,
                &raw mut actual_format,
                &raw mut nitems,
                &raw mut bytes_after,
                &raw mut prop,
            )
        };
        if code != c_int::from(xlib::Success) {
            return Err(Error::RequestFailed);
        }
        if actual_type == 0 {
            return Err(Error::PropertyNotFound);
        }

        let result = if typ != actual_type {
            Err(Error::PropertyTypeMismatch)
        } else if actual_format != 32 {
            Err(Error::PropertyFormatMismatch)
        } else if (nitems as usize) * std::mem::size_of::<c_long>() < std::mem::size_of::<T>()
            || prop.is_null()
        {
            Err(Error::RequestFailed)
        } else {
            // SAFETY: Xlib returns 32-format data as an array of `long`
            // (suitably aligned), at least size_of::<T>() bytes of which we
            // just verified; T is a repr(C) aggregate of longs.
            Ok(unsafe { prop.cast::<T>().read() })
        };
        if !prop.is_null() {
            // SAFETY: allocated by Xlib.
            unsafe { (self.xl.XFree)(prop.cast()) };
        }
        result
    }

    fn change_property<T: Copy>(&self, name: xlib::Atom, typ: xlib::Atom, value: &T) -> Result<()> {
        let nelements = std::mem::size_of::<T>() / std::mem::size_of::<c_long>();
        // SAFETY: valid display/window; `value` points to `nelements` longs
        // (T is a repr(C) aggregate of longs); data is copied by Xlib.
        let status = unsafe {
            (self.xl.XChangeProperty)(
                self.display,
                self.x11_surface.xid(),
                name,
                typ,
                32,
                xlib::PropModeReplace,
                std::ptr::from_ref(value).cast::<c_uchar>(),
                nelements as c_int,
            )
        };
        // Xlib alternates between error values (0 = success) and booleans
        // (1 = success); the Zig code treats 0 as failure here.
        if status == 0 {
            return Err(Error::RequestFailed);
        }
        Ok(())
    }

    fn delete_property(&self, name: xlib::Atom) -> Result<()> {
        // SAFETY: valid display and window id.
        let status =
            unsafe { (self.xl.XDeleteProperty)(self.display, self.x11_surface.xid(), name) };
        if status == 0 {
            return Err(Error::RequestFailed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_layouts_are_long_arrays() {
        let l = std::mem::size_of::<c_long>();
        assert_eq!(std::mem::size_of::<Region>(), 4 * l);
        assert_eq!(std::mem::size_of::<MotifWmHints>(), 5 * l);
    }
}
