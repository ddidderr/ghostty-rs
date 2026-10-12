//! The inspector C API (`ghostty_surface_inspector`, `ghostty_inspector_*`;
//! the `Inspector` struct of ZIG/src/apprt/embedded.zig).
//!
//! The embedded inspector owns a Dear ImGui context, feeds it the host's
//! input events and, on Apple platforms, draws it with Metal into the
//! command buffer the host provides (`ghostty_inspector_metal_*`).
#![allow(unsafe_code)]

#[cfg(target_vendor = "apple")]
use std::time::Instant;
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    rc::Rc,
};

use ghostty_apprt::runtime_api::RtSurface;
use ghostty_core::Surface as CoreSurfaceImpl;
use ghostty_input::{Action as KeyAction, Key, MouseButtonState, ScrollMods};
use ghostty_inspector::imgui::{Context, Key as ImguiKey, MouseButton as ImguiMouseButton};

use crate::{
    embedded::{SurfaceRt, mouse_button_from_c, surface},
    types::{ghostty_inspector_t, ghostty_surface_t},
};

/// The inspector state (Zig: `embedded.Inspector`). 1:1 with a surface.
pub struct Inspector {
    surface: Rc<SurfaceRt>,
    core: Rc<CoreSurfaceImpl>,
    ctx: Context,
    #[cfg(target_vendor = "apple")]
    backend: Option<crate::inspector_metal::MetalBackend>,
    content_scale: f64,
    /// The scale the ImGui style currently has (a `Style` cannot be rebuilt
    /// from Rust like Zig's `ImGuiStyle_ImGuiStyle`, so it is scaled by the
    /// change of the factor).
    style_scale: f32,
    /// Our previous instant used to calculate delta time for animations.
    #[cfg(target_vendor = "apple")]
    instant: Option<Instant>,
}

impl Inspector {
    fn init(surface: Rc<SurfaceRt>, core: Rc<CoreSurfaceImpl>) -> Option<Inspector> {
        let mut ctx = match Context::try_create() {
            Ok(c) => c,
            Err(err) => {
                log::error!("error initializing inspector err={err}");
                return None;
            }
        };
        if let Err(err) = ctx.set_platform_name(Some("ghostty_embedded")) {
            log::warn!("unable to set the imgui platform name err={err}");
        }

        // Setup our core inspector
        ghostty_inspector::setup(&mut ctx);
        if let Err(err) = core.activate_inspector() {
            log::error!("failed to activate inspector err={err}");
        }

        Some(Inspector {
            surface,
            core,
            ctx,
            #[cfg(target_vendor = "apple")]
            backend: None,
            content_scale: 1.0,
            style_scale: 1.0,
            #[cfg(target_vendor = "apple")]
            instant: None,
        })
    }

    /// Stop collecting data and release the backend (Zig: `deinit`).
    pub(crate) fn deinit(&mut self, _core: Option<&CoreSurfaceImpl>) {
        self.core.deactivate_inspector();
        #[cfg(target_vendor = "apple")]
        if let Some(b) = self.backend.take() {
            b.shutdown();
        }
    }

    /// Queue a render for the next frame.
    fn queue_render(&self) {
        RtSurface::redraw_inspector(self.surface.as_ref());
    }

    fn update_content_scale(&mut self, x: f64) {
        // Cache our scale because we use it for cursor position
        // calculations.
        self.content_scale = x;

        // Scale the style appropriately.
        let scale = x as f32;
        let last = std::mem::replace(&mut self.style_scale, scale);
        if (scale - last).abs() > f32::EPSILON && last > 0.0 {
            self.ctx.style_mut().scale_all_sizes(scale / last);
        }
    }

    fn update_size(&mut self, width: u32, height: u32) {
        self.ctx
            .io_mut()
            .set_display_size([width as f32, height as f32]);
    }

    fn mouse_button(&mut self, action: MouseButtonState, button: ghostty_input::MouseButton) {
        self.queue_render();
        let imgui_button = match button {
            ghostty_input::MouseButton::Left => ImguiMouseButton::Left,
            ghostty_input::MouseButton::Middle => ImguiMouseButton::Middle,
            ghostty_input::MouseButton::Right => ImguiMouseButton::Right,
            _ => return, // unsupported
        };
        self.ctx
            .io_mut()
            .add_mouse_button_event(imgui_button, action == MouseButtonState::Press);
    }

    fn scroll(&mut self, xoff: f64, yoff: f64, mods: ScrollMods) {
        self.queue_render();

        // For precision scrolling (trackpads), the values are in pixels
        // which scroll way too fast. Scale them down to approximate
        // discrete wheel notches. imgui expects 1.0 to scroll ~5 lines of
        // text.
        let scale = if mods.precision { 0.1 } else { 1.0 };
        self.ctx
            .io_mut()
            .add_mouse_wheel_event([(xoff * scale) as f32, (yoff * scale) as f32]);
    }

    fn cursor_pos(&mut self, x: f64, y: f64) {
        self.queue_render();
        self.ctx.io_mut().add_mouse_pos_event([
            (x * self.content_scale) as f32,
            (y * self.content_scale) as f32,
        ]);
    }

    fn focus(&mut self, focused: bool) {
        self.queue_render();
        self.ctx.io_mut().add_focus_event(focused);
    }

    fn text(&mut self, text: &str) {
        self.queue_render();
        self.ctx.io_mut().add_input_characters_utf8(text);
    }

    fn key(&mut self, action: KeyAction, key: Key, mods: ghostty_input::Mods) {
        self.queue_render();
        let io = self.ctx.io_mut();

        // Update all our modifiers
        io.add_key_event(ImguiKey::LeftShift, mods.shift);
        io.add_key_event(ImguiKey::LeftCtrl, mods.ctrl);
        io.add_key_event(ImguiKey::LeftAlt, mods.alt);
        io.add_key_event(ImguiKey::LeftSuper, mods.super_);

        // Send our keypress
        if let Some(imgui_key) = ghostty_inspector::keys::imgui_key(key) {
            io.add_key_event(
                imgui_key,
                matches!(action, KeyAction::Press | KeyAction::Repeat),
            );
        }
    }

    /// Update the delta time (Zig: `newFrame`).
    #[cfg(target_vendor = "apple")]
    pub(crate) fn new_frame(&mut self) {
        let now = Instant::now();
        let delta = self.instant.map_or(1.0 / 60.0, |prev| {
            now.duration_since(prev).as_secs_f32().max(0.00001)
        });
        self.instant = Some(now);
        self.ctx.io_mut().set_delta_time(delta);
    }

    /// Build one UI frame, calling the core inspector (`inspector.render`).
    /// `end` finishes the frame without producing draw data.
    #[cfg(target_vendor = "apple")]
    pub(crate) fn build_frame(&mut self) -> &mut ghostty_inspector::imgui::Ui {
        self.new_frame();
        let core = Rc::clone(&self.core);
        let ui = self.ctx.frame();
        core.render_inspector(ui);
        ui
    }

    #[cfg(target_vendor = "apple")]
    pub(crate) fn context(&mut self) -> &mut Context {
        &mut self.ctx
    }

    #[cfg(target_vendor = "apple")]
    pub(crate) fn take_backend(&mut self) -> Option<crate::inspector_metal::MetalBackend> {
        self.backend.take()
    }

    #[cfg(target_vendor = "apple")]
    pub(crate) fn set_backend(&mut self, backend: crate::inspector_metal::MetalBackend) {
        self.backend = Some(backend);
    }
}

/// Borrow the inspector behind a handle.
///
/// # Safety
/// `p` must be null or a handle from `ghostty_surface_inspector` whose
/// surface is still alive and whose inspector wasn't freed.
unsafe fn inspector<'a>(p: ghostty_inspector_t) -> Option<&'a mut Inspector> {
    // SAFETY: per the function contract.
    unsafe { p.cast::<Inspector>().as_mut() }
}

/// Initialize the inspector instance. A surface can only have one inspector
/// at any given time, so this will return the previous inspector if it was
/// already initialized.
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_surface_inspector(ptr: ghostty_surface_t) -> ghostty_inspector_t {
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return std::ptr::null_mut();
    };
    let slot = s.inspector_slot();
    if let Some(i) = slot.borrow_mut().as_mut() {
        return std::ptr::from_mut::<Inspector>(&mut **i).cast::<c_void>();
    }
    let Some(core) = s.core_surface() else {
        return std::ptr::null_mut();
    };
    let Some(i) = Inspector::init(Rc::clone(s.rt()), Rc::clone(core)) else {
        return std::ptr::null_mut();
    };
    let mut boxed = Box::new(i);
    let handle = std::ptr::from_mut::<Inspector>(&mut *boxed).cast::<c_void>();
    *slot.borrow_mut() = Some(boxed);
    handle
}

/// Free the inspector of a surface (note: takes the surface).
///
/// # Safety
/// `ptr` must be a live surface handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_free(ptr: ghostty_surface_t) {
    // SAFETY: per the function contract.
    let Some(s) = (unsafe { surface(ptr) }) else {
        return;
    };
    let taken = s.inspector_slot().borrow_mut().take();
    if let Some(mut i) = taken {
        i.deinit(None);
    }
}

/// Set the size of the inspector view in pixels.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_set_size(ptr: ghostty_inspector_t, w: u32, h: u32) {
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.update_size(w, h);
    }
}

/// Set the content scale of the inspector view.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_set_content_scale(
    ptr: ghostty_inspector_t,
    x: f64,
    _y: f64,
) {
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.update_content_scale(x);
    }
}

/// Send a mouse button event to the inspector.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_mouse_button(
    ptr: ghostty_inspector_t,
    action: c_int,
    button: c_int,
    _mods: c_int,
) {
    let state = match action {
        0 => MouseButtonState::Release,
        1 => MouseButtonState::Press,
        _ => return,
    };
    let Some(button) = mouse_button_from_c(button) else {
        return;
    };
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.mouse_button(state, button);
    }
}

/// Send a mouse position to the inspector.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_mouse_pos(ptr: ghostty_inspector_t, x: f64, y: f64) {
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.cursor_pos(x, y);
    }
}

/// Send a scroll event to the inspector.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_mouse_scroll(
    ptr: ghostty_inspector_t,
    x: f64,
    y: f64,
    scroll_mods: c_int,
) {
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.scroll(x, y, ScrollMods::from_int(scroll_mods as u32 as u8));
    }
}

/// Send a key event to the inspector.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_key(
    ptr: ghostty_inspector_t,
    action: c_int,
    key: c_int,
    c_mods: c_int,
) {
    let action = match action {
        0 => KeyAction::Release,
        1 => KeyAction::Press,
        2 => KeyAction::Repeat,
        _ => return,
    };
    let Some(key) = usize::try_from(key)
        .ok()
        .and_then(|k| Key::ALL.get(k))
        .copied()
    else {
        return;
    };
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.key(
            action,
            key,
            ghostty_input::Mods::from_int(c_mods as u32 as u16),
        );
    }
}

/// Send text input to the inspector.
///
/// # Safety
/// `ptr` a live inspector handle; `text` NUL terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_text(ptr: ghostty_inspector_t, text: *const c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: per the function contract.
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.text(&text);
    }
}

/// Set the focus state of the inspector.
///
/// # Safety
/// `ptr` must be a live inspector handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_inspector_set_focus(ptr: ghostty_inspector_t, focused: bool) {
    // SAFETY: per the function contract.
    if let Some(i) = unsafe { inspector(ptr) } {
        i.focus(focused);
    }
}

#[cfg(target_vendor = "apple")]
#[allow(clippy::wildcard_imports)]
mod metal_exports {
    use super::*;

    /// Initialize the inspector for a Metal backend. Takes ownership of one
    /// reference to `device`.
    ///
    /// # Safety
    /// `ptr` a live inspector handle; `device` a retained `MTLDevice`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_inspector_metal_init(
        ptr: ghostty_inspector_t,
        device: *mut c_void,
    ) -> bool {
        // SAFETY: per the function contract.
        let Some(i) = (unsafe { inspector(ptr) }) else {
            return false;
        };
        if let Some(b) = i.backend.take() {
            b.shutdown();
        }
        // SAFETY: `device` is a retained MTLDevice whose reference we own.
        match unsafe { crate::inspector_metal::MetalBackend::init(device, i.context()) } {
            Ok(b) => {
                i.backend = Some(b);
                log::debug!("initialized metal backend");
                true
            }
            Err(err) => {
                log::warn!("failed to initialize metal backend err={err}");
                false
            }
        }
    }

    /// Render the inspector into the host's command buffer. Takes ownership
    /// of one reference to each of `command_buffer` and `descriptor`.
    ///
    /// # Safety
    /// `ptr` a live inspector handle; the others retained Metal objects.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_inspector_metal_render(
        ptr: ghostty_inspector_t,
        command_buffer: *mut c_void,
        descriptor: *mut c_void,
    ) {
        // SAFETY: per the function contract.
        let Some(i) = (unsafe { inspector(ptr) }) else {
            return;
        };
        // SAFETY: retained objects whose reference we own.
        unsafe { crate::inspector_metal::render(i, command_buffer, descriptor) };
    }

    /// Shut the Metal backend down.
    ///
    /// # Safety
    /// `ptr` must be a live inspector handle.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ghostty_inspector_metal_shutdown(ptr: ghostty_inspector_t) {
        // SAFETY: per the function contract.
        if let Some(i) = unsafe { inspector(ptr) }
            && let Some(b) = i.backend.take()
        {
            b.shutdown();
        }
    }
}
