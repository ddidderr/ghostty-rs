//! PageList inspector (ZIG/src/inspector/widgets/pagelist.zig).

use dear_imgui_rs::{
    ComboBoxHeight,
    ComboBoxOptions,
    ItemHoveredFlags,
    MouseButton,
    StyleColor,
    TableFlags,
    TreeNodeFlags,
    Ui,
};
use ghostty_terminal::{
    page_list::{PageList, Scroll, Scrollbar},
    point::{Coordinate, Point, Tag},
    style::DEFAULT_ID,
};

use super::{help_marker, page, style};
use crate::units::to_kibi_bytes;

fn begin_table<'a>(ui: &'a Ui, id: &str, columns: usize) -> Option<dear_imgui_rs::TableToken<'a>> {
    ui.begin_table_with_flags(
        id,
        columns,
        dear_imgui_rs::TableOptions::default()
            .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    )
}

const TAGS: [Tag; 4] = [Tag::Active, Tag::Viewport, Tag::Screen, Tag::History];

/// PageList inspector widget.
#[derive(Debug, Default)]
pub struct Inspector;

impl Inspector {
    pub fn draw(&self, ui: &Ui, pages: &mut PageList) {
        ui.text_wrapped(
            "PageList manages the backing pages that hold scrollback and the active \
             terminal grid. Each page is a contiguous memory buffer with its \
             own rows, cells, style set, grapheme map, and hyperlink storage.",
        );

        if ui.collapsing_header("Overview", TreeNodeFlags::DEFAULT_OPEN) {
            summary_table(ui, pages);
        }

        if ui.collapsing_header("Scrollbar & Regions", TreeNodeFlags::DEFAULT_OPEN) {
            ui.separator_with_text("Scrollbar");
            scrollbar_info(ui, pages);
            ui.separator_with_text("Regions");
            regions_table(ui, pages);
        }

        if ui.collapsing_header("Tracked Pins", TreeNodeFlags::DEFAULT_OPEN) {
            tracked_pins_table(ui, pages);
        }

        if ui.collapsing_header("Pages", TreeNodeFlags::DEFAULT_OPEN) {
            help_marker(
                ui,
                "Pages are shown most-recent first. Each page holds a grid of rows/cells \
                 plus metadata tables for styles, graphemes, strings, and hyperlinks.",
            );

            let active_pin = pages.get_top_left(Tag::Active);
            let viewport_pin = pages.get_top_left(Tag::Viewport);

            let mut row_offset = pages.total_rows;
            let mut index = pages.total_pages();
            let mut node = pages.last;
            while let Some(node_id) = node {
                let Some(page_node) = pages.node(node_id) else {
                    break;
                };
                let pg = &page_node.data;
                node = page_node.prev;
                row_offset -= usize::from(pg.size.rows);
                index -= 1;

                // We use our location as the ID so that even if reallocations
                // happen we remain open if we're open already.
                let _id = ui.push_id(index as i32);

                // Open up the tree node.
                let Some(_open) = page::tree_node(
                    ui,
                    &page::TreeNodeState {
                        page: pg,
                        index,
                        row_range: [
                            row_offset,
                            (row_offset + usize::from(pg.size.rows)).wrapping_sub(1),
                        ],
                        active: active_pin.is_some_and(|p| p.node == node_id),
                        viewport: viewport_pin.is_some_and(|p| p.node == node_id),
                    },
                ) else {
                    continue;
                };
                page::inspector(ui, pg);
            }
        }
    }
}

fn summary_table(ui: &Ui, pages: &PageList) {
    let Some(_t) = begin_table(ui, "pagelist_summary", 3) else {
        return;
    };

    let row = |name: &str, help: &str, value: String| {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text(name);
        ui.table_set_column_index(1);
        help_marker(ui, help);
        ui.table_set_column_index(2);
        ui.text(value);
    };

    row(
        "Active Grid",
        "Active viewport size in columns x rows.",
        format!("{}c x {}r", pages.cols, pages.rows),
    );
    row(
        "Pages",
        "Total number of pages in the linked list.",
        pages.total_pages().to_string(),
    );
    row(
        "Total Rows",
        "Total rows represented by scrollback + active area.",
        pages.total_rows.to_string(),
    );
    row(
        "Page Bytes",
        "Total bytes allocated for active pages.",
        format!("{} KiB", to_kibi_bytes(pages.page_size)),
    );
    row(
        "Max Size",
        "Maximum bytes before pages must be evicated. The total\n\
         used bytes may be higher due to minimum individual page\n\
         sizes but the next allocation that would exceed this limit\n\
         will evict pages from the front of the list to free up space.",
        format!("{} KiB", to_kibi_bytes(pages.max_size())),
    );
    row(
        "Viewport",
        "Current viewport anchoring mode.",
        crate::keys::tag_name(&pages.viewport),
    );
    row(
        "Tracked Pins",
        "Number of pins tracked for automatic updates.",
        pages.count_tracked_pins().to_string(),
    );
}

fn scrollbar_info(ui: &Ui, pages: &mut PageList) {
    let scrollbar = pages.scrollbar();

    // If we have a scrollbar, show it.
    if scrollbar.total > 0 {
        let delta_row = scrollbar_widget(ui, &scrollbar);
        if delta_row != 0 {
            pages.scroll(Scroll::DeltaRow(delta_row));
        }
    }

    let Some(_t) = begin_table(ui, "scrollbar_info", 3) else {
        return;
    };

    let row = |name: &str, help: &str, value: usize| {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text(name);
        ui.table_set_column_index(1);
        help_marker(ui, help);
        ui.table_set_column_index(2);
        ui.text(value.to_string());
    };
    row(
        "Total",
        "Total number of scrollable rows including scrollback and active area.",
        scrollbar.total,
    );
    row(
        "Offset",
        "Current scroll position as row offset from the top of scrollback.",
        scrollbar.offset,
    );
    row(
        "Length",
        "Number of rows visible in the viewport.",
        scrollbar.len,
    );
}

fn regions_table(ui: &Ui, pages: &PageList) {
    let Some(_t) = begin_table(ui, "pagelist_regions", 4) else {
        return;
    };

    page::setup_fixed(ui, "Region");
    page::setup_fixed(ui, "");
    page::setup_fixed(ui, "Top-Left");
    page::setup_stretch(ui, "Bottom-Right");
    ui.table_headers_row();

    for tag in TAGS {
        region_row(ui, pages, tag);
    }
}

fn region_row(ui: &Ui, pages: &PageList, tag: Tag) {
    let tl_pin = pages.get_top_left(tag);
    let br_pin = pages.get_bottom_right(tag);

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text(crate::keys::tag_name(&tag));

    ui.table_set_column_index(1);
    help_marker(ui, region_help_text(tag));

    ui.table_set_column_index(2);
    if let Some(pt) = tl_pin.and_then(|p| pages.point_from_pin(tag, p)) {
        let coord = pt.coord();
        ui.text(format!("({}, {})", coord.x, coord.y));
    } else {
        ui.text_disabled("(n/a)");
    }

    ui.table_set_column_index(3);
    if let Some(br) = br_pin {
        if let Some(pt) = pages.point_from_pin(tag, br) {
            let coord = pt.coord();
            ui.text(format!("({}, {})", coord.x, coord.y));
        } else {
            ui.text_disabled("(n/a)");
        }
    } else {
        ui.text_disabled("(empty)");
    }
}

fn region_help_text(tag: Tag) -> &'static str {
    match tag {
        Tag::Active => {
            "The active area where a running program can jump the cursor \
             and make changes. This is the 'editable' part of the screen. \
             Bottom-right includes the full height of the screen, including \
             rows that may not be written yet."
        }
        Tag::Viewport => {
            "The visible viewport. If the user has scrolled, top-left changes. \
             Bottom-right is the last written row from the top-left."
        }
        Tag::Screen => {
            "Top-left is the furthest back in scrollback history. Bottom-right \
             is the last written row. Unlike 'active', this only contains \
             written rows."
        }
        Tag::History => {
            "Same top-left as 'screen' but bottom-right is the line just before \
             the top of 'active'. Contains only the scrollback history."
        }
    }
}

fn tracked_pins_table(ui: &Ui, pages: &PageList) {
    let Some(_t) = ui.begin_table_with_flags(
        "tracked_pins",
        5,
        dear_imgui_rs::TableOptions::default()
            .flags(TableFlags::BORDERS | TableFlags::ROW_BG)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
    ) else {
        return;
    };

    page::setup_fixed(ui, "Index");
    page::setup_fixed(ui, "Pin");
    page::setup_fixed(ui, "Context");
    page::setup_fixed(ui, "Dirty");
    page::setup_fixed(ui, "State");
    ui.table_headers_row();

    let active_pin = pages.get_top_left(Tag::Active);
    let viewport_pin = pages.get_top_left(Tag::Viewport);
    let viewport_handle = pages.viewport_pin_handle();

    for (idx, (tracked, pin)) in pages.tracked_pins().enumerate() {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text(idx.to_string());

        ui.table_set_column_index(1);
        if pin.garbage {
            ui.text_colored([1.0, 0.5, 0.3, 1.0], format!("({}, {})", pin.x, pin.y));
        } else {
            ui.text(format!("({}, {})", pin.x, pin.y));
        }

        ui.table_set_column_index(2);
        if let Some(pt) = pages.point_from_pin(Tag::Screen, pin) {
            let coord = pt.coord();
            ui.text(format!("screen ({}, {})", coord.x, coord.y));
        } else {
            ui.text_disabled("screen (out of range)");
        }

        ui.table_set_column_index(3);
        if pin.is_dirty(pages) {
            ui.text_colored([1.0, 0.4, 0.4, 1.0], "dirty");
        } else {
            ui.text_disabled("clean");
        }

        ui.table_set_column_index(4);
        if active_pin.is_some_and(|p| pin.eql(p)) {
            ui.text_colored([0.4, 0.9, 0.4, 1.0], "active top");
        } else if viewport_pin.is_some_and(|p| pin.eql(p)) {
            ui.text_colored([0.4, 0.8, 1.0, 1.0], "viewport top");
        } else if pin.garbage {
            ui.text_colored([1.0, 0.5, 0.3, 1.0], "garbage");
        } else if viewport_handle == Some(tracked) {
            ui.text("viewport pin");
        } else {
            ui.text_disabled("tracked");
        }
    }
}

/// Draws the scrollbar; returns the number of rows the user dragged.
fn scrollbar_widget(ui: &Ui, scrollbar: &Scrollbar) -> isize {
    let mut delta_row: isize = 0;

    let avail_width = ui.content_region_avail()[0];
    let bar_height = ui.frame_height();
    let cursor_pos = ui.cursor_screen_pos();

    let total_f = scrollbar.total as f32;
    let offset_f = scrollbar.offset as f32;
    let len_f = scrollbar.len as f32;

    let grab_start = (offset_f / total_f) * avail_width;
    let grab_width = ((len_f / total_f) * avail_width).max(4.0);

    let draw_list = ui.get_window_draw_list();
    let bg_color = ui.get_color_u32(StyleColor::ScrollbarBg);
    let grab_color = ui.get_color_u32(StyleColor::ScrollbarGrab);

    draw_list
        .add_rect(
            cursor_pos,
            [cursor_pos[0] + avail_width, cursor_pos[1] + bar_height],
            bg_color,
        )
        .filled(true)
        .build();

    draw_list
        .add_rect(
            [cursor_pos[0] + grab_start, cursor_pos[1]],
            [
                cursor_pos[0] + grab_start + grab_width,
                cursor_pos[1] + bar_height,
            ],
            grab_color,
        )
        .filled(true)
        .build();
    ui.invisible_button("scrollbar_drag", [avail_width, bar_height]);
    if ui.is_item_active() {
        let drag_delta = ui.mouse_drag_delta_with_threshold(MouseButton::Left, 0.0);
        if drag_delta[0] != 0.0 {
            let row_delta = (drag_delta[0] / avail_width) * total_f;
            delta_row = row_delta as isize;
            ui.reset_mouse_drag_delta(MouseButton::Left);
        }
    }

    if ui.is_item_hovered_with_flags(ItemHoveredFlags::DELAY_SHORT) {
        ui.set_tooltip(format!(
            "offset={} len={} total={}",
            scrollbar.offset, scrollbar.len, scrollbar.total
        ));
    }

    delta_row
}

/// Grid inspector widget for choosing and inspecting a specific cell.
#[derive(Debug)]
pub struct CellChooser {
    pub lookup_region: Tag,
    pub lookup_coord: Coordinate,
    pub cell_info: CellInfo,
}

impl Default for CellChooser {
    fn default() -> Self {
        Self {
            lookup_region: Tag::Viewport,
            lookup_coord: Coordinate { x: 0, y: 0 },
            cell_info: CellInfo,
        }
    }
}

impl CellChooser {
    pub fn draw(&mut self, ui: &Ui, pages: &PageList) {
        ui.text_wrapped(
            "Inspect a cell by choosing a coordinate space and entering the X/Y position. \
             The inspector resolves the point into the page list and displays the cell contents.",
        );

        ui.separator_with_text("Cell Inspector");

        let region_max = max_coord(pages, self.lookup_region);
        if let Some(coord) = region_max {
            self.lookup_coord.x = self.lookup_coord.x.min(coord.x);
            self.lookup_coord.y = self.lookup_coord.y.min(coord.y);
        } else {
            self.lookup_coord = Coordinate { x: 0, y: 0 };
        }

        {
            let disabled = region_max.is_none();
            let _disabled = ui.begin_disabled_with_cond(disabled);

            let preview = crate::keys::tag_name(&self.lookup_region);
            let combo_width = TAGS
                .iter()
                .map(|t| crate::keys::tag_name(t).len())
                .max()
                .unwrap_or(0)
                + 4;
            ui.set_next_item_width(ui.calc_text_size("X".repeat(combo_width))[0]);
            if let Some(_combo) = ui.begin_combo_with_flags(
                "##grid_region",
                preview,
                ComboBoxOptions::new().height(ComboBoxHeight::Small),
            ) {
                for tag in TAGS {
                    let selected = tag == self.lookup_region;
                    if ui
                        .selectable_config(crate::keys::tag_name(&tag))
                        .selected(selected)
                        .build()
                    {
                        self.lookup_region = tag;
                    }
                    if selected {
                        ui.set_item_default_focus();
                    }
                }
            }

            ui.same_line();

            let width = ui.calc_text_size("00000")[0];
            let mut x_value: u16 = self.lookup_coord.x;
            let mut y_value: u32 = self.lookup_coord.y;
            let mut changed = false;

            ui.align_text_to_frame_padding();
            ui.text("x:");
            ui.same_line();
            ui.set_next_item_width(width);
            if ui.input_scalar("##grid_x", &mut x_value).build() {
                changed = true;
            }

            ui.same_line();
            ui.align_text_to_frame_padding();
            ui.text("y:");
            ui.same_line();
            ui.set_next_item_width(width);
            if ui.input_scalar("##grid_y", &mut y_value).build() {
                changed = true;
            }

            ui.same_line();
            help_marker(
                ui,
                "Choose the coordinate space and X/Y position (0-indexed).",
            );

            if changed && let Some(coord) = region_max {
                self.lookup_coord.x = x_value.min(coord.x);
                self.lookup_coord.y = y_value.min(coord.y);
            }
        }

        if let Some(coord) = region_max {
            ui.text_disabled(format!("Range: x 0..{}, y 0..{}", coord.x, coord.y));
        } else {
            ui.text_disabled("(region has no rows)");
            return;
        }

        let pt = Point::new(self.lookup_region, self.lookup_coord);

        let Some(cell) = lookup_cell(pages, pt) else {
            ui.text_disabled("(cell out of range)");
            return;
        };

        CellInfo::draw(ui, &cell, pt);

        if cell.cell.style_id() != DEFAULT_ID {
            ui.separator_with_text("Style");
            let st = cell.page().style(cell.cell);
            style::table(ui, &st, None);
        }

        if cell.cell.hyperlink() {
            ui.separator_with_text("Hyperlink");
            hyperlink_table(ui, &cell);
        }

        if cell.cell.has_grapheme() {
            ui.separator_with_text("Grapheme");
            grapheme_table(ui, &cell);
        }
    }
}

/// The resolved cell of a point (Zig: `PageList.Cell`).
struct CellRef<'a> {
    page: &'a ghostty_terminal::page::Page,
    row: ghostty_terminal::page::Row,
    cell: ghostty_terminal::page::Cell,
    row_idx: usize,
    col_idx: usize,
}

impl CellRef<'_> {
    fn page(&self) -> &ghostty_terminal::page::Page {
        self.page
    }

    fn cell_index(&self) -> usize {
        self.page
            .cell_index(self.col_idx, self.row_idx)
            .unwrap_or(0)
    }
}

fn lookup_cell(pages: &PageList, pt: Point) -> Option<CellRef<'_>> {
    let pin = pages.pin(pt)?;
    let page = &pages.node(pin.node)?.data;
    let (row, cell) = page.get_row_and_cell(usize::from(pin.x), usize::from(pin.y))?;
    Some(CellRef {
        page,
        row: *row,
        cell: *cell,
        row_idx: usize::from(pin.y),
        col_idx: usize::from(pin.x),
    })
}

fn max_coord(pages: &PageList, tag: Tag) -> Option<Coordinate> {
    let br_pin = pages.get_bottom_right(tag)?;
    let br_point = pages.point_from_pin(tag, br_pin)?;
    Some(br_point.coord())
}

fn hyperlink_table(ui: &Ui, cell: &CellRef<'_>) {
    let Some(_t) = ui.begin_table_with_flags("cell_hyperlink", 2, TableFlags::NONE) else {
        return;
    };

    let page = cell.page();
    let Some(link_id) = page.lookup_hyperlink(cell.cell_index()) else {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Status");
        ui.table_set_column_index(1);
        ui.text_disabled("(missing link data)");
        return;
    };

    let Some(entry) = page.hyperlink_set.get(link_id) else {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Status");
        ui.table_set_column_index(1);
        ui.text_disabled("(missing link data)");
        return;
    };

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("ID");
    ui.table_set_column_index(1);
    match &entry.id {
        ghostty_terminal::hyperlink::HyperlinkId::Implicit(value) => {
            ui.text(format!("implicit {value}"));
        }
        ghostty_terminal::hyperlink::HyperlinkId::Explicit(id) => {
            if id.is_empty() {
                ui.text_disabled("(empty)");
            } else {
                ui.text(String::from_utf8_lossy(id));
            }
        }
    }

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("URI");
    ui.table_set_column_index(1);
    if entry.uri.is_empty() {
        ui.text_disabled("(empty)");
    } else {
        ui.text(String::from_utf8_lossy(&entry.uri));
    }

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Ref Count");
    ui.table_set_column_index(1);
    ui.text(page.hyperlink_set.ref_count(link_id).to_string());
}

fn grapheme_table(ui: &Ui, cell: &CellRef<'_>) {
    let Some(_t) = ui.begin_table_with_flags("cell_grapheme", 2, TableFlags::NONE) else {
        return;
    };

    let page = cell.page();
    let Some(cps) = page.lookup_grapheme(cell.cell_index()) else {
        ui.table_next_row();
        ui.table_set_column_index(0);
        ui.text("Status");
        ui.table_set_column_index(1);
        ui.text_disabled("(missing grapheme data)");
        return;
    };

    ui.table_next_row();
    ui.table_set_column_index(0);
    ui.text("Extra Codepoints");
    ui.table_set_column_index(1);
    if cps.is_empty() {
        ui.text_disabled("(none)");
        return;
    }

    if let Some(_list) = ui.list_box_config("##grapheme_list").begin(ui) {
        for cp in cps {
            ui.selectable(format!("U+{cp:X}"));
        }
    }
}

/// Cell inspector widget.
#[derive(Debug, Default)]
pub struct CellInfo;

impl CellInfo {
    fn draw(ui: &Ui, cell: &CellRef<'_>, point: Point) {
        let Some(_t) = begin_table(ui, "cell_info", 3) else {
            return;
        };

        let row3 = |name: &str, help: &str, f: &dyn Fn()| {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text(name);
            ui.table_set_column_index(1);
            help_marker(ui, help);
            ui.table_set_column_index(2);
            f();
        };

        row3(
            "Grid Position",
            "The cell's X/Y coordinates in the selected region.",
            &|| {
                let coord = point.coord();
                ui.text(format!("({}, {})", coord.x, coord.y));
            },
        );

        row3(
            "Page Location",
            "Row and column indices within the backing page.",
            &|| ui.text(format!("row={} col={}", cell.row_idx, cell.col_idx)),
        );

        row3(
            "Content",
            "Content tag describing how the cell data is stored.",
            &|| ui.text(crate::keys::tag_name(&cell.cell.content_tag())),
        );

        row3(
            "Codepoint",
            "Primary Unicode codepoint for the cell.",
            &|| {
                let cp = cell.cell.codepoint();
                if cp == 0 {
                    ui.text_disabled("(empty)");
                } else {
                    ui.text(format!("U+{cp:04X}"));
                }
            },
        );

        if cell.cell.has_grapheme() {
            row3(
                "Grapheme",
                "Extra codepoints that combine with the primary codepoint to form the grapheme cluster.",
                &|| {
                    if let Some(_list) = ui.list_box_config("##cell_grapheme").begin(ui) {
                        if let Some(cps) = cell.page().lookup_grapheme(cell.cell_index()) {
                            for cp in cps {
                                ui.selectable(format!("U+{cp:X}"));
                            }
                        } else {
                            ui.selectable("(missing)");
                        }
                    }
                },
            );
        }

        row3(
            "Width Property",
            "Character width property (narrow, wide, spacer, etc.).",
            &|| ui.text(crate::keys::tag_name(&cell.cell.wide())),
        );

        row3(
            "Row Flags",
            "Flags set on the row containing this cell.",
            &|| {
                let row = cell.row;
                if row.wrap()
                    || row.wrap_continuation()
                    || row.grapheme()
                    || row.styled()
                    || row.hyperlink()
                {
                    if row.wrap() {
                        ui.text_colored([0.4, 0.8, 1.0, 1.0], "wrap");
                        ui.same_line();
                    }
                    if row.wrap_continuation() {
                        ui.text_colored([0.4, 0.8, 1.0, 1.0], "cont");
                        ui.same_line();
                    }
                    if row.grapheme() {
                        ui.text_colored([0.9, 0.7, 0.3, 1.0], "grapheme");
                        ui.same_line();
                    }
                    if row.styled() {
                        ui.text_colored([0.7, 0.9, 0.5, 1.0], "styled");
                        ui.same_line();
                    }
                    if row.hyperlink() {
                        ui.text_colored([0.8, 0.6, 1.0, 1.0], "link");
                        ui.same_line();
                    }
                } else {
                    ui.text_disabled("(none)");
                }
            },
        );

        row3(
            "Style ID",
            "Internal style reference ID for this cell.",
            &|| ui.text(cell.cell.style_id().to_string()),
        );

        row3(
            "Style",
            "Resolved style for the cell (colors, attributes, etc.).",
            &|| {
                if cell.cell.style_id() == DEFAULT_ID {
                    ui.text_disabled("(default)");
                } else {
                    ui.text_disabled("(see below)");
                }
            },
        );

        if cell.cell.hyperlink() {
            row3(
                "Hyperlink",
                "OSC8 hyperlink ID associated with this cell.",
                &|| {
                    let link_id = cell.page().lookup_hyperlink(cell.cell_index()).unwrap_or(0);
                    ui.text(format!("id={link_id}"));
                },
            );
        }
    }
}
