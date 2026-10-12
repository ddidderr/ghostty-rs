//! Style table (ZIG/src/inspector/widgets/style.zig).

use dear_imgui_rs::{ColorDisplayMode, ColorEditFlags, ColorEditOptions, TableFlags, Ui};
use ghostty_terminal::style::{Color, Style};
use ghostty_vt::color::{Palette, Rgb};

use super::help_marker;

pub(crate) fn rgb_f32(rgb: Rgb) -> [f32; 3] {
    [
        f32::from(rgb.r) / 255.0,
        f32::from(rgb.g) / 255.0,
        f32::from(rgb.b) / 255.0,
    ]
}

/// Render a style as a table.
pub fn table(ui: &Ui, st: &Style, palette: Option<&Palette>) {
    if let Some(_t) = ui.begin_table_with_flags("style", 2, TableFlags::NONE) {
        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Foreground");
            ui.same_line();
            help_marker(ui, "The foreground (text) color");
            ui.table_set_column_index(1);
            color(ui, "fg", st.fg_color, palette);
        }

        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Background");
            ui.same_line();
            help_marker(ui, "The background (cell) color");
            ui.table_set_column_index(1);
            color(ui, "bg", st.bg_color, palette);
        }

        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Underline");
            ui.same_line();
            help_marker(ui, "The underline color, if underlines are enabled.");
            ui.table_set_column_index(1);
            color(ui, "underline", st.underline_color, palette);
        }

        let style_flags: [(&str, bool, &str); 7] = [
            (
                "bold",
                st.flags.bold,
                "Text will be rendered with bold weight.",
            ),
            (
                "italic",
                st.flags.italic,
                "Text will be rendered in italic style.",
            ),
            (
                "faint",
                st.flags.faint,
                "Text will be rendered with reduced intensity.",
            ),
            ("blink", st.flags.blink, "Text will blink."),
            (
                "inverse",
                st.flags.inverse,
                "Foreground and background colors are swapped.",
            ),
            (
                "invisible",
                st.flags.invisible,
                "Text will be invisible (hidden).",
            ),
            (
                "strikethrough",
                st.flags.strikethrough,
                "Text will have a line through it.",
            ),
        ];
        for (name, set, help) in style_flags {
            if !set {
                continue;
            }

            ui.table_next_row();
            {
                ui.table_set_column_index(0);
                ui.text(name);
                ui.same_line();
                help_marker(ui, help);
            }
            {
                ui.table_set_column_index(1);
                ui.text("true");
            }
        }
    }

    ui.text_disabled("(Any styles not shown are not currently set)");
}

fn hex_color_edit(ui: &Ui, rgb: Rgb) {
    let mut data = rgb_f32(rgb);
    ui.color_edit3_config("color_fg", &mut data)
        .flags(
            ColorEditOptions::new()
                .flags(ColorEditFlags::NO_PICKER | ColorEditFlags::NO_LABEL)
                .display_mode(ColorDisplayMode::Hex),
        )
        .build();
}

/// Render a style color.
pub fn color(ui: &Ui, id: &str, c: Color, palette: Option<&Palette>) {
    let _id = ui.push_id(id);

    match c {
        Color::None => ui.text("default"),

        Color::Palette(idx) => {
            ui.text(format!("Palette {idx}"));
            if let Some(p) = palette {
                hex_color_edit(ui, p[usize::from(idx)]);
            }
        }

        Color::Rgb(rgb) => hex_color_edit(ui, rgb),
    }
}
