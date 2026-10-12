//! Screen information (ZIG/src/inspector/widgets/screen.zig).

use dear_imgui_rs::{DockNodeFlags, TableFlags, TreeNodeFlags, Ui, WindowFlags};
use ghostty_terminal::kitty::graphics_storage::ImageStorage;
use ghostty_terminal::screen::{Cursor, Screen, SemanticClick, SemanticPrompt};
use ghostty_terminal::screen_set::Key as ScreenKey;
use ghostty_terminal::style::Style;
use ghostty_vt::color::{DynamicPalette, Palette};

use super::{help_marker, pagelist, style as style_widget};
use crate::units::to_kibi_bytes;

/// Window names for the screen dockspace.
const WINDOW_INFO: &str = "Info";
const WINDOW_CELL: &str = "Cell";
const WINDOW_PAGELIST: &str = "PageList";

/// A copy of the parts of a [`Cursor`] that the inspector shows. Zig copies
/// the whole cursor struct (including pointers) into recorded VT events; in
/// Rust the cursor owns a heap hyperlink and a tracked pin so we snapshot the
/// displayed fields instead.
#[derive(Debug, Clone)]
pub struct CursorSnapshot {
    pub x: u16,
    pub y: u16,
    pub pending_wrap: bool,
    pub protected: bool,
    pub style: Style,
    /// The URI of the active hyperlink, if any.
    pub hyperlink_uri: Option<String>,
}

impl CursorSnapshot {
    pub fn new(cursor: &Cursor) -> Self {
        Self {
            x: cursor.x,
            y: cursor.y,
            pending_wrap: cursor.pending_wrap,
            protected: cursor.protected,
            style: cursor.style,
            hyperlink_uri: cursor
                .hyperlink
                .as_ref()
                .map(|h| String::from_utf8_lossy(&h.uri).into_owned()),
        }
    }
}

/// Screen information inspector widget.
#[derive(Debug, Default)]
pub struct Info {
    pub pagelist: pagelist::Inspector,
    pub cell_chooser: pagelist::CellChooser,
}

/// The data needed to draw a screen.
pub struct Data<'a> {
    /// The screen that we're inspecting.
    pub screen: &'a mut Screen,

    /// Which screen key we're viewing.
    pub key: ScreenKey,

    /// Which screen is active (primary or alternate).
    pub active_key: ScreenKey,

    /// Whether xterm modify other keys mode 2 is enabled.
    pub modify_other_keys_2: bool,

    /// Color palette for cursor color resolution.
    pub color_palette: &'a DynamicPalette,
}

impl Info {
    /// Draw the screen info contents.
    pub fn draw(&mut self, ui: &Ui, open: bool, data: Data<'_>) {
        // Create the dockspace for this screen
        Self::create_dock_space(ui);

        let screen = data.screen;

        // Info window
        ui.window(WINDOW_INFO)
            .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
            .build(|| {
                if ui.collapsing_header("Cursor", TreeNodeFlags::DEFAULT_OPEN) {
                    let snapshot = CursorSnapshot::new(&screen.cursor);
                    cursor_table(ui, &snapshot);
                    ui.separator();
                    cursor_style(ui, &snapshot, Some(&data.color_palette.current));
                }

                if ui.collapsing_header("Keyboard", TreeNodeFlags::DEFAULT_OPEN) {
                    keyboard_table(ui, screen, data.modify_other_keys_2);
                }

                if ui.collapsing_header("Semantic Prompt", TreeNodeFlags::DEFAULT_OPEN) {
                    semantic_prompt_table(ui, &screen.semantic_prompt);
                }

                if ui.collapsing_header("Kitty Graphics", TreeNodeFlags::DEFAULT_OPEN) {
                    kitty_graphics_table(ui, &screen.kitty_images);
                }

                if ui.collapsing_header("Other Screen State", TreeNodeFlags::DEFAULT_OPEN) {
                    internal_state_table(ui, screen);
                }
            });

        // Cell window
        ui.window(WINDOW_CELL)
            .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
            .build(|| {
                self.cell_chooser.draw(ui, &screen.pages);
            });

        // PageList window
        ui.window(WINDOW_PAGELIST)
            .flags(WindowFlags::NO_FOCUS_ON_APPEARING)
            .build(|| {
                self.pagelist.draw(ui, &mut screen.pages);
            });

        // The remainder is the open state
        if !open {
            return;
        }

        // Show warning if viewing an inactive screen
        if data.key != data.active_key {
            ui.text_colored([1.0, 0.8, 0.0, 1.0], "\u{26a0} Viewing inactive screen");
            ui.separator();
        }
    }

    /// Create the dock space for the screen inspector. This creates
    /// a dedicated dock space for the screen inspector windows. But they
    /// can of course be undocked and moved around as desired.
    fn create_dock_space(ui: &Ui) {
        let dockspace_id = ui.get_id("Screen Dockspace").raw();

        // Check if we need to set up the dockspace
        if !crate::dock::dock_node_exists(ui, dockspace_id) {
            // Register our dockspace node and dock windows into the space
            crate::dock::create_dock_space_node(
                ui,
                dockspace_id,
                None,
                &[WINDOW_INFO, WINDOW_CELL, WINDOW_PAGELIST],
            );
        }

        // Create the dockspace
        let avail = ui.content_region_avail();
        ui.dock_space_raw(
            dockspace_id.into(),
            [avail[0].max(1.0), avail[1].max(1.0)],
            DockNodeFlags::NONE,
            None,
        );
    }
}

/// Render cursor state with a table of cursor-specific fields.
pub fn cursor_table(ui: &Ui, cursor: &CursorSnapshot) {
    let Some(_t) = ui.begin_table_with_flags("table_cursor", 2, TableFlags::NONE) else {
        return;
    };

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Position (x, y)");
    ui.same_line();
    help_marker(
        ui,
        "The current cursor position in the terminal grid (0-indexed).",
    );
    ui.table_set_column_index(1);
    ui.text(format!("({}, {})", cursor.x, cursor.y));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Hyperlink");
    ui.same_line();
    help_marker(
        ui,
        "The active OSC8 hyperlink for newly printed characters.",
    );
    ui.table_set_column_index(1);
    if let Some(uri) = &cursor.hyperlink_uri {
        ui.text(uri);
    } else {
        ui.text_disabled("(none)");
    }

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Pending Wrap");
        ui.same_line();
        help_marker(
            ui,
            "The 'last column flag' (LCF). If set, the next character will force a soft-wrap to the next line.",
        );
        ui.table_set_column_index(1);
        let mut value = cursor.pending_wrap;
        ui.checkbox("##pending_wrap", &mut value);
    }

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Protected");
        ui.same_line();
        help_marker(
            ui,
            "If enabled, new characters will have the protected attribute set, preventing erasure by certain sequences.",
        );
        ui.table_set_column_index(1);
        let mut value = cursor.protected;
        ui.checkbox("##protected", &mut value);
    }
}

/// Render cursor style information using the shared style table.
pub fn cursor_style(ui: &Ui, cursor: &CursorSnapshot, palette: Option<&Palette>) {
    style_widget::table(ui, &cursor.style, palette);
}

/// Render keyboard information with a table.
fn keyboard_table(ui: &Ui, screen: &Screen, modify_other_keys_2: bool) {
    let Some(_t) = ui.begin_table_with_flags("table_keyboard", 2, TableFlags::NONE) else {
        return;
    };

    let kitty_flags = screen.kitty_keyboard.current();

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Mode");
        ui.table_set_column_index(1);
        let mode = if kitty_flags.int() != 0 {
            "kitty"
        } else {
            "legacy"
        };
        ui.text(mode);
    }

    if kitty_flags.int() != 0 {
        let fields: [(&str, bool); 5] = [
            ("disambiguate", kitty_flags.disambiguate),
            ("report_events", kitty_flags.report_events),
            ("report_alternates", kitty_flags.report_alternates),
            ("report_all", kitty_flags.report_all),
            ("report_associated", kitty_flags.report_associated),
        ];
        for (name, value) in fields {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text(name);
            ui.table_set_column_index(1);
            ui.text(if value { "true" } else { "false" });
        }
    } else {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Xterm modify keys");
        ui.table_set_column_index(1);
        ui.text(if modify_other_keys_2 { "true" } else { "false" });
    }
}

/// Render kitty graphics information table.
pub fn kitty_graphics_table(ui: &Ui, kitty_images: &ImageStorage) {
    if !kitty_images.enabled() {
        ui.text_disabled("(Kitty graphics are disabled)");
        return;
    }

    let Some(_t) = ui.begin_table_with_flags("##kitty_graphics", 2, TableFlags::NONE) else {
        return;
    };

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Memory Usage");
    ui.table_set_column_index(1);
    ui.text(format!(
        "{} bytes ({} KiB)",
        kitty_images.total_bytes,
        to_kibi_bytes(kitty_images.total_bytes)
    ));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Memory Limit");
    ui.table_set_column_index(1);
    ui.text(format!(
        "{} bytes ({} KiB)",
        kitty_images.total_limit,
        to_kibi_bytes(kitty_images.total_limit)
    ));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Image Count");
    ui.table_set_column_index(1);
    ui.text(format!("{}", kitty_images.images.len()));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Placement Count");
    ui.table_set_column_index(1);
    ui.text(format!("{}", kitty_images.placements.len()));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Image Loading");
    ui.table_set_column_index(1);
    ui.text(if kitty_images.loading.is_some() {
        "true"
    } else {
        "false"
    });
}

/// Render internal terminal state table.
pub fn internal_state_table(ui: &Ui, screen: &Screen) {
    let pages = &screen.pages;

    let Some(_t) = ui.begin_table_with_flags("##terminal_state", 2, TableFlags::NONE) else {
        return;
    };

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Memory Usage");
    ui.table_set_column_index(1);
    ui.text(format!(
        "{} bytes ({} KiB)",
        pages.page_size,
        to_kibi_bytes(pages.page_size)
    ));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Memory Limit");
    ui.table_set_column_index(1);
    ui.text(format!(
        "{} bytes ({} KiB)",
        pages.max_size(),
        to_kibi_bytes(pages.max_size())
    ));

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Viewport Location");
    ui.table_set_column_index(1);
    ui.text(crate::keys::tag_name(&pages.viewport));
}

/// Render semantic prompt state table.
pub fn semantic_prompt_table(ui: &Ui, semantic_prompt: &SemanticPrompt) {
    let Some(_t) = ui.begin_table_with_flags("##semantic_prompt", 2, TableFlags::NONE) else {
        return;
    };

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Seen");
        ui.same_line();
        help_marker(
            ui,
            "Whether any semantic prompt markers (OSC 133) have been seen in this screen.",
        );
        ui.table_set_column_index(1);
        let mut value = semantic_prompt.seen;
        ui.checkbox("##seen", &mut value);
    }

    {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Click Handling");
        ui.same_line();
        help_marker(
            ui,
            "How click events are handled in prompts. Set via 'cl' or 'click_events' options in OSC 133.",
        );
        ui.table_set_column_index(1);
        match semantic_prompt.click {
            SemanticClick::None => ui.text_disabled("(none)"),
            SemanticClick::ClickEvents => ui.text("click_events"),
            SemanticClick::Cl(cl) => ui.text(format!("cl={}", crate::keys::tag_name(&cl))),
        }
    }
}
