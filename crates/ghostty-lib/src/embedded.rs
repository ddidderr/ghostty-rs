//! The embedded application runtime (ZIG/src/apprt/embedded.zig): the
//! runtime side of `ghostty-core` implemented over the C callbacks of
//! `ghostty_runtime_config_s`, plus the `ghostty_app_*` / `ghostty_surface_*`
//! exports.
//!
//! Everything here is main-thread only, like the Zig embedded runtime: the
//! host calls the C API from its UI thread. The only cross-thread entry is
//! the `wakeup` callback, invoked by core threads through [`WakerImpl`].
//!
//! Handles: `ghostty_app_t` is a `Box<AppHandle>` and `ghostty_surface_t` a
//! `Box<Surface>`. The address of the surface object doubles as the
//! `SurfaceId` core knows the surface by, and as the `ghostty_surface_t` in
//! `ghostty_target_s` of actions (Zig: `target.cval()` gives `rt_surface`).
#![allow(unsafe_code)]

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fmt::Write as _,
    rc::{Rc, Weak},
    sync::Arc,
};

use ghostty_apprt::{
    action::{Action, GotoSplit, ResizeSplit, ResizeSplitDirection, SplitDirection, Target},
    runtime_api::{CoreApp, CoreSurface, RtApp, RtSurface, SurfaceId, Waker},
    structs::{Clipboard, ClipboardRequest, ColorScheme, ContentScale, CursorPos, SurfaceSize},
    surface::{NewSurfaceContext, new_config, should_inherit_working_directory},
};
use ghostty_config::{command::Command, config::Config, io::ReadableIO, types_a::WorkingDirectory};
use ghostty_core::{App as CoreAppImpl, Surface as CoreSurfaceImpl, surface::binding::Text};
use ghostty_input::{
    Action as KeyAction,
    Key,
    KeyEvent,
    Mods,
    MouseButton,
    MouseButtonState,
    MousePressureStage,
    ScrollMods,
    binding::Action as BindingAction,
    keycodes,
};
use ghostty_terminal::{
    point::{Coordinate, Point, Tag as PointTag},
    selection::Selection,
    terminal::Terminal,
};

use crate::{
    action_c::{Keep, action_to_c, target_to_c},
    config_capi::cconfig,
    main_c::{alloc_c_text, free_c_text},
    types::{
        ghostty_app_t,
        ghostty_binding_flags_e,
        ghostty_clipboard_content_s,
        ghostty_config_t,
        ghostty_input_key_s,
        ghostty_point_s,
        ghostty_runtime_config_s,
        ghostty_selection_s,
        ghostty_surface_config_s,
        ghostty_surface_size_s,
        ghostty_surface_t,
        ghostty_text_s,
        platform,
    },
};

// ---------------------------------------------------------------------
// Waker

/// `App.wakeup`: invoked from any core thread; calls the host's `wakeup_cb`.
struct WakerImpl {
    cb: unsafe extern "C" fn(*mut c_void),
    userdata: usize,
}

impl Waker for WakerImpl {
    fn wake(&self) {
        // SAFETY: the embedder promises `wakeup_cb` may be called from any
        // thread with the `userdata` it supplied (the documented contract of
        // `ghostty_runtime_wakeup_cb`).
        unsafe { (self.cb)(self.userdata as *mut c_void) };
    }
}

// ---------------------------------------------------------------------
// Runtime app

/// The runtime side of the app (Zig: `embedded.App`), handed to core as
/// `Rc<dyn RtApp>`.
pub struct AppRt {
    opts: ghostty_runtime_config_s,
    /// The `ghostty_app_t` passed to the action callback.
    app_ptr: Cell<usize>,
    /// The configuration for the app (a clone, owned by the app).
    config: RefCell<Arc<Config>>,
    surfaces: RefCell<HashMap<SurfaceId, Weak<SurfaceRt>>>,
    keymap: RefCell<ghostty_input::Keymap>,
}

impl AppRt {
    fn surface_rt(&self, id: SurfaceId) -> Option<Rc<SurfaceRt>> {
        self.surfaces.borrow().get(&id).and_then(Weak::upgrade)
    }

    /// Special case certain actions before they are sent to the embedder
    /// (Zig: `performPreAction`).
    fn pre_action(&self, target: &Target<SurfaceId>, action: &Action) {
        match (action, target) {
            (Action::SetTitle(t), Target::Surface(id)) => {
                if let Some(s) = self.surface_rt(*id) {
                    *s.title.borrow_mut() = Some(t.title.clone());
                }
            }

            // For app updates, we update our core config. The caller owns
            // the param so we clone it.
            (Action::ConfigChange(c), Target::App) => {
                *self.config.borrow_mut() = Arc::clone(&c.config);
            }

            _ => {}
        }
    }

    /// Reload the keyboard layout (`App.reloadKeymap`).
    fn reload_keymap(&self) {
        if let Err(err) = self.keymap.borrow_mut().reload() {
            log::error!("error reloading keyboard map err={err:?}");
        }
    }
}

impl RtApp for AppRt {
    fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool {
        self.pre_action(&target, &action);

        log::debug!(
            "dispatching action target={target:?} action={:?}",
            action.key()
        );
        let Some(cb) = self.opts.action_cb else {
            return false;
        };
        let mut keep = Keep::default();
        let t = target_to_c(&target);
        let a = action_to_c(&action, &mut keep);
        // SAFETY: the embedder supplied the callback; it receives the app
        // handle it knows plus plain-data structs whose pointers stay valid
        // (`keep`) for the duration of the call.
        unsafe { cb(self.app_ptr.get() as ghostty_app_t, t, a) }
    }

    fn keyboard_layout(&self) -> ghostty_input::KeyboardLayout {
        keyboard_layout(&self.keymap.borrow())
    }
}

/// `App.keyboardLayout`: we only support keyboard layout detection on macOS.
#[cfg(target_os = "macos")]
fn keyboard_layout(keymap: &ghostty_input::Keymap) -> ghostty_input::KeyboardLayout {
    keymap
        .source_id()
        .and_then(|id| ghostty_input::KeyboardLayout::map_apple_id(&id))
        .unwrap_or(ghostty_input::KeyboardLayout::Unknown)
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::trivially_copy_pass_by_ref)]
fn keyboard_layout(_keymap: &ghostty_input::Keymap) -> ghostty_input::KeyboardLayout {
    ghostty_input::KeyboardLayout::Unknown
}

/// The app object behind `ghostty_app_t`.
pub struct AppHandle {
    core: Rc<CoreAppImpl>,
    rt: Rc<AppRt>,
}

/// Borrow the app behind a handle.
///
/// # Safety
/// `p` must be null or a live handle from `ghostty_app_new`.
unsafe fn app<'a>(p: ghostty_app_t) -> Option<&'a AppHandle> {
    // SAFETY: per the function contract.
    unsafe { p.cast::<AppHandle>().as_ref() }
}

/// Borrow a C string.
///
/// # Safety
/// `p` must be null or NUL terminated.
unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a CStr> {
    if p.is_null() {
        None
    } else {
        // SAFETY: per the function contract.
        Some(unsafe { CStr::from_ptr(p) })
    }
}

fn mods_from_c(v: c_int) -> Mods {
    // `@truncate(@as(c_uint, @bitCast(mods)))` to the u16 backing.
    Mods::from_int(v as u32 as u16)
}

/// An owned key event converted from the C struct (Zig: `KeyEvent.core`).
struct OwnedKey {
    action: KeyAction,
    key: Key,
    mods: Mods,
    consumed_mods: Mods,
    composing: bool,
    text: String,
    unshifted_codepoint: u32,
}

impl OwnedKey {
    /// `None` if the action is invalid.
    ///
    /// # Safety
    /// `text` must be null or NUL terminated.
    unsafe fn from_c(e: &ghostty_input_key_s) -> Option<OwnedKey> {
        let action = match e.action {
            0 => KeyAction::Release,
            1 => KeyAction::Press,
            2 => KeyAction::Repeat,
            _ => return None,
        };
        // SAFETY: per the function contract.
        let text = unsafe { cstr(e.text) }
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default();

        // We want to get the physical unmapped key to process keybinds.
        let key = keycodes::entries()
            .iter()
            .find(|entry| entry.native == e.keycode)
            .map_or(Key::Unidentified, |entry| entry.key);

        // The unshifted codepoint is a `u21`; anything larger is 0.
        let unshifted_codepoint = if e.unshifted_codepoint <= 0x1F_FFFF {
            e.unshifted_codepoint
        } else {
            0
        };

        Some(OwnedKey {
            action,
            key,
            mods: mods_from_c(e.mods),
            consumed_mods: mods_from_c(e.consumed_mods),
            composing: e.composing,
            text,
            unshifted_codepoint,
        })
    }

    fn event(&self) -> KeyEvent<'_> {
        KeyEvent {
            action: self.action,
            key: self.key,
            mods: self.mods,
            consumed_mods: self.consumed_mods,
            composing: self.composing,
            utf8: &self.text,
            unshifted_codepoint: self.unshifted_codepoint,
        }
    }
}

// ---------------------------------------------------------------------
// App exports

/// Create a new app.
///
/// # Safety
/// `opts` and `config` must be valid pointers (the config a handle from
/// `ghostty_config_new`/`_clone`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_new(
    opts: *const ghostty_runtime_config_s,
    config: ghostty_config_t,
) -> ghostty_app_t {
    // SAFETY: per the function contract.
    let (Some(opts), Some(config)) = (unsafe { opts.as_ref() }, unsafe { cconfig(config) }) else {
        log::error!("error initializing app err=NullArgument");
        return std::ptr::null_mut();
    };
    let Some(wakeup) = opts.wakeup_cb else {
        log::error!("error initializing app err=MissingWakeupCallback");
        return std::ptr::null_mut();
    };

    let waker = Arc::new(WakerImpl {
        cb: wakeup,
        userdata: opts.userdata as usize,
    });
    let core = match CoreAppImpl::new(waker) {
        Ok(c) => Rc::new(c),
        Err(err) => {
            log::error!("error initializing app err={err}");
            return std::ptr::null_mut();
        }
    };
    let keymap = match ghostty_input::Keymap::new() {
        Ok(k) => k,
        Err(err) => {
            log::error!("error initializing app err={err:?}");
            return std::ptr::null_mut();
        }
    };

    // We have to clone the config.
    let rt = Rc::new(AppRt {
        opts: *opts,
        app_ptr: Cell::new(0),
        config: RefCell::new(Arc::new(config.config.clone())),
        surfaces: RefCell::default(),
        keymap: RefCell::new(keymap),
    });
    let ptr = Box::into_raw(Box::new(AppHandle { core, rt }));
    // SAFETY: just allocated.
    unsafe { &*ptr }.rt.app_ptr.set(ptr as usize);
    ptr.cast::<c_void>()
}

/// Tick the event loop. This should be called whenever the "wakeup" callback
/// is invoked for the runtime.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_tick(v: ghostty_app_t) {
    // SAFETY: per the function contract.
    let Some(a) = (unsafe { app(v) }) else {
        return;
    };
    if let Err(err) = a.core.tick(&*a.rt) {
        log::error!("error app tick err={err}");
    }
}

/// Return the userdata associated with the app.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_userdata(v: ghostty_app_t) -> *mut c_void {
    // SAFETY: per the function contract.
    unsafe { app(v) }.map_or(std::ptr::null_mut(), |a| a.rt.opts.userdata)
}

/// Free the app.
///
/// # Safety
/// `v` must be a live app handle with no surfaces left; it is invalid
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_free(v: ghostty_app_t) {
    if v.is_null() {
        return;
    }
    // SAFETY: created by `ghostty_app_new` via `Box::into_raw`.
    drop(unsafe { Box::from_raw(v.cast::<AppHandle>()) });
}

/// Update the focused state of the app.
///
/// # Safety
/// `app` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_set_focus(handle: ghostty_app_t, focused: bool) {
    // SAFETY: per the function contract.
    if let Some(a) = unsafe { app(handle) } {
        a.core.focus_event(focused);
    }
}

/// Notify the app of a global keypress capture. This will return true if the
/// key was captured by the app, in which case the caller should not process
/// the key.
///
/// # Safety
/// `handle` must be a live app handle; `event.text` null or NUL terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_key(
    handle: ghostty_app_t,
    event: ghostty_input_key_s,
) -> bool {
    // SAFETY: per the function contract.
    let Some(a) = (unsafe { app(handle) }) else {
        return false;
    };
    // SAFETY: per the function contract.
    let Some(key) = (unsafe { OwnedKey::from_c(&event) }) else {
        return false;
    };
    // Never hold the borrow across the call: actions call into the host.
    let config = Arc::clone(&a.rt.config.borrow());
    a.core.key_event(&*a.rt, &config, &key.event())
}

/// Returns true if the given key event would trigger a binding if it were
/// sent to the surface right now. The "right now" is important because
/// things like trigger sequences are only valid until the next key event.
///
/// # Safety
/// As [`ghostty_app_key`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_key_is_binding(
    handle: ghostty_app_t,
    event: ghostty_input_key_s,
) -> bool {
    // SAFETY: per the function contract.
    let Some(a) = (unsafe { app(handle) }) else {
        return false;
    };
    // SAFETY: per the function contract.
    let Some(key) = (unsafe { OwnedKey::from_c(&event) }) else {
        log::warn!("error processing key event");
        return false;
    };
    let config = Arc::clone(&a.rt.config.borrow());
    a.core.key_event_is_binding(&config, &key.event())
}

/// Notify the app that the keyboard was changed. This causes the keyboard
/// layout to be reloaded from the OS.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_keyboard_changed(v: ghostty_app_t) {
    // SAFETY: per the function contract.
    if let Some(a) = unsafe { app(v) } {
        a.rt.reload_keymap();
    }
}

/// Open the configuration.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_open_config(v: ghostty_app_t) {
    // SAFETY: per the function contract.
    if let Some(a) = unsafe { app(v) } {
        a.rt.perform_action(Target::App, Action::OpenConfig);
    }
}

/// Update the configuration to the provided config. This will propagate to
/// all surfaces as well.
///
/// # Safety
/// `v` a live app handle, `config` a config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_update_config(v: ghostty_app_t, config: ghostty_config_t) {
    // SAFETY: per the function contract.
    let (Some(a), Some(c)) = (unsafe { app(v) }, unsafe { cconfig(config) }) else {
        return;
    };
    let cfg = Arc::new(c.config.clone());
    if let Err(err) = a.core.update_config(&*a.rt, &cfg) {
        log::error!("error updating config err={err}");
    }
}

/// Returns true if the app needs to confirm quitting.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_needs_confirm_quit(v: ghostty_app_t) -> bool {
    // SAFETY: per the function contract.
    unsafe { app(v) }.is_some_and(|a| a.core.needs_confirm_quit())
}

/// Returns true if the app has global keybinds.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_has_global_keybinds(v: ghostty_app_t) -> bool {
    use ghostty_input::binding::set::Value;
    // SAFETY: per the function contract.
    let Some(a) = (unsafe { app(v) }) else {
        return false;
    };
    let config = a.rt.config.borrow();
    config
        .keybind
        .set
        .bindings
        .values()
        .any(|value| match value {
            Value::Leader(_) => false,
            Value::Leaf(leaf) => leaf.flags.global,
            Value::LeafChained(leaf) => leaf.flags.global,
        })
}

fn color_scheme_from_c(v: c_int) -> Option<ColorScheme> {
    match v {
        0 => Some(ColorScheme::Light),
        1 => Some(ColorScheme::Dark),
        _ => None,
    }
}

/// Update the color scheme of the app.
///
/// # Safety
/// `v` must be a live app handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_app_set_color_scheme(v: ghostty_app_t, scheme_raw: c_int) {
    // SAFETY: per the function contract.
    let Some(a) = (unsafe { app(v) }) else {
        return;
    };
    let Some(scheme) = color_scheme_from_c(scheme_raw) else {
        log::warn!("invalid color scheme to ghostty_app_set_color_scheme value={scheme_raw}");
        return;
    };
    a.core.color_scheme_event(&*a.rt, scheme);
}

// ---------------------------------------------------------------------
// Runtime surface

/// The runtime side of a surface (Zig: `embedded.Surface` fields that core
/// reads), handed to core as `Rc<dyn RtSurface>`.
pub struct SurfaceRt {
    app: Rc<AppRt>,
    id: SurfaceId,
    userdata: *mut c_void,
    /// The `NSView` / `UIView` of the surface.
    view: *mut c_void,
    content_scale: Cell<ContentScale>,
    size: Cell<SurfaceSize>,
    cursor_pos: Cell<CursorPos>,
    /// The current title of the surface. The embedded apprt saves this so
    /// that `get_title` works without the implementer needing to save it.
    title: RefCell<Option<String>>,
}

impl SurfaceRt {
    fn target(&self) -> Target<SurfaceId> {
        Target::Surface(self.id)
    }
}

impl RtSurface for SurfaceRt {
    fn close(&self, process_active: bool) {
        let Some(cb) = self.app.opts.close_surface_cb else {
            log::info!("runtime embedder does not support closing a surface");
            return;
        };
        // SAFETY: embedder supplied callback with its own userdata.
        unsafe { cb(self.userdata, process_active) };
    }

    fn cgroup(&self) -> Option<String> {
        None
    }

    fn get_title(&self) -> Option<String> {
        self.title.borrow().clone()
    }

    fn get_content_scale(&self) -> ContentScale {
        self.content_scale.get()
    }

    fn get_size(&self) -> SurfaceSize {
        self.size.get()
    }

    fn get_cursor_pos(&self) -> CursorPos {
        self.cursor_pos.get()
    }

    fn supports_clipboard(&self, clipboard: Clipboard) -> bool {
        match clipboard {
            Clipboard::Standard => true,
            Clipboard::Selection | Clipboard::Primary => self.app.opts.supports_selection_clipboard,
        }
    }

    fn clipboard_request(&self, clipboard: Clipboard, state: ClipboardRequest) -> bool {
        // We need to allocate to get a pointer to store our clipboard
        // request so that it is stable until the read_clipboard callback
        // and call complete_clipboard_request. This sucks but clipboard
        // requests aren't high throughput so it's probably fine.
        let Some(cb) = self.app.opts.read_clipboard_cb else {
            return false;
        };
        let state_ptr = Box::into_raw(Box::new(state));
        // SAFETY: embedder callback; `state_ptr` stays valid until
        // `ghostty_surface_complete_clipboard_request` or we reclaim it.
        let started = unsafe {
            cb(
                self.userdata,
                c_int::from(clipboard as u8),
                state_ptr.cast::<c_void>(),
            )
        };
        if !started {
            // SAFETY: allocated above and not handed over.
            drop(unsafe { Box::from_raw(state_ptr) });
            return false;
        }
        true
    }

    fn set_clipboard(&self, clipboard: Clipboard, contents: &[(String, String)], confirm: bool) {
        let Some(cb) = self.app.opts.write_clipboard_cb else {
            return;
        };
        let owned: Vec<(CString, CString)> = contents
            .iter()
            .map(|(m, d)| {
                (
                    CString::new(m.replace('\0', "")).unwrap_or_default(),
                    CString::new(d.replace('\0', "")).unwrap_or_default(),
                )
            })
            .collect();
        let array: Vec<ghostty_clipboard_content_s> = owned
            .iter()
            .map(|(m, d)| ghostty_clipboard_content_s {
                mime: m.as_ptr(),
                data: d.as_ptr(),
            })
            .collect();
        // SAFETY: embedder callback; the array and strings outlive the call.
        unsafe {
            cb(
                self.userdata,
                c_int::from(clipboard as u8),
                array.as_ptr(),
                array.len(),
                confirm,
            );
        }
    }

    fn default_termio_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let mut env = ghostty_os::env::get_env_map();

        if cfg!(target_vendor = "apple") {
            if env.contains_key("__XCODE_BUILT_PRODUCTS_DIR_PATHS") {
                for k in [
                    "__XCODE_BUILT_PRODUCTS_DIR_PATHS",
                    "__XPC_DYLD_LIBRARY_PATH",
                    "DYLD_FRAMEWORK_PATH",
                    "DYLD_INSERT_LIBRARIES",
                    "DYLD_LIBRARY_PATH",
                    "LD_LIBRARY_PATH",
                    "SECURITYSESSIONID",
                    "XPC_SERVICE_NAME",
                ] {
                    env.remove(k);
                }
            }

            // Remove this so that running `ghostty` within Ghostty works.
            env.remove("GHOSTTY_MAC_LAUNCH_SOURCE");

            // If we were launched from the desktop then we want to remove
            // the LANGUAGE env var so that we don't inherit our translation
            // settings for Ghostty. If we aren't from the desktop then we
            // didn't set our LANGUAGE var so we don't need to remove it.
            if launched_from_desktop() {
                env.remove("LANGUAGE");
            }
        }

        env.into_iter().collect()
    }

    fn redraw_inspector(&self) {
        // `queueInspectorRender`
        self.app
            .perform_action(self.target(), Action::RenderInspector);
    }

    fn platform_view(&self) -> *mut c_void {
        self.view
    }

    #[cfg(feature = "test-platform")]
    fn gl_proc_address(&self, name: &str) -> *const c_void {
        test_platform::proc_address(name)
    }
}

#[cfg(target_os = "macos")]
fn launched_from_desktop() -> bool {
    ghostty_os::desktop::launched_from_desktop_lib()
}

#[cfg(not(target_os = "macos"))]
fn launched_from_desktop() -> bool {
    ghostty_os::desktop::launched_from_desktop()
}

/// The surface object behind `ghostty_surface_t`.
pub struct Surface {
    app: Rc<AppHandle>,
    rt: Rc<SurfaceRt>,
    core: OnceCell<Rc<CoreSurfaceImpl>>,
    inspector: RefCell<Option<Box<crate::inspector::Inspector>>>,
    /// Strings handed out by `ghostty_surface_inherited_config`; they stay
    /// valid until the surface is freed.
    inherited_wd: RefCell<Vec<CString>>,
}

impl Surface {
    fn core(&self) -> Option<&Rc<CoreSurfaceImpl>> {
        self.core.get()
    }

    pub(crate) fn inspector_slot(&self) -> &RefCell<Option<Box<crate::inspector::Inspector>>> {
        &self.inspector
    }

    pub(crate) fn core_surface(&self) -> Option<&Rc<CoreSurfaceImpl>> {
        self.core()
    }

    pub(crate) fn rt(&self) -> &Rc<SurfaceRt> {
        &self.rt
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // Shut down our inspector
        if let Some(mut i) = self.inspector.get_mut().take() {
            i.deinit(self.core.get().map(Rc::as_ref));
        }

        // Remove ourselves from the list of known surfaces in the app.
        self.app.core.delete_surface(self.rt.id);
        self.app.rt.surfaces.borrow_mut().remove(&self.rt.id);

        // Clean up our core surface so that all the rendering and IO stop.
        drop(self.core.take());
    }
}

/// Borrow the surface behind a handle.
///
/// # Safety
/// `p` must be null or a live handle from `ghostty_surface_new`.
pub(crate) unsafe fn surface<'a>(p: ghostty_surface_t) -> Option<&'a Surface> {
    // SAFETY: per the function contract.
    unsafe { p.cast::<Surface>().as_ref() }
}

/// `ghostty_surface_config_new`.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_surface_config_new() -> ghostty_surface_config_s {
    ghostty_surface_config_s {
        platform_tag: platform::INVALID,
        platform: crate::types::ghostty_platform_u {
            macos: crate::types::ghostty_platform_macos_s {
                nsview: std::ptr::null_mut(),
            },
        },
        userdata: std::ptr::null_mut(),
        scale_factor: 1.0,
        font_size: 0.0,
        working_directory: std::ptr::null(),
        command: std::ptr::null(),
        env_vars: std::ptr::null_mut(),
        env_var_count: 0,
        initial_input: std::ptr::null(),
        wait_after_command: false,
        context: 0,
    }
}

fn context_from_c(v: c_int) -> NewSurfaceContext {
    match v {
        1 => NewSurfaceContext::Tab,
        2 => NewSurfaceContext::Split,
        _ => NewSurfaceContext::Window,
    }
}

/// Test seam (feature `test-platform`): a fake platform whose OpenGL loader
/// the test installs, so surfaces can be created on Linux.
#[cfg(feature = "test-platform")]
pub mod test_platform {
    use std::{cell::RefCell, ffi::c_void, rc::Rc};

    /// The `platform_tag` that selects the fake platform.
    pub const PLATFORM_TAG: i32 = 1000;

    /// A GL function loader.
    pub type GlLoader = Rc<dyn Fn(&str) -> *const c_void>;

    thread_local! {
        static LOADER: RefCell<Option<GlLoader>> = const { RefCell::new(None) };
    }

    /// Install the GL loader for surfaces created on this thread.
    pub fn set_gl_loader(loader: Option<GlLoader>) {
        LOADER.with(|l| *l.borrow_mut() = loader);
    }

    pub(super) fn proc_address(name: &str) -> *const c_void {
        LOADER.with(|l| l.borrow().as_ref().map_or(std::ptr::null(), |f| f(name)))
    }
}

/// Initialize the platform view (Zig: `Platform.init`). Darwin only.
///
/// # Safety
/// The union member selected by `tag` must be initialized.
unsafe fn platform_view_from_c(
    opts: &ghostty_surface_config_s,
) -> Result<*mut c_void, &'static str> {
    #[cfg(feature = "test-platform")]
    if opts.platform_tag == test_platform::PLATFORM_TAG {
        // Any non-null "view" will do; the renderer is OpenGL here.
        return Ok(std::ptr::NonNull::<c_void>::dangling().as_ptr());
    }
    if !cfg!(target_vendor = "apple") {
        // The platform union has no members off Darwin.
        return Err("UnsupportedPlatform");
    }
    match opts.platform_tag {
        // SAFETY: the tag selects the initialized member.
        platform::MACOS => match unsafe { opts.platform.macos.nsview } {
            v if v.is_null() => Err("NSViewMustBeSet"),
            v => Ok(v),
        },
        // SAFETY: as above.
        platform::IOS => match unsafe { opts.platform.ios.uiview } {
            v if v.is_null() => Err("UIViewMustBeSet"),
            v => Ok(v),
        },
        _ => Err("InvalidEnumTag"),
    }
}

/// Zig's `std.zig.stringEscape` with non-ASCII code points written as
/// `\u{...}` (the config value is parsed as a Zig string; the bytes of
/// valid UTF-8 round trip either way).
fn string_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            ' ' | '!' | '#'..='&' | '\'' | '('..='[' | ']'..='~' => out.push(c),
            c if c.is_ascii() => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
        }
    }
    out
}

/// Apply the surface options to the config (the first half of Zig's
/// `Surface.init`).
///
/// # Safety
/// The pointers in `opts` must satisfy the contract of `ghostty.h`.
unsafe fn apply_surface_options(config: &mut Config, opts: &ghostty_surface_config_s) {
    // If we have a working directory from the options then we set it.
    // SAFETY: per the function contract.
    if let Some(wd) =
        unsafe { cstr(opts.working_directory) }.map(|c| c.to_string_lossy().into_owned())
        && !wd.is_empty()
    {
        match std::fs::metadata(&wd) {
            Err(err) => log::warn!("error opening requested working directory dir={wd} err={err}"),
            Ok(m) if !m.is_dir() => {
                log::warn!("requested working directory is not a directory dir={wd}");
            }
            Ok(_) => {
                let mut wd_val = WorkingDirectory::Path(wd);
                wd_val.finalize();
                config.working_directory = Some(wd_val);
            }
        }
    }

    // If we have a command from the options then we set it.
    // SAFETY: per the function contract.
    if let Some(cmd) = unsafe { cstr(opts.command) }.map(|c| c.to_string_lossy().into_owned())
        && !cmd.is_empty()
    {
        config.command = Some(Command::Shell(cmd));
        config.wait_after_command = true;
    }

    // Apply any environment variables that were requested.
    if opts.env_var_count > 0 && !opts.env_vars.is_null() {
        // SAFETY: the caller supplies `env_var_count` entries.
        let vars = unsafe { std::slice::from_raw_parts(opts.env_vars, opts.env_var_count) };
        for var in vars {
            // SAFETY: entries are NUL terminated strings.
            let (Some(k), Some(v)) = (unsafe { cstr(var.key) }, unsafe { cstr(var.value) }) else {
                continue;
            };
            config.env.put(&k.to_string_lossy(), &v.to_string_lossy());
        }
    }

    // If we have an initial input then we set it.
    // SAFETY: per the function contract.
    if let Some(input) = unsafe { cstr(opts.initial_input) } {
        // We need to escape the string because the "raw" field expects a
        // Zig string.
        config.input.list.clear();
        config
            .input
            .list
            .push(ReadableIO::Raw(string_escape(&input.to_string_lossy())));
    }

    // Wait after command
    if opts.wait_after_command {
        config.wait_after_command = true;
    }
}

/// Create a new surface as part of an app.
///
/// # Safety
/// `app_ptr` a live app handle; `opts` a valid config (see `ghostty.h`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_new(
    app_ptr: ghostty_app_t,
    opts: *const ghostty_surface_config_s,
) -> ghostty_surface_t {
    // SAFETY: per the function contract.
    let (Some(app_ref), Some(opts)) = (unsafe { app(app_ptr) }, unsafe { opts.as_ref() }) else {
        log::error!("error initializing surface err=NullArgument");
        return std::ptr::null_mut();
    };
    // SAFETY: per the function contract.
    let view = match unsafe { platform_view_from_c(opts) } {
        Ok(v) => v,
        Err(err) => {
            log::error!("error initializing surface err={err}");
            return std::ptr::null_mut();
        }
    };

    // Build the runtime surface. Its address is its id.
    let scale = opts.scale_factor as f32;
    let app_handle = {
        // `AppHandle` is owned by the C handle; the surface only needs
        // shared access to its parts, which are `Rc`s.
        Rc::new(AppHandle {
            core: Rc::clone(&app_ref.core),
            rt: Rc::clone(&app_ref.rt),
        })
    };
    let mut boxed = Box::new(Surface {
        app: app_handle,
        rt: Rc::new(SurfaceRt {
            app: Rc::clone(&app_ref.rt),
            id: 0,
            userdata: opts.userdata,
            view,
            content_scale: Cell::new(ContentScale { x: scale, y: scale }),
            size: Cell::new(SurfaceSize {
                width: 800,
                height: 600,
            }),
            cursor_pos: Cell::new(CursorPos { x: -1.0, y: -1.0 }),
            title: RefCell::new(None),
        }),
        core: OnceCell::new(),
        inspector: RefCell::new(None),
        inherited_wd: RefCell::new(Vec::new()),
    });
    let id = std::ptr::from_mut::<Surface>(&mut *boxed) as usize as SurfaceId;
    // The id is only known now (the address of the heap object).
    if let Some(rt) = Rc::get_mut(&mut boxed.rt) {
        rt.id = id;
    }
    app_ref
        .rt
        .surfaces
        .borrow_mut()
        .insert(id, Rc::downgrade(&boxed.rt));
    let ptr = Box::into_raw(boxed);

    // Shallow copy the config so that we can modify it.
    let mut config = {
        let app_config = app_ref.rt.config.borrow();
        let prev_pwd = app_ref.core.state().focused_surface().and_then(|s| s.pwd());
        new_config(&app_config, context_from_c(opts.context), prev_pwd)
    };
    // SAFETY: per the function contract.
    unsafe { apply_surface_options(&mut config, opts) };

    // Initialize our surface right away. We're given a view that is ready
    // to use.
    // SAFETY: `ptr` is the live allocation made above.
    let s = unsafe { &*ptr };
    let rt_app: Rc<dyn RtApp> = Rc::clone(&app_ref.rt) as Rc<dyn RtApp>;
    let rt_surface: Rc<dyn RtSurface> = Rc::clone(&s.rt) as Rc<dyn RtSurface>;
    match app_ref.core.create_surface(id, config, rt_app, rt_surface) {
        Ok(core) => {
            let _ = s.core.set(Rc::clone(&core));
            // If our options requested a specific font-size, set that.
            if opts.font_size != 0.0 {
                let mut font_size = core.font_size();
                font_size.points = opts.font_size;
                if let Err(err) = core.set_font_size(font_size) {
                    log::error!("error initializing surface err={err}");
                    // SAFETY: allocated by us and not exposed yet.
                    drop(unsafe { Box::from_raw(ptr) });
                    return std::ptr::null_mut();
                }
            }
            ptr.cast::<c_void>()
        }
        Err(err) => {
            log::error!("error initializing surface err={err}");
            // SAFETY: allocated by us and not exposed yet.
            drop(unsafe { Box::from_raw(ptr) });
            std::ptr::null_mut()
        }
    }
}

/// Free a surface.
///
/// # Safety
/// `ptr` must be a live surface handle; it is invalid afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_free(ptr: ghostty_surface_t) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: created by `ghostty_surface_new` via `Box::into_raw`.
    drop(unsafe { Box::from_raw(ptr.cast::<Surface>()) });
}

/// Returns the userdata associated with the surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_userdata(ptr: ghostty_surface_t) -> *mut c_void {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }.map_or(std::ptr::null_mut(), |s| s.rt.userdata)
}

/// Returns the app associated with a surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_app(ptr: ghostty_surface_t) -> ghostty_app_t {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }.map_or(std::ptr::null_mut(), |s| {
        s.rt.app.app_ptr.get() as ghostty_app_t
    })
}

/// Returns the config to use for surfaces that inherit from this one.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_inherited_config(
    ptr: ghostty_surface_t,
    source: c_int,
) -> ghostty_surface_config_s {
    let mut result = ghostty_surface_config_new();
    result.context = source;
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return result;
    };
    let Some(core) = s.core() else {
        return result;
    };
    let context = context_from_c(source);
    let config = s.rt.app.config.borrow();

    // The font size to inherit. If 0, default font size will be used.
    if config.window_inherit_font_size {
        result.font_size = core.font_size().points;
    }

    if should_inherit_working_directory(context, &config)
        && let Some(cwd) = core.pwd()
        && let Ok(c) = CString::new(cwd)
    {
        let mut keep = s.inherited_wd.borrow_mut();
        result.working_directory = c.as_ptr();
        keep.push(c);
    }
    result
}

/// Update the configuration to the provided config for only this surface.
///
/// # Safety
/// `ptr` a live surface handle, `config` a config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_update_config(
    ptr: ghostty_surface_t,
    config: ghostty_config_t,
) {
    // SAFETY: per the function contract.
    let (Some(s), Some(c)) = (unsafe { surface(ptr) }, unsafe { cconfig(config) }) else {
        return;
    };
    let Some(core) = s.core() else {
        return;
    };
    if let Err(err) = core.update_config(&Arc::new(c.config.clone())) {
        log::error!("error updating config err={err}");
    }
}

/// Returns true if the surface needs to confirm quitting.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_needs_confirm_quit(ptr: ghostty_surface_t) -> bool {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }
        .and_then(Surface::core)
        .is_some_and(|c| c.needs_confirm_quit())
}

/// Returns true if the surface process has exited.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_process_exited(ptr: ghostty_surface_t) -> bool {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }
        .and_then(Surface::core)
        .is_some_and(|c| CoreSurface::child_exited(c.as_ref()))
}

/// Returns true if the surface has a selection.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_has_selection(ptr: ghostty_surface_t) -> bool {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }
        .and_then(Surface::core)
        .is_some_and(|c| c.has_selection())
}

/// Fill the C text result from a read.
fn fill_text(
    core: &CoreSurfaceImpl,
    t: &Terminal,
    sel: Selection,
    result: &mut ghostty_text_s,
) -> bool {
    // Get our text directly from the core surface.
    let text: Text = match core.dump_text_locked(t, sel) {
        Ok(t) => t,
        Err(err) => {
            log::warn!("error reading text err={err}");
            return false;
        }
    };

    let (tl_px_x, tl_px_y, offset_start, offset_len) =
        text.viewport.map_or((-1.0, -1.0, 0, 0), |vp| {
            (vp.tl_px_x, vp.tl_px_y, vp.offset_start, vp.offset_len)
        });
    *result = ghostty_text_s {
        tl_px_x,
        tl_px_y,
        offset_start,
        offset_len,
        text: alloc_c_text(text.text.as_bytes()),
        text_len: text.text.len(),
    };
    true
}

/// Same as `ghostty_surface_read_text` but reads from the user selection, if
/// any.
///
/// # Safety
/// `ptr` a live surface handle, `result` a valid out pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_read_selection(
    ptr: ghostty_surface_t,
    result: *mut ghostty_text_s,
) -> bool {
    // SAFETY: per the function contract.
    let (Some(core), Some(result)) = (unsafe { surface(ptr) }.and_then(Surface::core), unsafe {
        result.as_mut()
    }) else {
        return false;
    };
    let guard = core.lock_terminal();
    let t: &Terminal = &guard;

    // If we don't have a selection, do nothing.
    let Some(sel) = t.screen().selection else {
        return false;
    };
    fill_text(core, t, sel, result)
}

/// The pin of a C point (Zig: `Point.pin`).
fn point_pin(p: &ghostty_point_s, t: &Terminal) -> Option<ghostty_terminal::page_list::Pin> {
    let tag = match p.tag {
        0 => PointTag::Active,
        1 => PointTag::Viewport,
        2 => PointTag::Screen,
        3 => PointTag::History,
        _ => return None,
    };
    let pages = &t.screen().pages;

    // Clamp our point to the screen bounds.
    let clamped_x = p.x.min(u32::from(pages.cols.saturating_sub(1)));
    let clamped_y = p.y.min(u32::from(pages.rows.saturating_sub(1)));

    match p.coord {
        // Exact coordinates require a specific pin.
        0 => {
            let x = u16::try_from(clamped_x).unwrap_or(u16::MAX);
            pages.pin(Point::new(tag, Coordinate { x, y: clamped_y }))
        }
        1 => pages.get_top_left(tag),
        2 => pages.get_bottom_right(tag),
        _ => None,
    }
}

fn selection_from_c(sel: &ghostty_selection_s, t: &Terminal) -> Option<Selection> {
    Some(Selection::new(
        point_pin(&sel.top_left, t)?,
        point_pin(&sel.bottom_right, t)?,
        sel.rectangle,
    ))
}

/// Read some arbitrary text from the surface.
///
/// This is an expensive operation so it shouldn't be called too often. We
/// recommend that callers cache the result and throttle calls to this
/// function.
///
/// # Safety
/// `ptr` a live surface handle, `result` a valid out pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_read_text(
    ptr: ghostty_surface_t,
    sel: ghostty_selection_s,
    result: *mut ghostty_text_s,
) -> bool {
    // SAFETY: per the function contract.
    let (Some(core), Some(result)) = (unsafe { surface(ptr) }.and_then(Surface::core), unsafe {
        result.as_mut()
    }) else {
        return false;
    };
    let guard = core.lock_terminal();
    let t: &Terminal = &guard;
    let Some(core_sel) = selection_from_c(&sel, t) else {
        return false;
    };
    fill_text(core, t, core_sel, result)
}

/// Free text read from the surface.
///
/// # Safety
/// `text` must be a result of one of the read functions (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_free_text(
    _ptr: ghostty_surface_t,
    text: *mut ghostty_text_s,
) {
    // SAFETY: per the function contract.
    let Some(text) = (unsafe { text.as_mut() }) else {
        return;
    };
    // SAFETY: allocated by `alloc_c_text` with `text_len` bytes + NUL.
    unsafe { free_c_text(text.text, text.text_len, true) };
    text.text = std::ptr::null();
    text.text_len = 0;
}

/// Tell the surface that it needs to schedule a render.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_refresh(ptr: ghostty_surface_t) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.refresh_callback()
    {
        log::error!("error in refresh callback err={err}");
    }
}

/// Tell the surface that it needs to schedule a render call as soon as
/// possible (NOW if possible).
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_draw(ptr: ghostty_surface_t) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.draw()
    {
        log::error!("error in draw err={err}");
    }
}

/// Update the size of a surface. This will trigger resize notifications to
/// the pty and the renderer.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_set_size(ptr: ghostty_surface_t, w: u32, h: u32) {
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return;
    };

    // Runtimes sometimes generate superfluous resize events even if the
    // size did not actually change (SwiftUI). We check that the size
    // actually changed from what we last recorded since resizes are
    // expensive.
    let cur = s.rt.size.get();
    if cur.width == w && cur.height == h {
        return;
    }
    let size = SurfaceSize {
        width: w,
        height: h,
    };
    s.rt.size.set(size);

    if let Some(core) = s.core()
        && let Err(err) = core.size_callback(size)
    {
        log::error!("error in size callback err={err}");
    }
}

/// Return the size information a surface has.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_size(ptr: ghostty_surface_t) -> ghostty_surface_size_s {
    let zero = ghostty_surface_size_s {
        columns: 0,
        rows: 0,
        width_px: 0,
        height_px: 0,
        cell_width_px: 0,
        cell_height_px: 0,
    };
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return zero;
    };
    let size = core.size();
    let grid = size.grid();
    ghostty_surface_size_s {
        columns: grid.columns,
        rows: grid.rows,
        width_px: size.screen.width,
        height_px: size.screen.height,
        cell_width_px: size.cell.width,
        cell_height_px: size.cell.height,
    }
}

/// Update the color scheme of the surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_set_color_scheme(
    ptr: ghostty_surface_t,
    scheme_raw: c_int,
) {
    let Some(scheme) = color_scheme_from_c(scheme_raw) else {
        log::warn!("invalid color scheme to ghostty_surface_set_color_scheme value={scheme_raw}");
        return;
    };
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.color_scheme_callback(scheme)
    {
        log::error!("error setting color scheme err={err}");
    }
}

/// Update the content scale of the surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_set_content_scale(ptr: ghostty_surface_t, x: f64, y: f64) {
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return;
    };

    // We are an embedded API so the caller can send us all sorts of
    // garbage. We want to make sure that the float values are valid and we
    // don't want to support fractional scaling below 1.
    let scaled = |v: f64| if v.is_nan() { 1.0 } else { v.max(1.0) } as f32;
    let scale = ContentScale {
        x: scaled(x),
        y: scaled(y),
    };
    s.rt.content_scale.set(scale);

    if let Some(core) = s.core()
        && let Err(err) = core.content_scale_callback(scale)
    {
        log::error!("error in content scale callback err={err}");
    }
}

/// Update the focused state of a surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_set_focus(ptr: ghostty_surface_t, focused: bool) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.focus_callback(focused)
    {
        log::error!("error in focus callback err={err}");
    }
}

/// Update the occlusion state of a surface.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_set_occlusion(ptr: ghostty_surface_t, visible: bool) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.occlusion_callback(visible)
    {
        log::error!("error in occlusion callback err={err}");
    }
}

/// Filter the mods if necessary. This handles settings such as
/// `macos-option-as-alt`. The filtered mods should be used for key
/// translation but should NOT be sent back via the `_key` function -- the
/// original mods should be used for that.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_key_translation_mods(
    ptr: ghostty_surface_t,
    mods_raw: c_int,
) -> c_int {
    let mods = mods_from_c(mods_raw);
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return mods_raw;
    };
    let Some(core) = s.core() else {
        return mods_raw;
    };
    let option_as_alt = core
        .config()
        .macos_option_as_alt
        .unwrap_or_else(|| s.rt.app.keyboard_layout().detect_option_as_alt());
    c_int::from(mods.translation(option_as_alt).int())
}

/// Send this for raw keypresses (i.e. the keyDown event on macOS). This will
/// handle the keymap translation and send the appropriate key and char
/// events.
///
/// # Safety
/// `ptr` a live surface handle; `event.text` null or NUL terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_key(
    ptr: ghostty_surface_t,
    event: ghostty_input_key_s,
) -> bool {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return false;
    };
    // SAFETY: per the function contract.
    let Some(key) = (unsafe { OwnedKey::from_c(&event) }) else {
        return false;
    };
    match CoreSurface::key_callback(core.as_ref(), key.event()) {
        Ok(ghostty_apprt::runtime_api::InputEffect::Closed) => true,
        Ok(ghostty_apprt::runtime_api::InputEffect::Ignored) => false,
        Ok(ghostty_apprt::runtime_api::InputEffect::Consumed) => true,
        Err(err) => {
            log::warn!("error processing key event err={err}");
            false
        }
    }
}

/// Returns true if the given key event would trigger a binding if it were
/// sent to the surface right now. The "right now" is important because
/// things like trigger sequences are only valid until the next key event.
///
/// # Safety
/// As [`ghostty_surface_key`]; `c_flags` null or a valid out pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_key_is_binding(
    ptr: ghostty_surface_t,
    event: ghostty_input_key_s,
    c_flags: *mut ghostty_binding_flags_e,
) -> bool {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return false;
    };
    // SAFETY: per the function contract.
    let Some(key) = (unsafe { OwnedKey::from_c(&event) }) else {
        log::warn!("error processing key event");
        return false;
    };
    let Some(flags) = core.key_event_is_binding(&key.event()) else {
        return false;
    };
    if !c_flags.is_null() {
        // Zig writes the `u8` `Flags.C` through the (enum-typed) pointer.
        // SAFETY: the pointer is valid for at least one byte.
        unsafe { c_flags.cast::<u8>().write(flags.cval()) };
    }
    true
}

/// Send raw text to the terminal. This is treated like a paste so this isn't
/// useful for sending escape sequences. For that, individual key input
/// should be used.
///
/// # Safety
/// `ptr` a live surface handle, `text` points to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_text(
    ptr: ghostty_surface_t,
    text: *const c_char,
    len: usize,
) {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return;
    };
    let bytes: &[u8] = if text.is_null() {
        &[]
    } else {
        // SAFETY: `text` points to `len` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) }
    };
    if let Err(err) = core.text_callback(&String::from_utf8_lossy(bytes)) {
        log::error!("error in key callback err={err}");
    }
}

/// Set the preedit text for the surface. This is used for IME composition.
/// If the length is 0, then the preedit text is cleared.
///
/// # Safety
/// `ptr` a live surface handle, `text` points to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_preedit(
    ptr: ghostty_surface_t,
    text: *const c_char,
    len: usize,
) {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return;
    };
    let preedit = if len == 0 || text.is_null() {
        None
    } else {
        // SAFETY: `text` points to `len` readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) };
        Some(String::from_utf8_lossy(bytes).into_owned())
    };
    if let Err(err) = core.preedit_callback(preedit.as_deref()) {
        log::error!("error in preedit callback err={err}");
    }
}

/// Returns true if the surface currently has mouse capturing enabled.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_mouse_captured(ptr: ghostty_surface_t) -> bool {
    // SAFETY: per the function contract.
    unsafe { surface(ptr) }
        .and_then(Surface::core)
        .is_some_and(|c| c.mouse_captured())
}

/// Send a mouse button event.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_mouse_button(
    ptr: ghostty_surface_t,
    action: c_int,
    button: c_int,
    mods: c_int,
) -> bool {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return false;
    };
    let state = match action {
        0 => MouseButtonState::Release,
        1 => MouseButtonState::Press,
        _ => return false,
    };
    let Some(button) = mouse_button_from_c(button) else {
        return false;
    };
    match core.mouse_button_callback(state, button, mods_from_c(mods)) {
        Ok(v) => v,
        Err(err) => {
            log::error!("error in mouse button callback err={err}");
            false
        }
    }
}

pub(crate) fn mouse_button_from_c(v: c_int) -> Option<MouseButton> {
    Some(match v {
        0 => MouseButton::Unknown,
        1 => MouseButton::Left,
        2 => MouseButton::Right,
        3 => MouseButton::Middle,
        4 => MouseButton::Four,
        5 => MouseButton::Five,
        6 => MouseButton::Six,
        7 => MouseButton::Seven,
        8 => MouseButton::Eight,
        9 => MouseButton::Nine,
        10 => MouseButton::Ten,
        11 => MouseButton::Eleven,
        _ => return None,
    })
}

/// Update the mouse position within the view.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_mouse_pos(
    ptr: ghostty_surface_t,
    x: f64,
    y: f64,
    mods: c_int,
) {
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return;
    };

    // The cursor position from the host directly is in screen coordinates
    // but all our interface works in pixels.
    let scale = s.rt.content_scale.get();
    let pos = CursorPos {
        x: x as f32 * scale.x,
        y: y as f32 * scale.y,
    };

    // There are cases where the platform reports a mouse motion event
    // without the cursor actually moving. For example, on macOS, updating
    // the window title can trigger a phantom mouse-move event at the same
    // coordinates. This can cause the mouse to incorrectly unhide when
    // mouse-hide-while-typing is enabled (commonly seen with TUI apps like
    // Zellij that frequently update the title). To prevent incorrect
    // behavior, we only continue with callback logic if the cursor has
    // actually moved.
    let prev = s.rt.cursor_pos.get();
    if (prev.x - pos.x).abs() < 1.0 && (prev.y - pos.y).abs() < 1.0 {
        return;
    }
    s.rt.cursor_pos.set(pos);

    if let Some(core) = s.core()
        && let Err(err) = core.cursor_pos_callback(pos, Some(mods_from_c(mods)))
    {
        log::error!("error in cursor pos callback err={err}");
    }
}

/// Send a scroll event.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_mouse_scroll(
    ptr: ghostty_surface_t,
    x: f64,
    y: f64,
    scroll_mods: c_int,
) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core)
        && let Err(err) = core.scroll_callback(x, y, ScrollMods::from_int(scroll_mods as u32 as u8))
    {
        log::error!("error in scroll callback err={err}");
    }
}

/// Send a mouse pressure event.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_mouse_pressure(
    ptr: ghostty_surface_t,
    stage_raw: u32,
    pressure: f64,
) {
    let stage = match stage_raw {
        0 => MousePressureStage::None,
        1 => MousePressureStage::Normal,
        2 => MousePressureStage::Deep,
        _ => {
            log::warn!("invalid mouse pressure stage value={stage_raw}");
            return;
        }
    };
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core) {
        CoreSurface::mouse_pressure_callback(core.as_ref(), stage, pressure);
    }
}

/// The position of the IME candidate window.
///
/// # Safety
/// `ptr` a live surface handle; the out pointers valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_ime_point(
    ptr: ghostty_surface_t,
    x: *mut f64,
    y: *mut f64,
    width: *mut f64,
    height: *mut f64,
) {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return;
    };
    let pos = core.ime_point();
    // SAFETY: the out pointers are valid per the contract.
    unsafe {
        if let Some(x) = x.as_mut() {
            *x = pos.x;
        }
        if let Some(y) = y.as_mut() {
            *y = pos.y;
        }
        if let Some(w) = width.as_mut() {
            *w = pos.width;
        }
        if let Some(h) = height.as_mut() {
            *h = pos.height;
        }
    }
}

/// Request that the surface become closed. This will go through the normal
/// trigger process that a close surface input binding would.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_request_close(ptr: ghostty_surface_t) {
    // SAFETY: per the function contract.
    if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core) {
        core.close();
    }
}

/// Request that the surface split in the given direction.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_split(ptr: ghostty_surface_t, direction: c_int) {
    let direction = match direction {
        0 => SplitDirection::Right,
        1 => SplitDirection::Down,
        2 => SplitDirection::Left,
        3 => SplitDirection::Up,
        _ => return,
    };
    // SAFETY: per the function contract.
    if let Some(s) = unsafe { surface(ptr) } {
        s.rt.app
            .perform_action(s.rt.target(), Action::NewSplit(direction));
    }
}

/// Focus on the next split (if any).
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_split_focus(ptr: ghostty_surface_t, direction: c_int) {
    let direction = match direction {
        0 => GotoSplit::Previous,
        1 => GotoSplit::Next,
        2 => GotoSplit::Up,
        3 => GotoSplit::Left,
        4 => GotoSplit::Down,
        5 => GotoSplit::Right,
        _ => return,
    };
    // SAFETY: per the function contract.
    if let Some(s) = unsafe { surface(ptr) } {
        s.rt.app
            .perform_action(s.rt.target(), Action::GotoSplit(direction));
    }
}

/// Resize the current split by moving the split divider in the given
/// direction. `direction` specifies which direction the split divider will
/// move relative to the focused split. `amount` is a fractional value
/// between 0 and 1 that specifies by how much the divider will move.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_split_resize(
    ptr: ghostty_surface_t,
    direction: c_int,
    amount: u16,
) {
    let direction = match direction {
        0 => ResizeSplitDirection::Up,
        1 => ResizeSplitDirection::Down,
        2 => ResizeSplitDirection::Left,
        3 => ResizeSplitDirection::Right,
        _ => return,
    };
    // SAFETY: per the function contract.
    if let Some(s) = unsafe { surface(ptr) } {
        s.rt.app.perform_action(
            s.rt.target(),
            Action::ResizeSplit(ResizeSplit { amount, direction }),
        );
    }
}

/// Equalize the size of all splits in the current window.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_split_equalize(ptr: ghostty_surface_t) {
    // SAFETY: per the function contract.
    if let Some(s) = unsafe { surface(ptr) } {
        s.rt.app
            .perform_action(s.rt.target(), Action::EqualizeSplits);
    }
}

/// Invoke an action on the surface.
///
/// # Safety
/// `ptr` a live surface handle, `action` points to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_binding_action(
    ptr: ghostty_surface_t,
    action: *const c_char,
    len: usize,
) -> bool {
    // SAFETY: per the function contract.
    let Some(core) = (unsafe { surface(ptr) }).and_then(Surface::core) else {
        return false;
    };
    if action.is_null() {
        return false;
    }
    // SAFETY: `action` points to `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(action.cast::<u8>(), len) };
    let action_str = String::from_utf8_lossy(bytes);
    let action = match BindingAction::parse(&action_str) {
        Ok(a) => a,
        Err(err) => {
            log::error!("error parsing binding action action={action_str} err={err:?}");
            return false;
        }
    };
    match core.perform_binding_action(&action) {
        Ok(v) => v,
        Err(err) => {
            log::error!("error performing binding action action={action} err={err}");
            false
        }
    }
}

/// Complete a clipboard read request started via the read callback. This can
/// only be called once for a given request. Once it is called with a request
/// the request pointer will be invalidated.
///
/// # Safety
/// `ptr` a live surface handle; `text` NUL terminated; `state` a pointer
/// handed to the `read_clipboard` / `confirm_read_clipboard` callbacks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_complete_clipboard_request(
    ptr: ghostty_surface_t,
    text: *const c_char,
    state: *mut c_void,
    confirmed: bool,
) {
    // SAFETY: per the function contract.
    let (Some(s), Some(str)) = (unsafe { surface(ptr) }, unsafe { cstr(text) }) else {
        return;
    };
    if state.is_null() {
        return;
    }
    let Some(core) = s.core() else {
        return;
    };
    let str = str.to_string_lossy();
    // SAFETY: the pointer is a `Box<ClipboardRequest>` leaked by
    // `clipboard_request`; it stays alive when confirmation is requested.
    let req = unsafe { *state.cast::<ClipboardRequest>() };

    // Attempt to complete the request, but we may request confirmation.
    match core.complete_clipboard_request(req, &str, confirmed) {
        Err(
            ghostty_apprt::runtime_api::ClipboardError::UnsafePaste
            | ghostty_apprt::runtime_api::ClipboardError::UnauthorizedPaste,
        ) => {
            if let Some(cb) = s.rt.app.opts.confirm_read_clipboard_cb {
                let ty = match req.request_type() {
                    ghostty_apprt::structs::ClipboardRequestType::Paste => 0,
                    ghostty_apprt::structs::ClipboardRequestType::Osc52Read => 1,
                    ghostty_apprt::structs::ClipboardRequestType::Osc52Write => 2,
                };
                let c = CString::new(str.replace('\0', "")).unwrap_or_default();
                // SAFETY: embedder callback; `state` is preserved for the
                // confirmation route.
                unsafe { cb(s.rt.userdata, c.as_ptr(), state, ty) };
            }
            return;
        }
        Err(err) => log::error!("error completing clipboard request err={err}"),
        Ok(()) => {}
    }

    // We don't defer this because the clipboard confirmation route
    // preserves the clipboard request.
    // SAFETY: allocated by `clipboard_request`, consumed exactly once here.
    drop(unsafe { Box::from_raw(state.cast::<ClipboardRequest>()) });
}

/// Sets the window background blur on macOS to the desired value. I do this
/// as an extern function because I don't know how to call these functions
/// in Swift.
///
/// This uses an undocumented, non-public API because this is what every
/// terminal appears to use, including Terminal.app.
///
/// # Safety
/// `app` a live app handle; `window` an `NSWindow` (macOS).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_set_window_background_blur(
    app_ptr: ghostty_app_t,
    window: *mut c_void,
) {
    // This is only supported on macOS
    #[cfg(target_os = "macos")]
    {
        use objc2::{msg_send, runtime::AnyObject};

        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGSSetWindowBackgroundBlurRadius(
                connection: *mut c_void,
                window: usize,
                radius: c_int,
            ) -> i32;
            fn CGSDefaultConnectionForThread() -> *mut c_void;
        }

        // SAFETY: per the function contract.
        let Some(a) = (unsafe { app(app_ptr) }) else {
            return;
        };
        if window.is_null() {
            return;
        }
        let config = a.rt.config.borrow();

        // Do nothing if we don't have background transparency enabled
        if config.background_opacity >= 1.0 {
            return;
        }
        let radius = match config.c_value("background-blur") {
            Some(ghostty_config::c_value::CValue::Short(v)) => c_int::from(v),
            _ => 0,
        };

        // SAFETY: `window` is an NSWindow per the contract; the CGS
        // functions are the private window server API Terminal.app uses.
        unsafe {
            let number: isize = msg_send![window.cast::<AnyObject>(), windowNumber];
            CGSSetWindowBackgroundBlurRadius(
                CGSDefaultConnectionForThread(),
                number as usize,
                radius,
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_ptr, window);
    }
}

// ---------------------------------------------------------------------
// Darwin only exports

#[cfg(target_vendor = "apple")]
#[allow(clippy::wildcard_imports)]
mod darwin {
    use super::*;
    use crate::types::ghostty_text_s;

    /// Set the display the surface is on (for the display link).
    ///
    /// # Safety
    /// `ptr` must be a live surface handle.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_surface_set_display_id(
        ptr: ghostty_surface_t,
        display_id: u32,
    ) {
        // SAFETY: per the function contract.
        if let Some(core) = unsafe { surface(ptr) }.and_then(Surface::core) {
            core.set_macos_display_id(display_id);
        }
    }

    /// This returns a CTFontRef that should be used for quicklook
    /// highlighted text. This is always the primary font in use regardless
    /// of the selected text.
    ///
    /// # Safety
    /// `ptr` must be a live surface handle.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_surface_quicklook_font(ptr: ghostty_surface_t) -> *mut c_void {
        // SAFETY: per the function contract.
        let Some(s) = (unsafe { surface(ptr) }) else {
            return std::ptr::null_mut();
        };
        let Some(core) = s.core() else {
            return std::ptr::null_mut();
        };
        // We need to unscale the content scale. We apply the content scale
        // to our font stack because we are rendering at 1x but callers of
        // this should be using scaled or apply scale themselves.
        let scale = s.rt.get_content_scale();
        core.quicklook_font(scale.y).unwrap_or(std::ptr::null_mut())
    }

    /// This returns the selected word for quicklook. This will populate the
    /// buffer with the word under the cursor and the selection info so that
    /// quicklook can be rendered.
    ///
    /// This does not modify the selection active on the surface (if any).
    ///
    /// # Safety
    /// `ptr` a live surface handle, `result` a valid out pointer.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_surface_quicklook_word(
        ptr: ghostty_surface_t,
        result: *mut ghostty_text_s,
    ) -> bool {
        // SAFETY: per the function contract.
        let (Some(s), Some(result)) = (unsafe { surface(ptr) }, unsafe { result.as_mut() }) else {
            return false;
        };
        let Some(core) = s.core() else {
            return false;
        };
        let guard = core.lock_terminal();
        let t: &Terminal = &guard;

        // Get our word selection
        let pos = s.rt.get_cursor_pos();
        let vp = core.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));
        let Some(pin) = t
            .screen()
            .pages
            .pin(Point::Viewport(Coordinate { x: vp.x, y: vp.y }))
        else {
            return false;
        };
        let Some(sel) = t
            .screen()
            .select_word(pin, &core.cfg().selection_word_chars)
        else {
            return false;
        };

        // Read the selection
        fill_text(core, t, sel, result)
    }
}
