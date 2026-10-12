//! Widgets of the inspector (ZIG/src/inspector/widgets.zig and widgets/*).

use dear_imgui_rs::{ItemHoveredFlags, TreeNodeFlags, Ui, WindowFlags};

pub mod key;
pub mod page;
pub mod pagelist;
pub mod renderer;
pub mod screen;
pub mod style;
pub mod surface;
pub mod terminal;
pub mod termio;

/// Draws a "(?)" disabled text marker that shows some help text
/// on hover.
pub fn help_marker(ui: &Ui, text: &str) {
    ui.text_disabled("(?)");
    // `BeginItemTooltip`
    if !ui.is_item_hovered_with_flags(ItemHoveredFlags::FOR_TOOLTIP) {
        return;
    }
    let Some(_tooltip) = ui.begin_tooltip() else {
        return;
    };

    let _wrap = ui.push_text_wrap_pos(ui.current_font_size() * 35.0);
    ui.text(text);
}

/// `DetachableHeader` allows rendering a collapsing header that can be
/// detached into its own window.
#[derive(Debug, Clone)]
pub struct DetachableHeader {
    /// Set whether the window is detached.
    pub detached: bool,

    /// If true, detaching will move the item into a docking position
    /// to the right.
    pub dock: bool,

    // Internal state do not touch.
    window_first: bool,
}

impl Default for DetachableHeader {
    fn default() -> Self {
        Self {
            detached: false,
            dock: true,
            window_first: true,
        }
    }
}

impl DetachableHeader {
    /// Draws the detached window if this header is detached; `content` is
    /// only called when the window is open (the `Begin` return value) and
    /// `End` is always called. Does nothing if not detached.
    pub fn window(&mut self, ui: &Ui, label: &str, content: impl FnOnce()) {
        // If we're not detached, we don't create a window.
        if !self.detached {
            self.window_first = true;
            return;
        }

        // If this is our first time showing the window then we need to
        // setup docking. We only do this on the first time because we
        // don't want to reset a user's docking behavior later.
        if self.window_first {
            self.window_first = false;
            if self.dock {
                let dock_id = crate::dock::window_dock_id(ui);
                if dock_id != 0 {
                    crate::dock::split_right_and_dock(dock_id, 0.4, label);
                }
            }
        }

        let mut detached = self.detached;
        ui.window(label)
            .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
            .opened(&mut detached)
            .build(content);
        self.detached = detached;
    }

    pub fn header(&mut self, ui: &Ui, label: &str) -> bool {
        // If we're detached, create a separate window.
        if self.detached {
            return false;
        }

        // Make sure all headers have a unique ID in the stack. We only
        // need to do this for the header side because creating a window
        // automatically creates an ID.
        let _id = ui.push_id(label);

        // Create the collapsing header with the pop out button overlaid.
        crate::dock::set_next_item_allow_overlap(ui);
        let is_open = ui.collapsing_header(label, TreeNodeFlags::NONE);

        // Place pop-out button inside the header bar
        let (header_min, header_max) = ui.item_rect();
        let frame_height = ui.frame_height();
        let button_size = frame_height - 4.0;
        let padding = 4.0;

        ui.same_line();
        ui.set_cursor_screen_pos([header_max[0] - button_size - padding, header_min[1] + 2.0]);
        {
            let _style = ui.push_style_var(dear_imgui_rs::StyleVar::FramePadding([0.0, 0.0]));
            if ui.button_with_size(">>##detach", [button_size, button_size]) {
                self.detached = true;
            }
        }

        if ui.is_item_hovered_with_flags(ItemHoveredFlags::DELAY_SHORT) {
            ui.set_tooltip("Detach into separate window");
        }

        is_open
    }
}

#[derive(Debug, Clone, Default)]
pub struct DetachableHeaderState {
    pub show_window: bool,

    /// Internal state. Don't touch.
    first_show: bool,
}

/// Render a collapsing header that can be detached into its own window.
/// When detached, renders as a separate window with a close button.
/// When attached, renders as a collapsing header with a pop-out button.
pub fn detachable_header(
    ui: &Ui,
    label: &str,
    state: &mut DetachableHeaderState,
    content: impl FnOnce(),
) {
    let _id = ui.push_id(label);

    if state.show_window {
        // On first show, dock this window to the right of the parent
        // window's dock. We only do this once so the user can freely
        // reposition the window afterward without it snapping back to the
        // right on every frame.
        if !state.first_show {
            state.first_show = true;
            let current_dock_id = crate::dock::window_dock_id(ui);
            if current_dock_id != 0 {
                crate::dock::split_right_and_dock(current_dock_id, 0.3, label);
            }
        }

        let mut show = state.show_window;
        ui.window(label)
            .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
            .opened(&mut show)
            .build(content);
        state.show_window = show;
        return;
    }

    // Reset first_show when window is closed so next open docks again
    state.first_show = false;

    crate::dock::set_next_item_allow_overlap(ui);
    let is_open = ui.collapsing_header(label, TreeNodeFlags::NONE);

    // Place pop-out button inside the header bar
    let (header_min, header_max) = ui.item_rect();
    let frame_height = ui.frame_height();
    let button_size = frame_height - 4.0;
    let padding = 4.0;

    ui.same_line();
    ui.set_cursor_screen_pos([header_max[0] - button_size - padding, header_min[1] + 2.0]);
    {
        let _style = ui.push_style_var(dear_imgui_rs::StyleVar::FramePadding([0.0, 0.0]));
        if ui.button_with_size(">>##detach", [button_size, button_size]) {
            state.show_window = true;
        }
    }
    if ui.is_item_hovered_with_flags(ItemHoveredFlags::DELAY_SHORT) {
        ui.set_tooltip("Pop out into separate window");
    }

    if is_open {
        content();
    }
}
