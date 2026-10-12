//! The inspector window layout and surface information
//! (ZIG/src/inspector/widgets/surface.zig).

use dear_imgui_rs::{DockNodeFlags, TableFlags, TreeNodeFlags, Ui, WindowFlags};
use ghostty_renderer::size::{Coordinate, CoordinateTag, Size, SurfaceCoord};
use ghostty_terminal::{
    page_list::Pin,
    point::{Coordinate as PointCoordinate, Tag},
    terminal::Terminal,
};

use super::{key, renderer, terminal, termio};
use crate::dock::begin_window;

/// This is discovered via the hardcoded string in the ImGui demo window.
const WINDOW_IMGUI_DEMO: &str = "Dear ImGui Demo";
const WINDOW_KEYBOARD: &str = "Keyboard";
const WINDOW_TERMINAL: &str = "Terminal";
const WINDOW_SURFACE: &str = "Surface";
const WINDOW_TERMIO: &str = "Terminal IO";
const WINDOW_RENDERER: &str = "Renderer";

/// The state of the surface that the inspector shows (the fields of the Zig
/// `Surface` that `surface.zig` reads). The core surface fills this in.
#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    /// The size of the surface (`surface.size`).
    pub size: Size,

    /// The font size in points and in pixels (`surface.font_size`).
    pub font_size_points: f32,
    pub font_size_pixels: f32,

    /// Whether each mouse button (indexed by the button's integer value) is
    /// currently pressed (`surface.mouse.click_state`).
    pub click_pressed: [bool; 12],

    /// The pin of the left click (`surface.mouse.left_click_pin`), already
    /// resolved against the active screen.
    pub left_click_pin: Option<Pin>,
    pub left_click_xpos: f64,
    pub left_click_ypos: f64,
}

#[derive(Debug)]
pub struct Inspector {
    /// Internal GUI state
    pub surface_info: Info,
    pub key_stream: key::Stream,
    pub terminal_info: terminal::Info,
    pub vt_stream: termio::Stream,
    pub renderer_info: renderer::Info,
    pub show_demo_window: bool,
}

impl Default for Inspector {
    fn default() -> Self {
        Self::new()
    }
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            surface_info: Info,
            key_stream: key::Stream::new(),
            terminal_info: terminal::Info::default(),
            vt_stream: termio::Stream::new(),
            renderer_info: renderer::Info::default(),
            show_demo_window: true,
        }
    }

    pub fn draw(&mut self, ui: &Ui, surface: &SurfaceInfo, mouse: &Mouse, t: &mut Terminal) {
        // Create our dockspace first. If we had to setup our dockspace,
        // then it is a first render.
        let first_render = Self::create_dock_space(ui);

        // Draw everything that requires the terminal state mutex (the caller
        // holds it).
        {
            // Terminal info window
            {
                let (open, _scope) = begin_window(
                    ui,
                    WINDOW_TERMINAL,
                    None,
                    WindowFlags::NO_FOCUS_ON_APPEARING,
                );
                self.terminal_info.draw(ui, open, t);
            }

            // Surface info window
            {
                let (open, _scope) =
                    begin_window(ui, WINDOW_SURFACE, None, WindowFlags::NO_FOCUS_ON_APPEARING);
                self.surface_info.draw(ui, open, surface, mouse, t);
            }

            // Keyboard info window
            {
                let (open, _scope) = begin_window(
                    ui,
                    WINDOW_KEYBOARD,
                    None,
                    WindowFlags::NO_FOCUS_ON_APPEARING,
                );
                self.key_stream.draw(ui, open);
            }

            // Terminal IO window
            {
                let (open, _scope) =
                    begin_window(ui, WINDOW_TERMIO, None, WindowFlags::NO_FOCUS_ON_APPEARING);
                if open {
                    self.vt_stream.draw(ui, &t.colors.palette.current);
                }
            }

            // Renderer info window
            {
                let (open, _scope) = begin_window(
                    ui,
                    WINDOW_RENDERER,
                    None,
                    WindowFlags::NO_FOCUS_ON_APPEARING,
                );
                self.renderer_info.draw(ui, open);
            }
        }

        // Apply the dock splits requested by the pop-out headers.
        crate::dock::flush_splits(ui);

        // In debug we show the ImGui demo window so we can easily view
        // available widgets and such.
        if cfg!(debug_assertions) && self.show_demo_window {
            ui.show_demo_window(&mut self.show_demo_window);
        }

        if first_render {
            // On first render, setup our initial focus state. We only
            // do this on first render so that we can let the user change
            // focus afterward without it snapping back.
            ui.set_window_focus(Some(WINDOW_TERMINAL));
        }
    }

    /// Create the global dock space for the inspector. A dock space
    /// is a special area where windows can be docked into. The global
    /// dock space fills the entire main viewport.
    ///
    /// Returns true if this was the first time the dock space was created.
    fn create_dock_space(ui: &Ui) -> bool {
        let dockspace_id = ui.get_id("Main Dockspace").raw();

        // Initial Docking setup
        let setup = !crate::dock::dock_node_exists(ui, dockspace_id);
        if setup {
            // Register our dockspace node, make it the full size of the
            // viewport. We only initialize one central docking point now but
            // this is the point we'd pre-split and so on for the initial
            // layout.
            let size = ui.main_viewport().size();
            crate::dock::create_dock_space_node(
                ui,
                dockspace_id,
                Some(size),
                &[
                    WINDOW_IMGUI_DEMO,
                    WINDOW_TERMINAL,
                    WINDOW_SURFACE,
                    WINDOW_KEYBOARD,
                    WINDOW_TERMIO,
                    WINDOW_RENDERER,
                ],
            );
        }

        // Put the dockspace over the viewport.
        ui.dock_space_over_main_viewport_raw(
            dockspace_id.into(),
            DockNodeFlags::PASSTHRU_CENTRAL_NODE,
        );
        setup
    }
}

/// Mouse state that we track in addition to normal mouse states that
/// Ghostty always knows about.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mouse {
    /// Last hovered x/y
    pub last_xpos: f64,
    pub last_ypos: f64,

    // Last hovered screen point
    pub last_point: Option<Pin>,
}

/// Surface information inspector widget.
#[derive(Debug, Default)]
pub struct Info;

impl Info {
    /// Draw the surface info window.
    pub fn draw(&self, ui: &Ui, open: bool, surface: &SurfaceInfo, mouse: &Mouse, t: &Terminal) {
        if !open {
            return;
        }

        if ui.collapsing_header("Help", TreeNodeFlags::NONE) {
            ui.text_wrapped(
                "This window displays information about the surface (window). \
                 A surface is the graphical area that displays the terminal \
                 content. It includes dimensions, font sizing, and mouse state \
                 information specific to this window instance.",
            );
        }

        ui.separator_with_text("Dimensions");
        dimensions_table(ui, surface);

        ui.separator_with_text("Font");
        font_table(ui, surface);

        ui.separator_with_text("Mouse");
        mouse_table(ui, surface, mouse, t);
    }
}

fn dimensions_table(ui: &Ui, surface: &SurfaceInfo) {
    let Some(_t) = ui.begin_table_with_flags("table_size", 2, TableFlags::NONE) else {
        return;
    };

    let row = |name: &str, value: String| {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text(name);
        ui.table_set_column_index(1);
        ui.text(value);
    };

    let size = &surface.size;
    row(
        "Screen Size",
        format!("{}px x {}px", size.screen.width, size.screen.height),
    );
    let grid_size = size.grid();
    row(
        "Grid Size",
        format!("{}c x {}r", grid_size.columns, grid_size.rows),
    );
    row(
        "Cell Size",
        format!("{}px x {}px", size.cell.width, size.cell.height),
    );
    row(
        "Window Padding",
        format!(
            "T={} B={} L={} R={} px",
            size.padding.top, size.padding.bottom, size.padding.left, size.padding.right
        ),
    );
}

fn font_table(ui: &Ui, surface: &SurfaceInfo) {
    let Some(_t) = ui.begin_table_with_flags("table_font", 2, TableFlags::NONE) else {
        return;
    };

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Size (Points)");
        ui.table_set_column_index(1);
        ui.text(format!("{:.2} pt", surface.font_size_points));
    }

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Size (Pixels)");
        ui.table_set_column_index(1);
        ui.text(format!("{:.2} px", surface.font_size_pixels));
    }
}

fn mouse_table(ui: &Ui, surface: &SurfaceInfo, mouse: &Mouse, t: &Terminal) {
    let Some(_t) = ui.begin_table_with_flags("table_mouse", 2, TableFlags::NONE) else {
        return;
    };

    let pages = &t.screens.active().pages;

    {
        let hover_point = mouse
            .last_point
            .and_then(|p| pages.point_from_pin(Tag::Active, p))
            .map_or_else(
                PointCoordinate::default,
                ghostty_terminal::point::Point::coord,
            );

        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Hover Grid");
        ui.table_set_column_index(1);
        ui.text(format!("row={}, col={}", hover_point.y, hover_point.x));
    }

    {
        let Coordinate::Terminal(coord) = Coordinate::Surface(SurfaceCoord {
            x: mouse.last_xpos,
            y: mouse.last_ypos,
        })
        .convert(CoordinateTag::Terminal, surface.size) else {
            unreachable!("converted to the terminal coordinate space");
        };

        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Hover Point");
        ui.table_set_column_index(1);
        ui.text(format!("({}px, {}px)", coord.x as i64, coord.y as i64));
    }

    let any_click = surface.click_pressed.iter().any(|p| *p);

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Click State");
        ui.table_set_column_index(1);
        if any_click {
            for (i, pressed) in surface.click_pressed.iter().enumerate() {
                if !*pressed {
                    continue;
                }
                ui.same_line();
                ui.text(mouse_button_label(i));
            }
        } else {
            ui.text("none");
        }
    }

    {
        let left_click_point = surface
            .left_click_pin
            .and_then(|p| pages.point_from_pin(Tag::Active, p))
            .map_or_else(
                PointCoordinate::default,
                ghostty_terminal::point::Point::coord,
            );

        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Click Grid");
        ui.table_set_column_index(1);
        ui.text(format!(
            "row={}, col={}",
            left_click_point.y, left_click_point.x
        ));
    }

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Click Point");
        ui.table_set_column_index(1);
        ui.text(format!(
            "({}px, {}px)",
            surface.left_click_xpos as u32, surface.left_click_ypos as u32
        ));
    }
}

/// The label for a mouse button given its integer value (`input.MouseButton`).
fn mouse_button_label(button: usize) -> &'static str {
    match button {
        1 => "L",
        2 => "R",
        3 => "M",
        4 => "{4}",
        5 => "{5}",
        6 => "{6}",
        7 => "{7}",
        8 => "{8}",
        9 => "{9}",
        10 => "{10}",
        11 => "{11}",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_labels() {
        assert_eq!(mouse_button_label(0), "?");
        assert_eq!(mouse_button_label(1), "L");
        assert_eq!(mouse_button_label(2), "R");
        assert_eq!(mouse_button_label(3), "M");
        assert_eq!(mouse_button_label(11), "{11}");
    }
}
