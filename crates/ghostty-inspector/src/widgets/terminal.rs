//! Terminal information (ZIG/src/inspector/widgets/terminal.zig).

use dear_imgui_rs::{
    ColorEditFlags, Condition, ItemHoveredFlags, StyleVar, TableColumnFlags, TableColumnWidth,
    TableFlags, TableRowFlags, Ui, WindowFlags,
};
use ghostty_terminal::screen_set::Key as ScreenKey;
use ghostty_terminal::terminal::{MouseShiftCapture, Terminal};
use ghostty_vt::color::{DynamicRgb, Palette};
use ghostty_vt::modes::ENTRIES as MODE_ENTRIES;

use super::{DetachableHeader, help_marker, screen, style::rgb_f32};
use crate::keys::tag_name;

const SCREEN_KEYS: [ScreenKey; 2] = [ScreenKey::Primary, ScreenKey::Alternate];

fn screen_index(key: ScreenKey) -> usize {
    match key {
        ScreenKey::Primary => 0,
        ScreenKey::Alternate => 1,
    }
}

/// Terminal information inspector widget.
#[derive(Debug, Default)]
pub struct Info {
    /// True if we're showing the 256-color palette window.
    pub show_palette: bool,

    /// The various detachable headers.
    pub misc_header: DetachableHeader,
    pub layout_header: DetachableHeader,
    pub mouse_header: DetachableHeader,
    pub color_header: DetachableHeader,
    pub modes_header: DetachableHeader,

    /// Screen detail windows for each screen key.
    pub screens: [Option<screen::Info>; 2],
}

impl Info {
    /// Draw the terminal info window.
    pub fn draw(&mut self, ui: &Ui, open: bool, t: &mut Terminal) {
        // Draw our open state if we're open.
        if open {
            self.draw_open(ui, t);
        }

        // Draw our detached state that draws regardless of if
        // we're open or not.
        self.misc_header
            .window(ui, "Terminal Misc", || misc_table(ui, t));
        self.layout_header
            .window(ui, "Terminal Layout", || layout_table(ui, t));
        self.mouse_header
            .window(ui, "Terminal Mouse", || mouse_table(ui, t));
        {
            let show_palette = &mut self.show_palette;
            self.color_header
                .window(ui, "Terminal Color", || color_table(ui, t, show_palette));
        }
        self.modes_header
            .window(ui, "Terminal Modes", || modes_table(ui, t));

        // Palette pop-out window
        if self.show_palette {
            let mut show = self.show_palette;
            ui.window("256-Color Palette")
                .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
                .opened(&mut show)
                .build(|| palette(ui, "palette", &t.colors.palette.current));
            self.show_palette = show;
        }

        // Screen pop-out windows
        for key in SCREEN_KEYS {
            let idx = screen_index(key);
            if self.screens[idx].is_none() {
                continue;
            }

            let modify_other_keys_2 = t.flags.modify_other_keys_2;
            let active_key = t.screens.active_key;
            let palette = &t.colors.palette;
            let Some(screen_ref) = t.screens.get_mut(key) else {
                // Could happen if we opened up a window for a screen
                // and that screen was subsequently deinitialized. In
                // this case, hide the window.
                self.screens[idx] = None;
                continue;
            };

            let title = format!("Screen: {}", tag_name(&key));

            // Setup our next window so it has some size to it.
            let viewport_size = ui.main_viewport().size();
            crate::dock::set_next_window_size(
                ui,
                [
                    400.0_f32.min(viewport_size[0]),
                    300.0_f32.min(viewport_size[1]),
                ],
                Condition::FirstUseEver,
            );

            let mut screen_open = true;
            let (screen_draw, _scope) = crate::dock::begin_window(
                ui,
                &title,
                Some(&mut screen_open),
                WindowFlags::NO_FOCUS_ON_APPEARING,
            );
            if let Some(info) = self.screens[idx].as_mut() {
                info.draw(
                    ui,
                    screen_draw,
                    screen::Data {
                        screen: screen_ref,
                        key,
                        active_key,
                        modify_other_keys_2,
                        color_palette: palette,
                    },
                );
            }

            // If the window was closed, remove it from our map so future
            // renders don't draw it.
            if !screen_open {
                self.screens[idx] = None;
            }
        }
    }

    fn draw_open(&mut self, ui: &Ui, t: &mut Terminal) {
        // Show our screens up top.
        screens_table(ui, t, &mut self.screens);

        if self.misc_header.header(ui, "Misc") {
            misc_table(ui, t);
        }
        if self.layout_header.header(ui, "Layout") {
            layout_table(ui, t);
        }
        if self.mouse_header.header(ui, "Mouse") {
            mouse_table(ui, t);
        }
        if self.color_header.header(ui, "Color") {
            color_table(ui, t, &mut self.show_palette);
        }
        if self.modes_header.header(ui, "Modes") {
            modes_table(ui, t);
        }
    }
}

/// Render the table of possible screens with various actions.
fn screens_table(ui: &Ui, t: &Terminal, map: &mut [Option<screen::Info>; 2]) {
    let Some(_t) = ui.begin_table_with_flags(
        "screens",
        3,
        dear_imgui_rs::TableOptions::default()
            .flags(TableFlags::BORDERS | TableFlags::ROW_BG)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    ) else {
        return;
    };

    let fixed = |name: &str| {
        ui.table_setup_column(
            name,
            TableColumnFlags::NONE,
            Some(TableColumnWidth::Fixed(0.0)),
        );
    };
    fixed("Screen");
    fixed("Status");
    fixed("");

    // Custom header row to include help marker before "Screen"
    {
        ui.table_next_row_with_flags(TableRowFlags::HEADERS, 0.0);
        {
            ui.table_set_column_index(0);
            {
                let _style = ui.push_style_var(StyleVar::FramePadding([0.0, 0.0]));
                help_marker(
                    ui,
                    "A terminal can have multiple screens, only one of which is active at \
                     a time. Each screen has its own grid, contents, and other state. \
                     This section allows you to inspect the different screens managed by \
                     the terminal.",
                );
            }
            ui.same_line_with_spacing(0.0, ui.clone_style().item_inner_spacing()[0]);
            ui.table_header("Screen");
        }
        {
            ui.table_set_column_index(1);
            ui.table_header("Status");
        }
        {
            ui.table_set_column_index(2);
            ui.table_header("");
        }
    }

    for key in SCREEN_KEYS {
        let is_initialized = t.screens.get(key).is_some();
        let is_active = t.screens.active_key == key;

        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text(tag_name(&key));
        }
        {
            ui.table_set_column_index(1);
            if is_active {
                ui.text_colored([0.4, 1.0, 0.4, 1.0], "active");
            } else if is_initialized {
                ui.text_colored([0.6, 0.6, 0.6, 1.0], "initialized");
            } else {
                ui.text_colored([0.4, 0.4, 0.4, 1.0], "(not initialized)");
            }
        }
        {
            ui.table_set_column_index(2);
            let _id = ui.push_id(screen_index(key) as i32);
            let _disabled = ui.begin_disabled_with_cond(!is_initialized);
            if ui.button("View") {
                let slot = &mut map[screen_index(key)];
                if slot.is_none() {
                    *slot = Some(screen::Info::default());
                }
            }
        }
    }
}

/// Table of miscellaneous terminal information.
fn misc_table(ui: &Ui, t: &Terminal) {
    let Some(_t) = ui.begin_table_with_flags("table_misc", 2, TableFlags::NONE) else {
        return;
    };

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Working Directory");
            ui.same_line();
            help_marker(ui, "The current working directory reported by the shell.");
        }
        {
            ui.table_set_column_index(1);
            if t.pwd.is_empty() {
                ui.text_disabled("(none)");
            } else {
                ui.text(String::from_utf8_lossy(&t.pwd));
            }
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Focused");
            ui.same_line();
            help_marker(ui, "Whether the terminal itself is currently focused.");
        }
        {
            ui.table_set_column_index(1);
            let mut value = t.flags.focused;
            ui.checkbox("##focused", &mut value);
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Previous Char");
            ui.same_line();
            help_marker(
                ui,
                "The previously printed character, used only for the REP sequence.",
            );
        }
        {
            ui.table_set_column_index(1);
            if let Some(c) = t.previous_char {
                ui.text(format!("U+{c:04X}"));
            } else {
                ui.text_disabled("(none)");
            }
        }
    }
}

/// Table of terminal layout information.
fn layout_table(ui: &Ui, t: &mut Terminal) {
    let Some(_t) = ui.begin_table_with_flags("table_layout", 2, TableFlags::NONE) else {
        return;
    };

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Grid");
            ui.same_line();
            help_marker(ui, "The size of the terminal grid in columns and rows.");
        }
        {
            ui.table_set_column_index(1);
            ui.text(format!("{}c x {}r", t.cols, t.rows));
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Pixels");
            ui.same_line();
            help_marker(ui, "The size of the terminal grid in pixels.");
        }
        {
            ui.table_set_column_index(1);
            ui.text(format!("{}w x {}h", t.width_px, t.height_px));
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Scroll Region");
            ui.same_line();
            help_marker(
                ui,
                "The scrolling region boundaries (top, bottom, left, right).",
            );
        }
        {
            ui.table_set_column_index(1);
            let _width = ui.push_item_width(ui.calc_text_size("00000")[0]);

            let mut over = t.scrolling_region;
            let mut changed = false;
            let rows_max = t.rows.saturating_sub(1);
            let cols_max = t.cols.saturating_sub(1);

            ui.align_text_to_frame_padding();
            ui.text("T:");
            ui.same_line();
            if ui.input_scalar("##scroll_top", &mut over.top).build() {
                over.top = over.top.min(rows_max);
                changed = true;
            }

            ui.same_line();
            ui.text("B:");
            ui.same_line();
            if ui.input_scalar("##scroll_bottom", &mut over.bottom).build() {
                over.bottom = over.bottom.min(rows_max);
                changed = true;
            }

            ui.same_line();
            ui.text("L:");
            ui.same_line();
            if ui.input_scalar("##scroll_left", &mut over.left).build() {
                over.left = over.left.min(cols_max);
                changed = true;
            }

            ui.same_line();
            ui.text("R:");
            ui.same_line();
            if ui.input_scalar("##scroll_right", &mut over.right).build() {
                over.right = over.right.min(cols_max);
                changed = true;
            }

            if changed && over.top < over.bottom && over.left < over.right {
                t.scrolling_region = over;
            }
        }
    }
}

/// Table of mouse-related terminal information.
fn mouse_table(ui: &Ui, t: &Terminal) {
    let Some(_t) = ui.begin_table_with_flags("table_mouse", 2, TableFlags::NONE) else {
        return;
    };

    let row = |name: &str, help: &str, value: &dyn Fn()| {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text(name);
            ui.same_line();
            help_marker(ui, help);
        }
        {
            ui.table_set_column_index(1);
            value();
        }
    };

    row(
        "Event Mode",
        "The mouse event reporting mode set by the application.",
        &|| ui.text(tag_name(&t.flags.mouse_event)),
    );
    row("Format", "The mouse event encoding format.", &|| {
        ui.text(tag_name(&t.flags.mouse_format));
    });
    row(
        "Shape",
        "The current mouse cursor shape set by the application.",
        &|| ui.text(tag_name(&t.mouse_shape)),
    );
    row(
        "Shift Capture",
        "XTSHIFTESCAPE state for capturing shift in mouse protocol.",
        &|| {
            if t.flags.mouse_shift_capture == MouseShiftCapture::Null {
                ui.text_disabled("(unset)");
            } else {
                ui.text(tag_name(&t.flags.mouse_shift_capture));
            }
        },
    );
}

/// Table of color-related terminal information.
fn color_table(ui: &Ui, t: &Terminal, show_palette: &mut bool) {
    ui.text_wrapped(
        "Color state for the terminal. Note these colors only apply \
         to the palette and unstyled colors. Many modern terminal \
         applications use direct RGB colors which are not reflected here.",
    );
    ui.separator();

    let Some(_t) = ui.begin_table_with_flags("table_color", 2, TableFlags::NONE) else {
        return;
    };

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Background");
            ui.same_line();
            help_marker(ui, "Unstyled cell background color.");
        }
        {
            ui.table_set_column_index(1);
            dynamic_rgb(ui, "bg_color", t.colors.background);
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Foreground");
            ui.same_line();
            help_marker(ui, "Unstyled cell foreground color.");
        }
        {
            ui.table_set_column_index(1);
            dynamic_rgb(ui, "fg_color", t.colors.foreground);
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Cursor");
            ui.same_line();
            help_marker(ui, "Cursor coloring set by escape sequences.");
        }
        {
            ui.table_set_column_index(1);
            dynamic_rgb(ui, "cursor_color", t.colors.cursor);
        }
    }

    {
        ui.table_next_row();
        {
            ui.table_set_column_index(0);
            ui.text("Palette");
            ui.same_line();
            help_marker(ui, "The 256-color palette.");
        }
        {
            ui.table_set_column_index(1);
            if ui.button("View") {
                *show_palette = true;
            }
        }
    }
}

/// Table of terminal modes.
fn modes_table(ui: &Ui, t: &Terminal) {
    let Some(_t) = ui.begin_table_with_flags(
        "table_modes",
        3,
        dear_imgui_rs::TableOptions::default()
            .flags(TableFlags::ROW_BG)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    ) else {
        return;
    };

    {
        ui.table_setup_column("", TableColumnFlags::NO_RESIZE, None);
        ui.table_setup_column("Number", TableColumnFlags::PREFER_SORT_ASCENDING, None);
        ui.table_setup_column(
            "Name",
            TableColumnFlags::NONE,
            Some(TableColumnWidth::Stretch(0.0)),
        );
        ui.table_headers_row();
    }

    for entry in MODE_ENTRIES {
        ui.table_next_row();
        let _id = ui.push_id(i32::from(entry.value));
        {
            ui.table_set_column_index(0);
            let mut value = t.modes.get(entry.mode);
            ui.checkbox("##checkbox", &mut value);
        }
        {
            ui.table_set_column_index(1);
            ui.text(format!(
                "{}{}",
                if entry.ansi { "" } else { "?" },
                entry.value
            ));
        }
        {
            ui.table_set_column_index(2);
            ui.text(entry.name);
        }
    }
}

/// Render a DynamicRGB color.
fn dynamic_rgb(ui: &Ui, label: &str, rgb: DynamicRgb) {
    let columns = if rgb.override_.is_some() { 2 } else { 1 };
    let Some(_t) = ui.begin_table_with_flags(
        label,
        columns,
        dear_imgui_rs::TableOptions::default()
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    ) else {
        return;
    };

    if rgb.override_.is_some() {
        ui.table_setup_column(
            "##label",
            TableColumnFlags::NONE,
            Some(TableColumnWidth::Fixed(0.0)),
        );
    }
    ui.table_setup_column(
        "##value",
        TableColumnFlags::NONE,
        Some(TableColumnWidth::Stretch(0.0)),
    );

    if let Some(c) = rgb.override_ {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("override:");
        ui.same_line();
        help_marker(ui, "Overridden color set by escape sequences.");

        ui.table_set_column_index(1);
        let mut col = rgb_f32(c);
        ui.color_edit3("##override", &mut col);
    }

    ui.table_next_row();
    ui.table_set_column_index(0);
    if let Some(c) = rgb.default {
        if rgb.override_.is_some() {
            ui.text("default:");
            ui.same_line();
            help_marker(ui, "Default color from configuration.");

            ui.table_set_column_index(1);
        }

        let mut col = rgb_f32(c);
        ui.color_edit3("##default", &mut col);
    } else {
        ui.text_disabled("(unset)");
    }
}

/// Render a color palette as a 16x16 grid of color buttons.
fn palette(ui: &Ui, label: &str, pal: &Palette) {
    let _id = ui.push_id(label);

    for row in 0..16_usize {
        for col in 0..16_usize {
            let idx = row * 16 + col;
            let rgb = pal[idx];
            let mut col_arr = rgb_f32(rgb);

            if col > 0 {
                ui.same_line();
            }

            let _idx_id = ui.push_id(idx as i32);
            ui.color_edit3_config("##color", &mut col_arr)
                .flags(ColorEditFlags::NO_INPUTS)
                .build();
            if ui.is_item_hovered_with_flags(ItemHoveredFlags::DELAY_SHORT) {
                ui.set_tooltip(format!("{}: #{:02X}{:02X}{:02X}", idx, rgb.r, rgb.g, rgb.b));
            }
        }
    }
}
