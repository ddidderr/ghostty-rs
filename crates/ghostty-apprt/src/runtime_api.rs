//! The trait boundary between app runtimes (apprt-gtk) and `ghostty-core`.
//!
//! In Zig, `apprt/gtk` calls directly into `CoreApp` (App.zig) and
//! `CoreSurface` (Surface.zig). `ghostty-core` isn't ported yet (and it can't
//! be a dependency of this crate without a cycle once core implements these
//! traits), so the exact surface of what GTK needs is expressed here:
//!
//! * [`CoreApp`] / [`CoreSurface`]: implemented by ghostty-core, called by GTK.
//! * [`RtApp`] / [`RtSurface`]: implemented by this crate, handed to core so it
//!   can call back (Zig: `rt_app.performAction`, `rt_surface.getSize()`, ...).
//!
//! Threading: everything here is **main-thread only** (`!Send`), like the GTK
//! objects behind it. The only cross-thread entry point is [`Waker`]
//! (`App.wakeup`), used by core threads to request an app `tick()`.
//!
//! See `docs/rewrite/notes/ghostty-apprt-gtk.md` for the full spec.

use std::{rc::Rc, sync::Arc};

use ghostty_config::config::Config;
use ghostty_input::{Mods, MouseButton, MouseButtonState, ScrollMods};

use crate::{
    action::{Action, Target},
    structs::{
        Clipboard,
        ClipboardRequest,
        ColorScheme,
        ContentScale,
        CursorPos,
        ImePos,
        SurfaceSize,
    },
};

/// Identifier for a core surface (Zig: `*CoreSurface` pointer). Allocated by
/// this crate for each GTK surface; core stores it in its `Target::Surface`.
pub type SurfaceId = u64;

/// Minimum window size in cells (Zig: `CoreSurface.min_window_{width,height}_cells`).
pub const MIN_WINDOW_WIDTH_CELLS: u32 = 10;
/// See [`MIN_WINDOW_WIDTH_CELLS`].
pub const MIN_WINDOW_HEIGHT_CELLS: u32 = 4;

/// Result of `CoreSurface::key_callback` (Zig: `Surface.InputEffect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEffect {
    /// The key was consumed by a binding that closed the surface; the
    /// surface may no longer be valid.
    Closed,
    /// The key wasn't used.
    Ignored,
    /// The key was used (binding performed or sent to the pty).
    Consumed,
}

/// Errors from `CoreSurface::complete_clipboard_request` that the GTK side
/// reacts to (Zig error set: `UnsafePaste`, `UnauthorizedPaste`).
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("unsafe paste")]
    UnsafePaste,
    #[error("unauthorized paste")]
    UnauthorizedPaste,
    #[error("{0}")]
    Other(String),
}

/// Generic core error (Zig: `anyerror`).
pub type CoreError = Box<dyn std::error::Error + Send + Sync>;

/// A message sent by the GTK side to the core app mailbox
/// (Zig: `core_app.mailbox.push`).
#[derive(Debug, Clone)]
pub enum AppMessage {
    /// Open the config file in the default editor (`App.Message.open_config`).
    OpenConfig,
    /// `App.Message.new_window` (queued by `activate`; core responds with
    /// `rt.perform_action(Target::App, Action::NewWindow)`).
    NewWindow,
    /// `surface_message{ surface, .present_surface }`; core validates the id.
    PresentSurface(SurfaceId),
}

/// Desired font size request used when initializing a surface (Zig:
/// `font.face.DesiredSize` points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesiredFontSize {
    pub points: f32,
    pub xdpi: u16,
    pub ydpi: u16,
}

/// Input to `CoreSurface::key_callback`: `ghostty_input::KeyEvent` is used
/// directly.
pub use ghostty_input::KeyEvent;

/// Cell metrics for a config+DPI combo (Zig: `font_grid_set.ref(..)` ->
/// `grid.cellSize()`), used to compute the default window size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

/// Implemented by ghostty-core's `App`.
pub trait CoreApp {
    /// `App.tick`: drain the app mailbox and perform pending work. Core calls
    /// back into `rt` (e.g. `rt.perform_action`). Called on the main thread
    /// whenever a [`Waker`] wake was requested, and after config updates.
    fn tick(&self, rt: &dyn RtApp) -> Result<(), CoreError>;

    /// `App.updateConfig`: apply a new config to the app and all surfaces
    /// (surfaces get `Message::ChangeConfig`). Returns Ok on success.
    fn update_config(&self, rt: &dyn RtApp, config: &Arc<Config>) -> Result<(), CoreError>;

    /// `App.needsConfirmQuit`: any surface with a running process?
    fn needs_confirm_quit(&self) -> bool;

    /// `App.colorSchemeEvent`: the system color scheme changed.
    fn color_scheme_event(&self, rt: &dyn RtApp, scheme: ColorScheme);

    /// `App.performAllAction`: perform `action` on every surface
    /// (`rt.perform_action(Target::Surface(id), ..)` for each).
    fn perform_all_action(
        &self,
        rt: &dyn RtApp,
        action: &ghostty_input::binding::Action,
    ) -> Result<(), CoreError>;

    /// `App.mailbox.push(.., .forever)`.
    fn push_message(&self, msg: AppMessage);

    /// `App.addSurface` + `Surface.init`: create a core surface for a GTK
    /// surface. `config` is the (possibly modified) surface config. The
    /// returned surface is owned by the GTK surface. `rt_surface` is the
    /// back-reference core uses (Zig: `&priv.rt_surface`).
    fn new_surface(
        &self,
        id: SurfaceId,
        config: Config,
        rt_app: Rc<dyn RtApp>,
        rt_surface: Rc<dyn RtSurface>,
    ) -> Result<Rc<dyn CoreSurface>, CoreError>;

    /// `App.deleteSurface`: called when the GTK surface is finalized, after
    /// the `CoreSurface` was dropped.
    fn delete_surface(&self, id: SurfaceId);

    /// Ids of all live surfaces (`core_app.surfaces.items`).
    fn surface_ids(&self) -> Vec<SurfaceId>;

    /// Cell size for the given config and DPI, used for the default window
    /// size (`font_grid_set.ref(..).cellSize()`); None if fonts fail.
    fn cell_size(&self, config: &Config, font_size: DesiredFontSize) -> Option<CellSize>;
}

/// Implemented by ghostty-core's `Surface`. All methods take `&self`
/// (interior mutability inside core), main thread only.
pub trait CoreSurface {
    /// The config the surface was created with / last updated to.
    fn config(&self) -> Arc<Config>;

    // ---- input -------------------------------------------------------
    fn key_callback(&self, event: KeyEvent<'_>) -> Result<InputEffect, CoreError>;
    /// `preeditCallback(null)` clears.
    fn preedit_callback(&self, text: Option<&str>) -> Result<(), CoreError>;
    fn focus_callback(&self, focused: bool) -> Result<(), CoreError>;
    /// Returns true if the click was consumed.
    fn mouse_button_callback(
        &self,
        state: MouseButtonState,
        button: MouseButton,
        mods: Mods,
    ) -> Result<bool, CoreError>;
    /// `pos` is in surface (logical*scale) pixels; `(-1,-1)` = left window.
    fn cursor_pos_callback(&self, pos: CursorPos, mods: Option<Mods>) -> Result<(), CoreError>;
    fn scroll_callback(&self, xoff: f64, yoff: f64, mods: ScrollMods) -> Result<(), CoreError>;
    fn mouse_pressure_callback(&self, _stage: ghostty_input::MousePressureStage, _pressure: f64) {}
    fn size_callback(&self, size: SurfaceSize) -> Result<(), CoreError>;
    fn content_scale_callback(&self, scale: ContentScale) -> Result<(), CoreError>;
    /// IME candidate window position in surface pixels (`imePoint`).
    fn ime_point(&self) -> ImePos;
    fn has_selection(&self) -> bool;
    /// `colorSchemeCallback`: the system color scheme changed.
    fn color_scheme_callback(&self, scheme: ColorScheme) -> Result<(), CoreError>;
    /// `Surface.updateConfig`: apply a new config to just this surface.
    fn update_config(&self, config: &Arc<Config>) -> Result<(), CoreError>;
    /// Perform a keybinding action on this surface; returns whether it was
    /// performed.
    fn perform_binding_action(
        &self,
        action: &ghostty_input::binding::Action,
    ) -> Result<bool, CoreError>;

    // ---- clipboard ---------------------------------------------------
    /// `completeClipboardRequest(req, data, confirmed)`.
    fn complete_clipboard_request(
        &self,
        req: ClipboardRequest,
        data: &str,
        confirmed: bool,
    ) -> Result<(), ClipboardError>;
    /// [`CoreSurface::complete_clipboard_request`] for a paste of raw bytes
    /// (Zig data is `[]const u8`). The default converts lossily.
    fn complete_paste_bytes(&self, data: &[u8], confirmed: bool) -> Result<(), ClipboardError> {
        self.complete_clipboard_request(
            ClipboardRequest::Paste,
            &String::from_utf8_lossy(data),
            confirmed,
        )
    }
    /// After the user confirms (`surface.config.clipboard_read = .allow`).
    fn set_clipboard_read_allowed(&self, allow: bool);
    /// See [`CoreSurface::set_clipboard_read_allowed`] (`clipboard_write`).
    fn set_clipboard_write_allowed(&self, allow: bool);

    // ---- lifecycle / rendering ----------------------------------------
    /// Whether closing needs confirmation (`needsConfirmQuit`).
    fn needs_confirm_quit(&self) -> bool;
    /// Whether the child exited (`child_exited`) - used by close logic.
    fn child_exited(&self) -> bool;
    /// `renderer.displayRealized()`: GLArea realized, GL context current.
    fn renderer_realized(&self) -> Result<(), CoreError>;
    /// `renderer.displayUnrealized()`: GL context current (if it could be).
    fn renderer_unrealized(&self);
    /// `renderer.drawFrame(true)`: draw into the currently bound GLArea
    /// framebuffer (GL context is current). `default_framebuffer` is the
    /// GLArea's FBO id (`GL_FRAMEBUFFER_BINDING`).
    fn draw_frame(&self, default_framebuffer: u32) -> Result<(), CoreError>;

    // ---- misc state read by the surface widget (added by F5) -------------
    /// The `SurfaceId` this core surface was created with (`new_surface`'s
    /// `id`); lets the GTK side map a core surface back to its widget.
    fn id(&self) -> Option<SurfaceId> {
        None
    }
    /// `surface.readonly` (read-only mode state).
    fn readonly(&self) -> bool {
        false
    }
    /// `surface.size.grid()` -> (columns, rows); used by the resize overlay.
    fn grid_size(&self) -> (u32, u32) {
        (0, 0)
    }
    /// `surface.font_size` (inherited by child surfaces in `setParent`).
    fn font_size(&self) -> DesiredFontSize {
        DesiredFontSize {
            points: 12.0,
            xdpi: 96,
            ydpi: 96,
        }
    }

    /// Debug aid: write the last presented frame as a PNG (GL context must be
    /// current). Used by the apprt's `GHOSTTY_DEBUG_INPUT_SCRIPT` replay.
    fn write_screenshot(&self, path: &str) -> Result<(), CoreError> {
        let _ = path;
        Err("screenshots are not supported".into())
    }

    // ---- inspector ----------------------------------------------------
    /// `activateInspector`: begin collecting inspection data. This does not
    /// affect the GUI, the runtime shows/hides the inspector UI itself.
    fn activate_inspector(&self) -> Result<(), CoreError> {
        Ok(())
    }
    /// `deactivateInspector`: stop collecting inspection data.
    fn deactivate_inspector(&self) {}
    /// `Inspector.render`: draw the inspector into the current Dear ImGui
    /// frame (the main thread; called from the inspector widget's render).
    fn render_inspector(&self, ui: &ghostty_inspector::imgui::Ui) {
        let _ = ui;
    }
}

/// Implemented by this crate (GTK `Application`); handed to core.
pub trait RtApp {
    /// `App.performAction`. Returns whether the action was performed.
    /// Runs on the main thread.
    fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool;

    /// The current keyboard layout (embedded `App.keyboardLayout`, used on
    /// macOS to derive the default `macos-option-as-alt`). Runtimes that
    /// can't tell report `Unknown`.
    fn keyboard_layout(&self) -> ghostty_input::KeyboardLayout {
        ghostty_input::KeyboardLayout::Unknown
    }
}

/// Implemented by this crate (GTK `Surface`); handed to core. Mirrors
/// `apprt/gtk/Surface.zig`.
pub trait RtSurface {
    /// `close(process_active)`.
    fn close(&self, process_active: bool);
    /// Cgroup path of the surface (Linux, `gtk-single-instance` scopes).
    fn cgroup(&self) -> Option<String>;
    fn get_title(&self) -> Option<String>;
    fn get_content_scale(&self) -> ContentScale;
    fn get_size(&self) -> SurfaceSize;
    fn get_cursor_pos(&self) -> CursorPos;
    fn supports_clipboard(&self, clipboard: Clipboard) -> bool {
        let _ = clipboard;
        true
    }
    /// `clipboardRequest`; the answer arrives later through
    /// `CoreSurface::complete_clipboard_request`. Returns true if started.
    fn clipboard_request(&self, clipboard: Clipboard, state: ClipboardRequest) -> bool;
    /// `setClipboard(type, contents, confirm)`; contents are (mime, data).
    /// The data is raw bytes like Zig's `[:0]const u8` (a copied file name
    /// need not be valid UTF-8).
    fn set_clipboard(&self, clipboard: Clipboard, contents: &[(String, Vec<u8>)], confirm: bool);
    /// `defaultTermioEnv`: env vars to set for the child (GDK/GTK related
    /// cleanups).
    fn default_termio_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)>;
    fn redraw_inspector(&self) {}
    /// Resolve an OpenGL function pointer for the surface's GL context
    /// (GTK: dlsym on libepoxy). Called with the context current. Null if
    /// the symbol is unknown.
    fn gl_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        let _ = name;
        std::ptr::null()
    }
    /// The native view the renderer attaches to (embedded runtime: the
    /// `NSView` / `UIView` the host provided; Metal renderer only). Null if
    /// the runtime has no such view.
    fn platform_view(&self) -> *mut std::ffi::c_void {
        std::ptr::null_mut()
    }
    /// Callback to create a transient systemd scope for the child process
    /// (Zig: `apprt/gtk/cgroup.zig` + `post_fork`). Called on the main
    /// thread when the surface is created; the returned closure is called
    /// from the IO thread after fork. `None` disables cgroup isolation.
    fn create_scope_fn(&self) -> Option<CreateScopeFn> {
        None
    }
}

/// `fn(pid, memory_high, tasks_max)`: `None` if there is no D-Bus
/// connection, `Some(Err(msg))` on failure.
pub type CreateScopeFn =
    Arc<dyn Fn(u32, Option<u64>, Option<u64>) -> Option<Result<(), String>> + Send + Sync>;

/// Thread-safe handle to wake the app thread (`App.wakeup`). Core's threads
/// call [`Waker::wake`]; the GTK main loop then runs `CoreApp::tick`.
pub trait Waker: Send + Sync {
    fn wake(&self);
}
