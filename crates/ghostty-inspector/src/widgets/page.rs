//! Page inspector (ZIG/src/inspector/widgets/page.zig).

use dear_imgui_rs::{ChildFlags, TableColumnFlags, TableColumnWidth, TableFlags, Ui};
use ghostty_terminal::hyperlink::HyperlinkId;
use ghostty_terminal::page::Page;

use super::{help_marker, style};
use crate::units::to_kibi_bytes;

pub(crate) fn setup_fixed(ui: &Ui, label: &str) {
    ui.table_setup_column(
        label,
        TableColumnFlags::NONE,
        Some(TableColumnWidth::Fixed(0.0)),
    );
}

pub(crate) fn setup_stretch(ui: &Ui, label: &str) {
    ui.table_setup_column(
        label,
        TableColumnFlags::NONE,
        Some(TableColumnWidth::Stretch(0.0)),
    );
}

/// The flags used by most of the page tables.
fn page_table_flags() -> TableFlags {
    TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG
}

/// Sizing policy used by most of the page tables (`SizingFixedFit`).
fn begin_page_table<'a>(
    ui: &'a Ui,
    id: &str,
    columns: usize,
) -> Option<dear_imgui_rs::TableToken<'a>> {
    ui.begin_table_with_flags(
        id,
        columns,
        dear_imgui_rs::TableOptions::default()
            .flags(page_table_flags())
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    )
}

pub fn inspector(ui: &Ui, page: &Page) {
    ui.separator_with_text("Managed Memory");
    managed_memory(ui, page);

    ui.separator_with_text("Styles");
    styles_list(ui, page);

    ui.separator_with_text("Hyperlinks");
    hyperlinks_list(ui, page);

    ui.separator_with_text("Rows");
    rows_table(ui, page);
}

/// Arguments for [`tree_node`].
pub struct TreeNodeState<'a> {
    /// The page
    pub page: &'a Page,
    /// The index of the page in a page list, used for headers.
    pub index: usize,
    /// The range of rows this page covers, inclusive.
    pub row_range: [usize; 2],
    /// Whether this page is the active or viewport node.
    pub active: bool,
    pub viewport: bool,
}

/// Draw a tree node header with metadata about this page. Returns the tree
/// node token if the tree node is open; dropping it closes the node.
pub fn tree_node<'ui>(
    ui: &'ui Ui,
    state: &TreeNodeState<'_>,
) -> Option<dear_imgui_rs::TreeNodeToken<'ui>> {
    // Setup our node.
    let label = format!("Page {}", state.index);
    let token = ui
        .tree_node_config(label.as_str())
        .allow_item_overlap(true)
        .span_full_width(true)
        .frame_padding(true)
        .push();

    // Move our cursor into the tree header so we can add extra info.
    let (header_min, header_max) = ui.item_rect();
    let header_height = header_max[1] - header_min[1];
    let text_line = ui.text_line_height();
    let y_center = header_min[1] + (header_height - text_line) * 0.5;
    ui.set_cursor_screen_pos([header_min[0] + 170.0, y_center]);

    // Metadata
    ui.text_disabled(format!(
        "{}c x {}r",
        state.page.size.cols, state.page.size.rows
    ));
    ui.same_line();
    ui.text(format!(
        "rows {}..{}",
        state.row_range[0], state.row_range[1]
    ));

    // Labels
    if state.active {
        ui.same_line();
        ui.text_colored([0.4, 0.9, 0.4, 1.0], "active");
    }
    if state.viewport {
        ui.same_line();
        ui.text_colored([0.4, 0.8, 1.0, 1.0], "viewport");
    }
    if state.page.is_dirty() {
        ui.same_line();
        ui.text_colored([1.0, 0.4, 0.4, 1.0], "dirty");
    }

    token
}

pub fn managed_memory(ui: &Ui, page: &Page) {
    let layout = Page::layout(page.capacity);

    if let Some(_t) = begin_page_table(ui, "##overview", 3) {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Memory Size");
        ui.table_set_column_index(1);
        help_marker(
            ui,
            "Memory allocated for this page. Note the backing memory \
             may be a larger allocation from which this page \
             uses a portion.",
        );
        ui.table_set_column_index(2);
        ui.text(format!("{} KiB", to_kibi_bytes(layout.total_size)));
    }

    if let Some(_t) = begin_page_table(ui, "##managed", 4) {
        setup_fixed(ui, "Resource");
        setup_fixed(ui, "");
        setup_fixed(ui, "Used");
        setup_fixed(ui, "Capacity");
        ui.table_headers_row();

        let size = page.size;
        let cap = page.capacity;
        let row = |name: &str, help: &str, used: String, capacity: String| {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text(name);
            ui.table_set_column_index(1);
            help_marker(ui, help);
            ui.table_set_column_index(2);
            ui.text(used);
            ui.table_set_column_index(3);
            ui.text(capacity);
        };

        row(
            "Columns",
            "Number of columns in the terminal grid.",
            size.cols.to_string(),
            cap.cols.to_string(),
        );
        row(
            "Rows",
            "Number of rows in this page.",
            size.rows.to_string(),
            cap.rows.to_string(),
        );
        row(
            "Styles",
            "Unique text styles (colors, attributes) currently in use.",
            page.styles.count().to_string(),
            layout.styles_layout.cap.to_string(),
        );
        row(
            "Graphemes",
            "Extended grapheme clusters for multi-codepoint characters.",
            page.grapheme_count().to_string(),
            page.grapheme_capacity().to_string(),
        );
        row(
            "Strings (bytes)",
            "String storage for hyperlink URIs and other text data.",
            page.string_used_bytes().to_string(),
            page.string_capacity_bytes().to_string(),
        );
        row(
            "Hyperlink Map",
            "Maps cell positions to hyperlink IDs.",
            page.hyperlink_count().to_string(),
            page.hyperlink_capacity().to_string(),
        );
        row(
            "Hyperlink IDs",
            "Unique hyperlink definitions (URI + optional ID).",
            page.hyperlink_set.count().to_string(),
            layout.hyperlink_set_layout.cap.to_string(),
        );
    }
}

fn rows_table(ui: &Ui, page: &Page) {
    let visible_rows = usize::from(page.size.rows).min(12);
    let row_height = ui.text_line_height_with_spacing();
    let child_height = row_height * (visible_rows as f32 + 2.0);

    // Child window so scrolling is separate.
    ui.child_window("##page_rows")
        .size([0.0, child_height])
        .child_flags(ChildFlags::BORDERS)
        .build(ui, || {
            let Some(_t) = begin_page_table(ui, "##page_rows_table", 10) else {
                return;
            };

            ui.table_setup_scroll_freeze(0, 1);
            for name in [
                "Row", "Text", "Dirty", "Wrap", "Cont", "Styled", "Grapheme", "Link", "Prompt",
                "Kitty",
            ] {
                setup_fixed(ui, name);
            }
            ui.table_headers_row();

            for row_index in 0..usize::from(page.size.rows) {
                let (Some(row), Some(cells)) = (page.get_row(row_index), page.get_cells(row_index))
                else {
                    continue;
                };
                let text_cells = cells.iter().filter(|c| c.has_text()).count();

                ui.table_next_row();

                ui.table_set_column_index(0);
                ui.text(row_index.to_string());

                ui.table_set_column_index(1);
                if text_cells == 0 {
                    ui.text_disabled("0");
                } else {
                    ui.text(text_cells.to_string());
                }

                ui.table_set_column_index(2);
                flag_cell(ui, row.dirty());

                ui.table_set_column_index(3);
                flag_cell(ui, row.wrap());

                ui.table_set_column_index(4);
                flag_cell(ui, row.wrap_continuation());

                ui.table_set_column_index(5);
                flag_cell(ui, row.styled());

                ui.table_set_column_index(6);
                flag_cell(ui, row.grapheme());

                ui.table_set_column_index(7);
                flag_cell(ui, row.hyperlink());

                ui.table_set_column_index(8);
                ui.text(crate::keys::tag_name(&row.semantic_prompt()));

                ui.table_set_column_index(9);
                flag_cell(ui, row.kitty_virtual_placeholder());
            }
        });
}

fn styles_list(ui: &Ui, page: &Page) {
    let items: Vec<_> = page
        .styles
        .entries()
        .filter(|(id, _, _)| *id != 0)
        .collect();
    let count = items.len();

    if count == 0 {
        ui.text_disabled("(no styles in use)");
        return;
    }

    let visible_rows = count.min(8);
    let row_height = ui.text_line_height_with_spacing();
    let child_height = row_height * (visible_rows as f32 + 2.0);

    ui.child_window("##page_styles")
        .size([0.0, child_height])
        .child_flags(ChildFlags::BORDERS)
        .build(ui, || {
            let Some(_t) = begin_page_table(ui, "##page_styles_table", 3) else {
                return;
            };

            ui.table_setup_scroll_freeze(0, 1);
            setup_fixed(ui, "ID");
            setup_fixed(ui, "Refs");
            setup_stretch(ui, "Style");
            ui.table_headers_row();

            for (index, st, refs) in items {
                ui.table_next_row();
                let _id = ui.push_id(i32::from(index));

                ui.table_set_column_index(0);
                ui.text(index.to_string());

                ui.table_set_column_index(1);
                ui.text(refs.to_string());

                ui.table_set_column_index(2);
                if let Some(_node) = ui.tree_node("Details") {
                    style::table(ui, st, None);
                }
            }
        });
}

fn hyperlinks_list(ui: &Ui, page: &Page) {
    let items: Vec<_> = page
        .hyperlink_set
        .entries()
        .filter(|(id, _, _)| *id != 0)
        .collect();
    let count = items.len();

    if count == 0 {
        ui.text_disabled("(no hyperlinks in use)");
        return;
    }

    let visible_rows = count.min(8);
    let row_height = ui.text_line_height_with_spacing();
    let child_height = row_height * (visible_rows as f32 + 2.0);

    ui.child_window("##page_hyperlinks")
        .size([0.0, child_height])
        .child_flags(ChildFlags::BORDERS)
        .build(ui, || {
            let Some(_t) = begin_page_table(ui, "##page_hyperlinks_table", 4) else {
                return;
            };

            ui.table_setup_scroll_freeze(0, 1);
            setup_fixed(ui, "ID");
            setup_fixed(ui, "Refs");
            setup_fixed(ui, "Explicit ID");
            setup_stretch(ui, "URI");
            ui.table_headers_row();

            for (index, link, refs) in items {
                ui.table_next_row();

                ui.table_set_column_index(0);
                ui.text(index.to_string());

                ui.table_set_column_index(1);
                ui.text(refs.to_string());

                ui.table_set_column_index(2);
                match &link.id {
                    HyperlinkId::Explicit(id) => ui.text(String::from_utf8_lossy(id)),
                    HyperlinkId::Implicit(_) => ui.text_disabled("-"),
                }

                ui.table_set_column_index(3);
                ui.text(String::from_utf8_lossy(&link.uri));
            }
        });
}

fn flag_cell(ui: &Ui, value: bool) {
    if value {
        ui.text_colored([0.4, 0.9, 0.4, 1.0], "yes");
    } else {
        ui.text_disabled("-");
    }
}
