//! Thin wrappers over Dear ImGui's imperative `DockBuilder` API.
//!
//! The safe `dear-imgui-rs` API only exposes declarative dock layouts; the
//! detachable headers need to split the node of the window that is currently
//! being built (Zig: `DockBuilderSplitNode`), so those few calls go through
//! the raw `sys` bindings. This is the only module in the crate that is
//! allowed to use `unsafe`.
#![allow(unsafe_code)]

use dear_imgui_rs::{Ui, sys};

/// The ID of the dock node of the current window (0 when undocked).
pub fn window_dock_id(ui: &Ui) -> u32 {
    ui.get_window_dock_id().raw()
}

/// A deferred "split the node to the right and dock this window there"
/// request.
struct SplitRequest {
    dock_id: u32,
    ratio: f32,
    label: String,
}

thread_local! {
    static PENDING_SPLITS: std::cell::RefCell<Vec<SplitRequest>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Request that the node `dock_id` is split to the right (`ratio` is the
/// fraction given to the new right node) and the window named `label` docked
/// into the right node. Zig performs this immediately while the parent window
/// is still being built (`DockBuilderSplitNode`, `DockBuilderDockWindow`,
/// `DockBuilderFinish`), but that re-parents the dock node of the window
/// being built, which trips an upstream Dear ImGui sanity check when
/// assertions are enabled. The request is applied by [`flush_splits`] once
/// all windows of the frame have ended, so the new window gets docked one
/// frame later.
pub fn split_right_and_dock(dock_id: u32, ratio: f32, label: &str) {
    PENDING_SPLITS.with(|p| {
        p.borrow_mut().push(SplitRequest {
            dock_id,
            ratio,
            label: label.to_owned(),
        });
    });
}

/// Apply the requests made by [`split_right_and_dock`]. Must be called with
/// no window being built.
pub fn flush_splits(ui: &Ui) {
    let requests = PENDING_SPLITS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for req in requests {
        let Ok(c_label) = std::ffi::CString::new(req.label) else {
            continue;
        };
        ui.with_bound_context(|| {
            let mut right: sys::ImGuiID = 0;
            let mut left: sys::ImGuiID = 0;
            // SAFETY: the Dear ImGui context of `ui` is bound (and current)
            // for the duration of this closure, all pointers are valid for
            // the call, and `c_label` outlives the calls that borrow it.
            unsafe {
                sys::igDockBuilderSplitNode(
                    req.dock_id,
                    sys::ImGuiDir_Right,
                    req.ratio,
                    &raw mut right,
                    &raw mut left,
                );
                sys::igDockBuilderDockWindow(c_label.as_ptr(), right);
                sys::igDockBuilderFinish(req.dock_id);
            }
        });
    }
}

/// `SetNextItemAllowOverlap`: the next item can be overlapped by items
/// submitted after it (used to put a button inside a header).
pub fn set_next_item_allow_overlap(ui: &Ui) {
    ui.with_bound_context(|| {
        // SAFETY: the context of `ui` is bound for the duration of the call.
        unsafe { sys::igSetNextItemAllowOverlap() };
    });
}

/// An open `Begin` scope; `End` is called when dropped. Unlike
/// [`Ui::window`], the scope stays open (as in Zig's `Begin` + `defer End`)
/// even when `Begin` returned false, which matters for widgets that must run
/// their per-frame logic regardless of whether the window content is visible.
pub struct WindowScope<'ui> {
    ui: &'ui Ui,
}

impl Drop for WindowScope<'_> {
    fn drop(&mut self) {
        self.ui.with_bound_context(|| {
            // SAFETY: balanced with the `igBegin` in `begin_window`.
            unsafe { sys::igEnd() };
        });
    }
}

/// `Begin(name, p_open, flags)`. Returns whether the window content is
/// visible together with the scope that ends the window.
pub fn begin_window<'ui>(
    ui: &'ui Ui,
    name: &str,
    open: Option<&mut bool>,
    flags: dear_imgui_rs::WindowFlags,
) -> (bool, WindowScope<'ui>) {
    let c_name = std::ffi::CString::new(name.replace('\0', "")).unwrap_or_default();
    let open_ptr: *mut bool = match open {
        Some(b) => b,
        None => std::ptr::null_mut(),
    };
    let visible = ui.with_bound_context(|| {
        // SAFETY: the context is bound, `c_name` is a valid NUL-terminated
        // string for the call and `open_ptr` is null or a valid exclusive
        // pointer derived from a `&mut bool`.
        unsafe { sys::igBegin(c_name.as_ptr(), open_ptr, flags.bits()) }
    });
    (visible, WindowScope { ui })
}

/// Whether a dock node with the given ID exists (`DockBuilderGetNode`).
pub fn dock_node_exists(ui: &Ui, id: u32) -> bool {
    ui.with_bound_context(|| {
        // SAFETY: the context is bound; the returned pointer is only compared
        // against null.
        !unsafe { sys::igDockBuilderGetNode(id) }.is_null()
    })
}

/// Create a dockspace node with the given ID and dock all `windows` into it
/// (`DockBuilderAddNode` with the `DockSpace` flag, optionally
/// `DockBuilderSetNodeSize`, `DockBuilderDockWindow`, `DockBuilderFinish`).
pub fn create_dock_space_node(ui: &Ui, id: u32, size: Option<[f32; 2]>, windows: &[&str]) {
    let names: Vec<std::ffi::CString> = windows
        .iter()
        .filter_map(|w| std::ffi::CString::new(*w).ok())
        .collect();
    ui.with_bound_context(|| {
        // SAFETY: the context is bound, and every `CString` outlives the
        // calls that borrow it.
        unsafe {
            let added = sys::igDockBuilderAddNode(id, sys::ImGuiDockNodeFlags_DockSpace);
            debug_assert_eq!(added, id);
            if let Some(size) = size {
                sys::igDockBuilderSetNodeSize(
                    id,
                    sys::ImVec2_c {
                        x: size[0],
                        y: size[1],
                    },
                );
            }
            for name in &names {
                sys::igDockBuilderDockWindow(name.as_ptr(), id);
            }
            sys::igDockBuilderFinish(id);
        }
    });
}

/// `SetNextWindowSize`.
pub fn set_next_window_size(ui: &Ui, size: [f32; 2], cond: dear_imgui_rs::Condition) {
    ui.with_bound_context(|| {
        // SAFETY: the context is bound for the duration of the call.
        unsafe {
            sys::igSetNextWindowSize(
                sys::ImVec2 {
                    x: size[0],
                    y: size[1],
                },
                cond as i32,
            );
        }
    });
}
