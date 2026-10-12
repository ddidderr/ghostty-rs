//! Render state: a snapshot of the terminal state required to render a frame.
//!
//! Port of `ZIG/src/terminal/render.zig`.
//!
//! Developer note: this is in the terminal crate and not the renderer crate
//! because the goal is that this remains generic to multiple renderers.
//!
//! Contains the state required to render the screen, including optimizing
//! for repeated render calls and only rendering dirty regions.
//!
//! Previously, our renderer would use `clone` to clone the screen within
//! the viewport to perform rendering. This worked well enough that we kept
//! it all the way up through the Ghostty 1.2.x series, but the clone time
//! was repeatedly a bottleneck blocking IO.
//!
//! Rather than a generic clone that tries to clone all screen state per call
//! (within a region), a stateful approach that optimizes for only what a
//! renderer needs to do makes more sense.
//!
//! To use this, initialize the render state to empty, then call `update`
//! on each frame to update the state to the latest terminal state.
//!
//! ```
//! # use ghostty_terminal::{render::RenderState, terminal::{Terminal, Options}};
//! # let mut terminal = Terminal::new(Options::new(80, 24))?;
//! let mut state = RenderState::new();
//! state.update(&mut terminal)?;
//! # Ok::<(), ghostty_terminal::page::Error>(())
//! ```
//!
//! Note: the render state retains as much memory as possible between updates
//! to prevent future allocations. If a very large frame is rendered once,
//! the render state will retain that much memory until dropped. To avoid
//! waste, it is recommended that the caller drop and start with an
//! empty render state every so often.

use std::collections::HashSet;

use ghostty_vt::{
    color::{self, Palette, Rgb},
    modes::Mode,
};

use crate::{
    cursor,
    highlight::Flattened,
    page::{self, ContentTag, Error, Wide},
    page_list::{Direction, PageList, Pin},
    point::{Coordinate, Point, Tag},
    screen_set::Key as ScreenKey,
    selection::Selection,
    size,
    style::{Color, Style},
    terminal::{Dirty as TerminalDirty, Terminal},
};

/// Contains the state required to render the screen. See the module docs.
#[derive(Debug, Clone)]
pub struct RenderState {
    /// The current screen dimensions. It is possible that these don't match
    /// the renderer's current dimensions in grid cells because resizing
    /// can happen asynchronously. This can lead to a one or two frame
    /// mismatch a renderer needs to handle.
    ///
    /// The viewport is always exactly equal to the active area size so this
    /// is also the viewport size.
    pub rows: size::CellCountInt,
    pub cols: size::CellCountInt,

    /// The color state for the terminal.
    pub colors: Colors,

    /// Cursor state within the viewport.
    pub cursor: Cursor,

    /// The rows (y=0 is top) of the viewport. Guaranteed to be `rows` length.
    pub row_data: Vec<Row>,

    /// The dirty state of the render state. This is set by the update method.
    /// The renderer/caller should set this to false when it has handled
    /// the dirty state.
    pub dirty: Dirty,

    /// The screen type that this state represents. This is used primarily
    /// to detect changes.
    pub screen: ScreenKey,

    /// The last viewport pin used to generate this state. This is NOT
    /// a tracked pin and is generally NOT safe to read other than the direct
    /// values for comparison.
    pub viewport_pin: Option<Pin>,

    /// The cached selection so we can avoid expensive selection calculations
    /// if possible.
    selection_cache: Option<SelectionCache>,
}

/// The color state for the terminal.
///
/// The background/foreground will be reversed if the terminal reverse
/// color mode is on! You do not need to handle that manually!
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: Palette,
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    /// The x/y position of the cursor within the active area.
    pub active: Coordinate,

    /// The x/y position of the cursor within the viewport. This
    /// may be None if the cursor is not visible within the viewport.
    pub viewport: Option<CursorViewport>,

    /// The cell data for the cursor position. Managed memory is not
    /// safe to access from this.
    pub cell: page::Cell,

    /// The style, always valid even if the cell is default style.
    pub style: Style,

    /// The visual style of the cursor itself, such as a block or bar.
    pub visual_style: cursor::Style,

    /// True if the cursor is detected to be at a password input field.
    pub password_input: bool,

    /// Cursor visibility state determined by the terminal mode.
    pub visible: bool,

    /// Cursor blink state determined by the terminal mode.
    pub blinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorViewport {
    /// The x/y position of the cursor within the viewport.
    pub x: size::CellCountInt,
    pub y: size::CellCountInt,

    /// Whether the cursor is part of a wide character and
    /// on the tail of it. If so, some renderers may use this
    /// to move the cursor back one.
    pub wide_tail: bool,
}

/// A row within the viewport.
#[derive(Debug, Clone)]
pub struct Row {
    /// The page pin. This is not safe to read unless you can guarantee
    /// the terminal state hasn't changed since the last `update` call.
    pub pin: Pin,

    /// Raw row data.
    pub raw: page::Row,

    /// The cells in this row. Guaranteed to be `cols` length.
    pub cells: Cells,

    /// A dirty flag that can be used by the renderer to track
    /// its own draw state. `update` will mark this true whenever
    /// this row is changed, too.
    pub dirty: bool,

    /// The x range of the selection within this row.
    pub selection: Option<[size::CellCountInt; 2]>,

    /// The highlights within this row.
    pub highlights: Vec<Highlight>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    /// A special tag that can be used by the caller to differentiate
    /// different highlight types. The value is opaque to the RenderState.
    pub tag: u8,

    /// The x ranges of highlights within this row.
    pub range: [size::CellCountInt; 2],
}

/// One cell of a row (the Zig `RenderState.Cell`), by reference.
#[derive(Debug, Clone, Copy)]
pub struct Cell<'a> {
    /// Always set, this is the raw copied cell data from page.Cell.
    /// The managed memory (hyperlinks, graphemes, etc.) is NOT safe
    /// to access from here. It is duplicated into the other fields if
    /// it exists.
    pub raw: page::Cell,

    /// Grapheme data for the cell (the codepoints after the first). Empty
    /// unless the raw cell's content_tag is `CodepointGrapheme`.
    pub grapheme: &'a [u32],

    /// The style data for the cell. The default style unless the style_id
    /// is non-default on raw (or the cell is a bg-color-only cell).
    pub style: Style,
}

/// The cells of a row, stored as a struct of arrays (the Zig
/// `MultiArrayList(Cell)`) so that the common plain-text case only copies the
/// raw cells.
///
/// `raw` always has exactly `cols` entries. `grapheme` and `style_ids` either
/// have exactly `cols` entries, or are empty when the source row has no
/// managed memory (in which case every cell has no grapheme and the default
/// style). Styles are stored sparsely: `style_ids[x]` is 0 for the default
/// style, otherwise `styles[style_ids[x] - 1]`. Always use the accessors
/// rather than indexing those directly.
#[derive(Debug, Clone, Default)]
pub struct Cells {
    pub raw: Vec<page::Cell>,
    pub grapheme: Vec<Vec<u32>>,
    pub style_ids: Vec<u16>,
    pub styles: Vec<Style>,
}

impl Cells {
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The grapheme (extra codepoints) of the cell at `x`.
    pub fn grapheme(&self, x: usize) -> &[u32] {
        self.grapheme.get(x).map_or(&[], Vec::as_slice)
    }

    /// The style of the cell at `x` (default style if none).
    pub fn style(&self, x: usize) -> Style {
        match self.style_ids.get(x) {
            Some(&id) if id > 0 => self.styles[usize::from(id) - 1],
            _ => Style::default(),
        }
    }

    /// The full cell at `x`. Panics if out of range.
    pub fn get(&self, x: usize) -> Cell<'_> {
        Cell {
            raw: self.raw[x],
            grapheme: self.grapheme(x),
            style: self.style(x),
        }
    }

    /// Iterate all cells in order.
    pub fn iter(&self) -> impl Iterator<Item = Cell<'_>> + '_ {
        (0..self.raw.len()).map(|x| self.get(x))
    }

    fn clear(&mut self) {
        self.raw.clear();
        self.grapheme.clear();
        self.style_ids.clear();
        self.styles.clear();
    }
}

/// Dirty state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dirty {
    /// Not dirty at all. Can skip rendering if prior state was
    /// already rendered.
    #[default]
    False,

    /// Partially dirty. Some rows changed but not all. None of the
    /// global state changed such as colors.
    Partial,

    /// Fully dirty. Global state changed or dimensions changed. All rows
    /// should be redrawn.
    Full,
}

#[derive(Debug, Clone, Copy)]
struct SelectionCache {
    selection: Selection,
    tl_pin: Pin,
    br_pin: Pin,
}

impl Default for RenderState {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderState {
    /// Initial (empty) state.
    pub fn new() -> RenderState {
        RenderState {
            rows: 0,
            cols: 0,
            colors: Colors {
                background: Rgb::default(),
                foreground: Rgb::default(),
                cursor: None,
                palette: color::DEFAULT,
            },
            cursor: Cursor {
                active: Coordinate { x: 0, y: 0 },
                viewport: None,
                cell: page::Cell::default(),
                style: Style::default(),
                visual_style: cursor::Style::Block,
                password_input: false,
                visible: true,
                blinking: false,
            },
            row_data: Vec::new(),
            dirty: Dirty::False,
            screen: ScreenKey::Primary,
            viewport_pin: None,
            selection_cache: None,
        }
    }

    /// Update the render state to the latest terminal state.
    ///
    /// This will reset the terminal dirty state since it is consumed
    /// by this render state update.
    pub fn update(&mut self, t: &mut Terminal) -> Result<(), Error> {
        let active_key = t.screens.active_key;
        let terminal_dirty = t.flags.dirty != TerminalDirty::default();
        let s = t.screens.active_mut();
        let viewport_pin = s
            .pages
            .get_top_left(Tag::Viewport)
            .ok_or(Error::InvalidCoordinate)?;
        let redraw = 'redraw: {
            // If our screen key changed, we need to do a full rebuild
            // because our render state is viewport-specific.
            if active_key != self.screen {
                break 'redraw true;
            }

            // If our terminal is dirty at all, we do a full rebuild. These
            // dirty values are full-terminal dirty values.
            if terminal_dirty {
                break 'redraw true;
            }

            // If our screen is dirty at all, we do a full rebuild. This is
            // a full screen dirty tracker.
            if s.dirty != crate::screen::Dirty::default() {
                break 'redraw true;
            }

            // If our dimensions changed, we do a full rebuild.
            if self.rows != s.pages.rows || self.cols != s.pages.cols {
                break 'redraw true;
            }

            // If our viewport pin changed, we do a full rebuild.
            if let Some(old) = self.viewport_pin
                && !old.eql(viewport_pin)
            {
                break 'redraw true;
            }

            false
        };

        // Always set our cheap fields, its more expensive to compare
        self.rows = s.pages.rows;
        self.cols = s.pages.cols;
        self.viewport_pin = Some(viewport_pin);
        self.cursor.active = Coordinate {
            x: s.cursor.x,
            y: u32::from(s.cursor.y),
        };
        self.cursor.cell = s.cursor_cell();
        self.cursor.style = s.cursor.style;
        self.cursor.visual_style = s.cursor.cursor_style;
        self.cursor.password_input = t.flags.password_input;
        self.cursor.visible = t.modes.get(Mode::CursorVisible);
        self.cursor.blinking = t.modes.get(Mode::CursorBlinking);

        // Always reset the cursor viewport position. In the future we can
        // probably cache this by comparing the cursor pin and viewport pin
        // but may not be worth it.
        self.cursor.viewport = None;

        // Colors.
        self.colors.cursor = t.colors.cursor.get();
        self.colors.palette = t.colors.palette.current;
        // Background/foreground can be unset initially which would
        // depend on "default" background/foreground. The expected use
        // case of Terminal is that the caller set their own configured
        // defaults on load so this doesn't happen.
        if let (Some(bg), Some(fg)) = (t.colors.background.get(), t.colors.foreground.get()) {
            if t.modes.get(Mode::ReverseColors) {
                self.colors.background = fg;
                self.colors.foreground = bg;
            } else {
                self.colors.background = bg;
                self.colors.foreground = fg;
            }
        }

        // Ensure our row length is exactly our height, freeing or allocating
        // data as necessary. In most cases we'll have a perfectly matching
        // size.
        let nrows = usize::from(self.rows);
        if self.row_data.len() != nrows {
            if self.row_data.len() < nrows {
                let filler = viewport_pin;
                self.row_data.resize_with(nrows, || Row {
                    pin: filler,
                    raw: page::Row::default(),
                    cells: Cells::default(),
                    dirty: true,
                    selection: None,
                    highlights: Vec::new(),
                });
            } else {
                self.row_data.truncate(nrows);
            }
        }

        // Go through and set up our row pins and find the cursor. We do the
        // cursor even if the row is not dirty because the cursor is unrelated.
        // We have to store pins even if we're not dirty because dirty is
        // only a renderer optimization. It doesn't apply to memory movement.
        // This will let us remap any cell pins back to an exact entry in our
        // RenderState.
        let cursor_pin = s.cursor_pin();
        let mut y: usize = 0;
        for row_pin in s.pages.row_iterator(
            Direction::RightDown,
            Point::Viewport(Coordinate::default()),
            None,
        ) {
            if y >= nrows {
                break;
            }
            if self.cursor.viewport.is_none()
                && row_pin.node == cursor_pin.node
                && row_pin.y == cursor_pin.y
            {
                self.cursor.viewport = Some(CursorViewport {
                    y: y as size::CellCountInt,
                    x: s.cursor.x,

                    // Future: we should use our own state here to look this
                    // up rather than calling this.
                    wide_tail: if s.cursor.x > 0 {
                        s.cursor_cell_left(1).wide() == Wide::Wide
                    } else {
                        false
                    },
                });
            }
            self.row_data[y].pin = row_pin;
            y += 1;
        }
        assert_eq!(y, nrows);

        // Track the last page that we know was dirty. This lets us
        // more quickly do the full-page dirty check.
        let mut last_dirty_page: Option<crate::page_list::NodeId> = None;
        let mut any_dirty = false;
        let cols = usize::from(self.cols);

        for y in 0..nrows {
            let row_pin = self.row_data[y].pin;
            let node = s
                .pages
                .node_mut(row_pin.node)
                .ok_or(Error::InvalidCoordinate)?;

            let is_dirty = 'dirty: {
                // If we're redrawing then we're definitely dirty.
                if redraw {
                    break 'dirty true;
                }

                // If our page is the same as last time then its dirty.
                if last_dirty_page == Some(row_pin.node) {
                    break 'dirty true;
                }
                if node.data.dirty {
                    // If this page is dirty then clear the dirty flag
                    // of the last page and then store this one. This benchmarks
                    // faster than iterating pages again later.
                    if let Some(last) = last_dirty_page
                        && let Some(n) = s.pages.node_mut(last)
                    {
                        n.data.dirty = false;
                    }
                    last_dirty_page = Some(row_pin.node);
                    break 'dirty true;
                }

                // If our row is dirty then we're dirty.
                node.data
                    .get_row(usize::from(row_pin.y))
                    .is_some_and(|r| r.dirty())
            };
            if !is_dirty {
                continue;
            }

            // Set that at least one row was dirty.
            any_dirty = true;

            // Clear our row dirty, we'll clear our page dirty later.
            // We can't clear it now because we have more rows to go through.
            let node = s
                .pages
                .node_mut(row_pin.node)
                .ok_or(Error::InvalidCoordinate)?;
            let raw_row = {
                let r = node
                    .data
                    .get_row_mut(usize::from(row_pin.y))
                    .ok_or(Error::InvalidCoordinate)?;
                r.set_dirty(false);
                *r
            };
            let p: &page::Page = &node.data;

            let row = &mut self.row_data[y];

            // Reset our cells if we're rebuilding this row.
            if !row.cells.is_empty() {
                row.cells.clear();
                row.selection = None;
                row.highlights.clear();
            }
            row.dirty = true;

            // Get all our cells in the page.
            let page_cells = p
                .get_cells(usize::from(row_pin.y))
                .ok_or(Error::InvalidCoordinate)?;
            assert_eq!(page_cells.len(), cols);

            // Copy our raw row data
            row.raw = raw_row;

            // We always copy our raw cell data. In the case we have no
            // managed memory, we can skip setting any other fields.
            //
            // This is an important optimization. For plain-text screens
            // this ends up being something around 300% faster based on
            // the `screen-clone` benchmark.
            row.cells.raw.extend_from_slice(page_cells);
            if !raw_row.managed_memory() {
                continue;
            }

            row.cells.style_ids.resize(cols, 0);
            // Cache of the last page style id and its index in `styles`.
            let mut last_sid: u16 = 0;
            let mut last_idx: u16 = 0;
            let mut last_bg_key: u32 = 0;
            let mut last_bg_idx: u16 = 0;
            for (x, page_cell) in page_cells.iter().enumerate() {
                // Plain cells (the vast majority) need nothing more.
                if page_cell.is_plain() {
                    continue;
                }

                // Append assuming its a single-codepoint, styled cell
                // (most common by far).
                let sid = page_cell.style_id();
                if sid > 0 {
                    if sid != last_sid {
                        row.cells.styles.push(p.style(*page_cell));
                        last_sid = sid;
                        last_idx = row.cells.styles.len() as u16;
                    }
                    row.cells.style_ids[x] = last_idx;
                }

                // Switch on our content tag to handle less likely cases.
                match page_cell.content_tag() {
                    // Primary codepoint goes into `raw` field.
                    ContentTag::Codepoint => {}

                    // If we have a multi-codepoint grapheme, look it up and
                    // set our content type.
                    ContentTag::CodepointGrapheme => {
                        if let Ok(idx) = p.cell_index(x, usize::from(row_pin.y))
                            && let Some(g) = p.lookup_grapheme(idx)
                        {
                            // Only rows that actually contain graphemes pay
                            // for the per-cell vectors.
                            if row.cells.grapheme.is_empty() {
                                row.cells.grapheme.resize_with(cols, Vec::new);
                            }
                            row.cells.grapheme[x].extend_from_slice(g);
                        }
                    }

                    ContentTag::BgColorRgb => {
                        let c = page_cell.color_rgb();
                        let key =
                            1 << 24 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b);
                        if key != last_bg_key {
                            row.cells.styles.push(Style {
                                bg_color: Color::Rgb(c),
                                ..Style::default()
                            });
                            last_bg_key = key;
                            last_bg_idx = row.cells.styles.len() as u16;
                        }
                        row.cells.style_ids[x] = last_bg_idx;
                    }

                    ContentTag::BgColorPalette => {
                        let c = page_cell.color_palette();
                        let key = 2 << 24 | u32::from(c);
                        if key != last_bg_key {
                            row.cells.styles.push(Style {
                                bg_color: Color::Palette(c),
                                ..Style::default()
                            });
                            last_bg_key = key;
                            last_bg_idx = row.cells.styles.len() as u16;
                        }
                        row.cells.style_ids[x] = last_bg_idx;
                    }
                }
            }
        }

        // If our screen has a selection, then mark the rows with the
        // selection. We do this outside of the loop above because its unlikely
        // a selection exists and because the way our selections are structured
        // today is very inefficient.
        //
        // NOTE: To improve the performance of the block below, we'll need
        // to rethink how we model selections in general.
        //
        // There are performance improvements that can be made here, though.
        // For example, `contained_row` recalculates a bunch of information
        // we can cache.
        if let Some(sel) = s.selection {
            'selection: {
                // Populate our selection cache to avoid some expensive
                // recalculation.
                let cached = match self.selection_cache {
                    // If we're redrawing, we recalculate the cache just to
                    // be safe. If our selection isn't equal, we aren't cached!
                    Some(c) if !redraw && c.selection.eql(&s.pages, &sel) => Some(c),
                    _ => None,
                };
                let cache = if let Some(c) = cached {
                    // If we have no dirty rows, we can not recalculate.
                    if !any_dirty {
                        break 'selection;
                    }

                    // We have dirty rows, we can utilize the cache.
                    c
                } else {
                    // Create a new cache
                    let tl_pin = sel.top_left(&s.pages);
                    let br_pin = sel.bottom_right(&s.pages);
                    let c = SelectionCache {
                        selection: Selection::new(tl_pin, br_pin, sel.rectangle),
                        tl_pin,
                        br_pin,
                    };
                    self.selection_cache = Some(c);
                    c
                };

                // Grab the inefficient data we need from the selection. At
                // least we can cache it.
                let tl = s
                    .pages
                    .point_from_pin(Tag::Screen, cache.tl_pin)
                    .ok_or(Error::InvalidCoordinate)?
                    .coord();
                let br = s
                    .pages
                    .point_from_pin(Tag::Screen, cache.br_pin)
                    .ok_or(Error::InvalidCoordinate)?
                    .coord();

                // We need to determine if our selection is within the viewport.
                // The viewport is generally very small so the efficient way to
                // do this is to traverse the viewport pages and check for the
                // matching selection pages.
                for row in &mut self.row_data {
                    let pin = row.pin;
                    let p = s
                        .pages
                        .point_from_pin(Tag::Screen, pin)
                        .ok_or(Error::InvalidCoordinate)?
                        .coord();
                    let Some(row_sel) = sel.contained_row_cached(
                        &s.pages,
                        cache.tl_pin,
                        cache.br_pin,
                        pin,
                        tl,
                        br,
                        p,
                    ) else {
                        continue;
                    };
                    let start = row_sel.start(&s.pages);
                    let end = row_sel.end(&s.pages);
                    assert_eq!(start.node, end.node);
                    assert!(start.x <= end.x);
                    assert_eq!(start.y, end.y);
                    row.selection = Some([start.x, end.x]);
                }
            }
        }

        // Handle dirty state.
        if redraw {
            // Fully redraw resets some other state.
            self.screen = active_key;
            self.dirty = Dirty::Full;

            // Note: we don't clear any row_data here because our rebuild
            // above did this.
        } else if any_dirty && self.dirty == Dirty::False {
            self.dirty = Dirty::Partial;
        }

        // Finalize our final dirty page
        if let Some(last) = last_dirty_page
            && let Some(n) = s.pages.node_mut(last)
        {
            n.data.dirty = false;
        }

        // Clear our dirty flags
        t.flags.dirty = TerminalDirty::default();
        s.dirty = crate::screen::Dirty::default();
        Ok(())
    }

    /// Update the highlights in the render state from the given flattened
    /// highlights. Because this uses flattened highlights, it does not require
    /// reading from the terminal state so it should be done outside of
    /// any critical sections.
    ///
    /// This will not clear any previous highlights, so the caller must
    /// manually clear them if desired.
    pub fn update_highlights_flattened(&mut self, tag: u8, hls: &[Flattened]) {
        // Fast path, we have no highlights!
        if hls.is_empty() {
            return;
        }

        // This is, admittedly, horrendous. This is some low hanging fruit
        // to optimize. In my defense, screens are usually small, the number
        // of highlights is usually small, and this only happens on the
        // viewport outside of a locked area. Still, I'd love to see this
        // improved someday.

        // We need to track whether any row had a match so we can mark
        // the dirty state.
        let mut any_dirty = false;
        let cols = self.cols;

        for row in &mut self.row_data {
            let row_pin = row.pin;
            for hl in hls {
                let n = hl.chunks.len();
                for (i, chunk) in hl.chunks.iter().enumerate() {
                    // If this node doesn't match or we're not within
                    // the row range, skip it.
                    if chunk.node != row_pin.node
                        || row_pin.y < chunk.start
                        || row_pin.y >= chunk.end
                    {
                        continue;
                    }

                    // We're a match!
                    row.highlights.push(Highlight {
                        tag,
                        range: [
                            if i == 0 && row_pin.y == hl.chunks[0].start {
                                hl.top_x
                            } else {
                                0
                            },
                            if i == n - 1 && row_pin.y == hl.chunks[n - 1].end - 1 {
                                hl.bot_x
                            } else {
                                cols - 1
                            },
                        ],
                    });

                    row.dirty = true;
                    any_dirty = true;
                }
            }
        }

        // Mark our dirty state.
        if any_dirty && self.dirty == Dirty::False {
            self.dirty = Dirty::Partial;
        }
    }

    /// Convert the current render state contents to a UTF-8 encoded
    /// string written to the given string. This will unwrap all the wrapped
    /// rows. This is useful for a minimal viewport search.
    ///
    /// This currently writes empty cell contents as \x00 and writes all
    /// blank lines. This is fine for our current usage (link search) but
    /// we can adjust this later.
    ///
    /// If `map` is given, every byte written gets the coordinate of the cell
    /// it came from appended (a newline maps to `x = cols`).
    ///
    /// NOTE: There is a limitation in that wrapped lines before/after
    /// the the top/bottom line of the viewport are not included, since
    /// the render state cuts them off.
    ///
    /// Returns `Err` if a cell holds an invalid Unicode scalar value.
    pub fn string(
        &self,
        out: &mut String,
        mut map: Option<&mut Vec<Coordinate>>,
    ) -> Result<(), std::fmt::Error> {
        for (y, row) in self.row_data.iter().enumerate() {
            for (x, cell) in row.cells.raw.iter().enumerate() {
                let coord = Coordinate {
                    x: x as u16,
                    y: y as u32,
                };
                let c = char::from_u32(cell.codepoint()).ok_or(std::fmt::Error)?;
                let mut len = c.len_utf8();
                out.push(c);
                if cell.has_grapheme() {
                    for &cp in row.cells.grapheme(x) {
                        let c = char::from_u32(cp).ok_or(std::fmt::Error)?;
                        len += c.len_utf8();
                        out.push(c);
                    }
                }

                if let Some(m) = map.as_deref_mut() {
                    m.extend(std::iter::repeat_n(coord, len));
                }
            }

            if !row.raw.wrap() {
                out.push('\n');
                if let Some(m) = map.as_deref_mut() {
                    m.push(Coordinate {
                        x: row.cells.len() as u16,
                        y: y as u32,
                    });
                }
            }
        }
        Ok(())
    }

    /// Returns a set of the cells that match to an OSC8 hyperlink over the
    /// given point in the render state.
    ///
    /// IMPORTANT: The terminal must not have updated since the last call to
    /// `update`. If there is any chance the terminal has updated, the caller
    /// must first call `update` again to refresh the render state.
    ///
    /// For example, you may want to hold a lock for the duration of the
    /// update and hyperlink lookup to ensure no updates happen in between.
    ///
    /// Unlike Zig (where pins hold page pointers) the page list the pins
    /// belong to must be passed (`&terminal.screen().pages`).
    pub fn link_cells(&self, pages: &PageList, viewport_point: Coordinate) -> HashSet<Coordinate> {
        let mut result = HashSet::new();

        // Our viewport point is sent in by the caller and can't be trusted.
        // If it is outside the valid area then just return empty because
        // we can't possibly have a link there.
        if viewport_point.x >= self.cols || viewport_point.y as usize >= self.row_data.len() {
            return result;
        }

        // Grab our link ID
        let link_pin = self.row_data[viewport_point.y as usize].pin;
        let Some(link_node) = pages.node(link_pin.node) else {
            return result;
        };
        let link_page = &link_node.data;
        let Some(link) = ({
            let x = usize::from(viewport_point.x);
            let y = usize::from(link_pin.y);
            let Some((_, cell)) = link_page.get_row_and_cell(x, y) else {
                return result;
            };

            // The likely scenario is that our mouse isn't even over a link.
            if !cell.hyperlink() {
                return result;
            }

            let Ok(idx) = link_page.cell_index(x, y) else {
                return result;
            };
            let Some(link_id) = link_page.lookup_hyperlink(idx) else {
                return result;
            };
            link_page.hyperlink_set.get(link_id)
        }) else {
            return result;
        };

        for (y, row) in self.row_data.iter().enumerate() {
            let pin = row.pin;
            let Some(other_node) = pages.node(pin.node) else {
                continue;
            };
            let other_page = &other_node.data;
            for (x, cell) in row.cells.raw.iter().enumerate() {
                if !cell.hyperlink() {
                    continue;
                }

                let Ok(idx) = other_page.cell_index(x, usize::from(pin.y)) else {
                    continue;
                };
                let Some(link_id) = other_page.lookup_hyperlink(idx) else {
                    continue;
                };
                let Some(other) = other_page.hyperlink_set.get(link_id) else {
                    continue;
                };

                if link.eql(other) {
                    result.insert(Coordinate {
                        y: y as u32,
                        x: x as u16,
                    });
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Options;

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(Options::new(cols, rows)).unwrap()
    }

    fn feed(t: &mut Terminal, bytes: &[u8]) {
        let mut s = t.vt_stream();
        s.next_slice(bytes).unwrap();
    }

    fn clear_dirty(state: &mut RenderState) {
        state.dirty = Dirty::False;
        for r in &mut state.row_data {
            r.dirty = false;
        }
    }

    #[test]
    fn styled() {
        let mut t = term(80, 24);

        // This fills the screen up
        t.decaln().unwrap();

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
    }

    #[test]
    fn basic_text() {
        let mut t = term(10, 3);
        feed(&mut t, b"ABCD");

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        // Verify we have the right number of rows
        assert_eq!(state.row_data.len(), 3);

        // All rows should have cols cells
        for r in &state.row_data {
            assert_eq!(r.cells.len(), 10);
        }

        // Row zero should contain our text
        let cells = &state.row_data[0].cells;
        assert_eq!(cells.get(0).raw.codepoint(), 'A' as u32);
        assert_eq!(cells.get(1).raw.codepoint(), 'B' as u32);
        assert_eq!(cells.get(2).raw.codepoint(), 'C' as u32);
        assert_eq!(cells.get(3).raw.codepoint(), 'D' as u32);
        assert_eq!(cells.get(4).raw.codepoint(), 0);
    }

    #[test]
    fn styled_text() {
        let mut t = term(10, 3);
        feed(&mut t, b"\x1b[1mA"); // Bold
        feed(&mut t, b"\x1b[0;3mB"); // Italic
        feed(&mut t, b"\x1b[0;4mC"); // Underline

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        assert_eq!(state.row_data.len(), 3);
        for r in &state.row_data {
            assert_eq!(r.cells.len(), 10);
        }

        let cells = &state.row_data[0].cells;
        {
            let cell = cells.get(0);
            assert_eq!(cell.raw.codepoint(), 'A' as u32);
            assert!(cell.style.flags.bold);
        }
        {
            let cell = cells.get(1);
            assert_eq!(cell.raw.codepoint(), 'B' as u32);
            assert!(!cell.style.flags.bold);
            assert!(cell.style.flags.italic);
        }
        assert_eq!(cells.get(2).raw.codepoint(), 'C' as u32);
        assert_eq!(cells.get(3).raw.codepoint(), 0);
    }

    #[test]
    fn grapheme() {
        let mut t = term(10, 3);
        feed(&mut t, b"A");
        feed(&mut t, "\u{1F468}\u{200D}".as_bytes()); // this has a ZWJ

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        assert_eq!(state.row_data.len(), 3);
        for r in &state.row_data {
            assert_eq!(r.cells.len(), 10);
        }

        let cells = &state.row_data[0].cells;
        assert_eq!(cells.get(0).raw.codepoint(), 'A' as u32);
        {
            let cell = cells.get(1);
            assert_eq!(cell.raw.codepoint(), 0x1F468);
            assert_eq!(cell.raw.wide(), Wide::Wide);
            assert_eq!(cell.grapheme, &[0x200D]);
        }
        {
            let cell = cells.get(2);
            assert_eq!(cell.raw.codepoint(), 0);
            assert_eq!(cell.raw.wide(), Wide::SpacerTail);
        }
    }

    #[test]
    fn cursor_state_in_viewport() {
        let mut t = term(10, 5);
        feed(&mut t, b"A\x1b[H");

        let mut state = RenderState::new();

        // Initial update
        state.update(&mut t).unwrap();
        assert_eq!(state.cursor.active.x, 0);
        assert_eq!(state.cursor.active.y, 0);
        assert_eq!(state.cursor.viewport.unwrap().x, 0);
        assert_eq!(state.cursor.viewport.unwrap().y, 0);
        assert_eq!(state.cursor.cell.codepoint(), 'A' as u32);
        assert!(state.cursor.style.is_default());

        // Set a style on the cursor
        feed(&mut t, b"\x1b[1m"); // Bold
        state.update(&mut t).unwrap();
        assert!(!state.cursor.style.is_default());
        assert!(state.cursor.style.flags.bold);
        feed(&mut t, b"\x1b[0m"); // Reset style

        // Move cursor to 2,1
        feed(&mut t, b"\x1b[2;3H");
        state.update(&mut t).unwrap();
        assert_eq!(state.cursor.active.x, 2);
        assert_eq!(state.cursor.active.y, 1);
        assert_eq!(state.cursor.viewport.unwrap().x, 2);
        assert_eq!(state.cursor.viewport.unwrap().y, 1);
    }

    #[test]
    fn cursor_state_out_of_viewport() {
        let mut t = term(10, 2);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\n");

        let mut state = RenderState::new();

        // Initial update
        state.update(&mut t).unwrap();
        assert_eq!(state.cursor.active.x, 0);
        assert_eq!(state.cursor.active.y, 1);
        assert_eq!(state.cursor.viewport.unwrap().x, 0);
        assert_eq!(state.cursor.viewport.unwrap().y, 1);

        // Scroll the viewport
        t.scroll_viewport(crate::terminal::ScrollViewport::Top);
        state.update(&mut t).unwrap();

        // Set a style on the cursor
        assert_eq!(state.cursor.active.x, 0);
        assert_eq!(state.cursor.active.y, 1);
        assert!(state.cursor.viewport.is_none());
    }

    #[test]
    fn dirty_state() {
        let mut t = term(10, 5);

        let mut state = RenderState::new();

        // First update should trigger redraw due to resize
        state.update(&mut t).unwrap();
        assert_eq!(state.dirty, Dirty::Full);

        // Reset dirty flag and dirty rows
        clear_dirty(&mut state);

        // Second update with no changes - no dirty rows
        state.update(&mut t).unwrap();
        assert_eq!(state.dirty, Dirty::False);
        for r in &state.row_data {
            assert!(!r.dirty);
        }

        // Write to first line
        feed(&mut t, b"A");
        state.update(&mut t).unwrap();
        assert_eq!(state.dirty, Dirty::Partial);
        assert!(state.row_data[0].dirty); // First row dirty
        assert!(!state.row_data[1].dirty); // Second row clean
    }

    #[test]
    fn colors() {
        let mut t = term(10, 5);

        let mut state = RenderState::new();

        // Default colors
        state.update(&mut t).unwrap();

        // Change cursor color
        feed(&mut t, b"\x1b]12;#FF0000\x07");
        state.update(&mut t).unwrap();

        let c = state.colors.cursor.unwrap();
        assert_eq!(c.r, 0xFF);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);

        // Change palette color 0 to White
        feed(&mut t, b"\x1b]4;0;#FFFFFF\x07");
        state.update(&mut t).unwrap();
        let p0 = state.colors.palette[0];
        assert_eq!(p0.r, 0xFF);
        assert_eq!(p0.g, 0xFF);
        assert_eq!(p0.b, 0xFF);
    }

    fn pin_active(t: &Terminal, x: u16, y: u32) -> Pin {
        t.screen()
            .pages
            .pin(Point::Active(Coordinate { x, y }))
            .unwrap()
    }

    #[test]
    fn selection_single_line() {
        let mut t = term(10, 3);

        let (a, b) = (pin_active(&t, 0, 1), pin_active(&t, 2, 1));
        t.screen_mut()
            .select(Some(Selection::new(a, b, false)))
            .unwrap();

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        assert_eq!(state.row_data[0].selection, None);
        assert_eq!(state.row_data[1].selection, Some([0, 2]));
        assert_eq!(state.row_data[2].selection, None);

        // Clear the selection
        t.screen_mut().select(None).unwrap();
        state.update(&mut t).unwrap();
        assert_eq!(state.row_data[0].selection, None);
        assert_eq!(state.row_data[1].selection, None);
        assert_eq!(state.row_data[2].selection, None);
    }

    #[test]
    fn selection_multiple_lines() {
        let mut t = term(10, 3);

        let (a, b) = (pin_active(&t, 0, 1), pin_active(&t, 2, 2));
        t.screen_mut()
            .select(Some(Selection::new(a, b, false)))
            .unwrap();

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        assert_eq!(state.row_data[0].selection, None);
        assert_eq!(
            state.row_data[1].selection,
            Some([0, t.screen().pages.cols - 1])
        );
        assert_eq!(state.row_data[2].selection, Some([0, 2]));
    }

    #[test]
    fn link_cells() {
        let mut t = term(10, 5);

        let mut state = RenderState::new();

        // Create a hyperlink
        feed(
            &mut t,
            b"\x1b]8;;http://example.com\x1b\\LINK\x1b]8;;\x1b\\",
        );
        state.update(&mut t).unwrap();

        // Query link at 0,0
        let cells = state.link_cells(&t.screen().pages, Coordinate { x: 0, y: 0 });
        assert_eq!(cells.len(), 4);
        assert!(cells.contains(&Coordinate { x: 0, y: 0 }));
        assert!(cells.contains(&Coordinate { x: 1, y: 0 }));
        assert!(cells.contains(&Coordinate { x: 2, y: 0 }));
        assert!(cells.contains(&Coordinate { x: 3, y: 0 }));

        // Query no link
        let cells2 = state.link_cells(&t.screen().pages, Coordinate { x: 4, y: 0 });
        assert_eq!(cells2.len(), 0);
    }

    #[test]
    fn string() {
        let mut t = term(5, 2);
        feed(&mut t, b"AB");

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        let mut result = String::new();
        state.string(&mut result, None).unwrap();

        let expected = "AB\x00\x00\x00\n\x00\x00\x00\x00\x00\n";
        assert_eq!(result, expected);

        // With a map, every byte maps to its cell; newline maps to x = cols.
        let mut result = String::new();
        let mut map = Vec::new();
        state.string(&mut result, Some(&mut map)).unwrap();
        assert_eq!(map.len(), result.len());
        assert_eq!(map[0], Coordinate { x: 0, y: 0 });
        assert_eq!(map[5], Coordinate { x: 5, y: 0 });
        assert_eq!(map[6], Coordinate { x: 0, y: 1 });
    }

    #[test]
    fn link_cells_with_scrollback_spanning_pages() {
        let viewport_rows: u16 = 10;
        let tail_rows: u16 = 5;

        let mut t = Terminal::new(Options {
            max_scrollback: 10_000,
            ..Options::new(page::STD_CAPACITY.cols, viewport_rows)
        })
        .unwrap();

        let first_page_cap = {
            let pages = &t.screen().pages;
            pages.node(pages.first.unwrap()).unwrap().data.capacity.rows
        };

        // Fill first page
        for _ in 0..first_page_cap - 1 {
            feed(&mut t, b"\r\n");
        }

        // Create second page with hyperlink
        feed(&mut t, b"\r\n");
        feed(
            &mut t,
            b"\x1b]8;;http://example.com\x1b\\LINK\x1b]8;;\x1b\\",
        );
        for _ in 0..tail_rows - 1 {
            feed(&mut t, b"\r\n");
        }

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        let expected_viewport_y = u32::from(viewport_rows - tail_rows);
        let cells = state.link_cells(
            &t.screen().pages,
            Coordinate {
                x: 0,
                y: expected_viewport_y,
            },
        );
        assert_eq!(cells.len(), 4);
    }

    #[test]
    fn link_cells_with_invalid_viewport_point() {
        let mut t = term(10, 5);

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        // Row out of bound
        let cells = state.link_cells(
            &t.screen().pages,
            Coordinate {
                x: 0,
                y: u32::from(t.rows) + 10,
            },
        );
        assert_eq!(cells.len(), 0);

        // Col out of bound
        let cells = state.link_cells(
            &t.screen().pages,
            Coordinate {
                x: t.cols + 10,
                y: 0,
            },
        );
        assert_eq!(cells.len(), 0);
    }

    #[test]
    fn dirty_row_resets_highlights() {
        let mut t = term(10, 3);
        feed(&mut t, b"ABC");

        let mut state = RenderState::new();
        state.update(&mut t).unwrap();

        // Reset dirty state
        clear_dirty(&mut state);

        // Manually add a highlight to row 0
        state.row_data[0].highlights.push(Highlight {
            tag: 1,
            range: [0, 2],
        });
        assert_eq!(state.row_data[0].highlights.len(), 1);

        // Write to row 0 to make it dirty
        feed(&mut t, b"\x1b[H"); // Move to home
        feed(&mut t, b"X");
        state.update(&mut t).unwrap();

        // Verify the highlight was reset on the dirty row
        assert_eq!(state.row_data[0].highlights.len(), 0);
    }

    // Additional tests (not in Zig).

    #[test]
    fn bg_color_cell_style() {
        let mut t = term(10, 3);
        // The row must have managed memory (a styled cell) for the style of
        // bg-color-only cells to be populated (same as Zig).
        feed(&mut t, b"\x1b[48;2;1;2;3mA\x1b[K");
        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
        let cell = state.row_data[0].cells.get(5);
        assert_eq!(cell.raw.content_tag(), ContentTag::BgColorRgb);
        assert_eq!(cell.style.bg_color, Color::Rgb(Rgb { r: 1, g: 2, b: 3 }));
    }

    #[test]
    fn update_highlights_flattened() {
        let mut t = term(10, 3);
        feed(&mut t, b"ABC");
        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
        clear_dirty(&mut state);

        let pages = &t.screen().pages;
        let start = pages
            .pin(Point::Viewport(Coordinate { x: 1, y: 0 }))
            .unwrap();
        let end = pages
            .pin(Point::Viewport(Coordinate { x: 4, y: 1 }))
            .unwrap();
        let hl = Flattened::init(pages, start, end).unwrap();
        state.update_highlights_flattened(7, &[hl]);
        assert_eq!(state.dirty, Dirty::Partial);
        assert_eq!(
            state.row_data[0].highlights,
            vec![Highlight {
                tag: 7,
                range: [1, 9]
            }]
        );
        assert_eq!(
            state.row_data[1].highlights,
            vec![Highlight {
                tag: 7,
                range: [0, 4]
            }]
        );
        assert_eq!(state.row_data[2].highlights, vec![]);
        assert!(state.row_data[0].dirty && state.row_data[1].dirty);
        assert!(!state.row_data[2].dirty);
    }
}
