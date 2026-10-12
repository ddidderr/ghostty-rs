//! GPU cell contents (`renderer/cell.zig`).
//!
//! `Contents` holds the CPU-side copy of everything the cell shaders draw:
//! one background color per grid cell plus per-row lists of foreground
//! (text, underline, strikethrough, overline) instances.

use ghostty_terminal::page::Cell as PageCell;

use crate::{
    cursor::Style as CursorStyle,
    opengl::shaders::{CellBg, CellText},
    size::{CellCountInt, GridSize},
};

/// The possible cell content keys that exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Bg,
    Text,
    Underline,
    Strikethrough,
    Overline,
}

/// The contents of all the cells in the terminal.
///
/// The goal of this data structure is to allow for efficient row-wise
/// clearing of data from the GPU buffers, to allow for row-wise dirty
/// tracking to eliminate the overhead of rebuilding the GPU buffers
/// each frame.
///
/// Must be initialized by resizing before calling any operations.
#[derive(Debug, Default)]
pub struct Contents {
    pub size: GridSize,

    /// Flat array containing cell background colors for the terminal grid.
    ///
    /// Indexed as `bg_cells[row * size.columns + col]`.
    ///
    /// Prefer accessing with [`Contents::bg_cell`] instead of directly
    /// indexing in order to avoid integer size bugs.
    pub bg_cells: Vec<CellBg>,

    /// All of the foreground cell lists (`ArrayListCollection`).
    ///
    /// When sized with [`Contents::resize`] the individual lists are given
    /// enough room that they can hold a single row with `#cols` glyphs,
    /// underlines, and strikethroughs, but they may still grow since combining
    /// glyphs and multi-substitution fonts can put more glyphs in a row.
    ///
    /// Rows are indexed as `fg_rows[y + 1]`, because the first list in the
    /// collection is reserved for the block cursor (it must be drawn first)
    /// and the last for the other cursor styles (drawn last).
    pub fg_rows: Vec<Vec<CellText>>,
}

impl Contents {
    pub fn new() -> Contents {
        Contents::default()
    }

    /// Resize the cell contents for the given grid size. This will
    /// always invalidate the entire cell contents.
    pub fn resize(&mut self, size: GridSize) {
        self.size = size;

        let cell_count = usize::from(size.columns) * usize::from(size.rows);
        self.bg_cells = vec![[0, 0, 0, 0]; cell_count];

        // The foreground lists can hold glyphs, underlines and
        // strikethroughs, so we give them an initial capacity of columns * 3
        // which avoids further allocations in the vast majority of cases.
        //
        // We have rows + 2 lists because indexes 0 and rows + 1 are used for
        // special lists containing the cursor cell which need to be first and
        // last in the buffer, respectively. Those only need room for 1 item.
        let rows = usize::from(size.rows);
        let mut fg_rows: Vec<Vec<CellText>> = Vec::with_capacity(rows + 2);
        fg_rows.push(Vec::with_capacity(1));
        for _ in 0..rows {
            fg_rows.push(Vec::with_capacity(usize::from(size.columns) * 3));
        }
        fg_rows.push(Vec::with_capacity(1));
        self.fg_rows = fg_rows;
    }

    /// Reset the cell contents to an empty state without resizing.
    pub fn reset(&mut self) {
        self.bg_cells.fill([0, 0, 0, 0]);
        for list in &mut self.fg_rows {
            list.clear();
        }
    }

    /// Set the cursor value. If the value is None then the cursor is hidden.
    pub fn set_cursor(&mut self, v: Option<CellText>, cursor_style: Option<CursorStyle>) {
        if self.size.rows == 0 {
            return;
        }
        let last = usize::from(self.size.rows) + 1;
        self.fg_rows[0].clear();
        self.fg_rows[last].clear();

        let Some(cell) = v else { return };
        let Some(style) = cursor_style else { return };

        match style {
            // Block cursors should be drawn first
            CursorStyle::Block => self.fg_rows[0].push(cell),
            // Other cursor styles should be drawn last
            CursorStyle::BlockHollow
            | CursorStyle::Bar
            | CursorStyle::Underline
            | CursorStyle::Lock => self.fg_rows[last].push(cell),
        }
    }

    /// Returns the current cursor glyph if present, checking both cursor lists.
    pub fn get_cursor_glyph(&self) -> Option<CellText> {
        if self.size.rows == 0 {
            return None;
        }
        if let Some(c) = self.fg_rows[0].first() {
            return Some(*c);
        }
        if let Some(c) = self.fg_rows[usize::from(self.size.rows) + 1].first() {
            return Some(*c);
        }
        None
    }

    /// Access a background cell. Prefer this over direct indexing of
    /// `bg_cells` in order to avoid integer size bugs causing overflows.
    #[inline]
    pub fn bg_cell(&mut self, row: usize, col: usize) -> &mut CellBg {
        let cols = usize::from(self.size.columns);
        &mut self.bg_cells[row * cols + col]
    }

    /// Add a cell to the appropriate list. Adding the same cell twice will
    /// result in duplication in the vertex buffer. The caller should clear
    /// the corresponding row with [`Contents::clear`] to remove old cells
    /// first.
    ///
    /// `key` is only used to document the call site (Zig selects the cell
    /// type at comptime); background cells are set via [`Contents::bg_cell`].
    #[inline]
    pub fn add(&mut self, key: Key, cell: CellText) {
        let y = usize::from(cell.grid_pos[1]);
        debug_assert!(y < usize::from(self.size.rows));
        match key {
            Key::Bg => unreachable!("background cells are set with bg_cell"),
            // We have a special list containing the cursor cell at the start
            // of our fg row collection, so we need to add 1 to the y to get
            // the correct index.
            Key::Text | Key::Underline | Key::Strikethrough | Key::Overline => {
                self.fg_rows[y + 1].push(cell);
            }
        }
    }

    /// Clear all of the cell contents for a given row.
    pub fn clear(&mut self, y: CellCountInt) {
        debug_assert!(y < self.size.rows);
        let cols = usize::from(self.size.columns);
        let start = usize::from(y) * cols;
        self.bg_cells[start..start + cols].fill([0, 0, 0, 0]);

        // We have a special list containing the cursor cell at the start
        // of our fg row collection, so we need to add 1 to the y to get
        // the correct index.
        self.fg_rows[usize::from(y) + 1].clear();
    }
}

/// Returns true if a codepoint for a cell is a covering character. A covering
/// character is a character that covers the entire cell. This is used to
/// make window-padding-color=extend work better. See #2099.
pub fn is_covering(cp: u32) -> bool {
    matches!(
        cp, // U+2588 FULL BLOCK
        0x2588
    )
}

/// Returns true of the codepoint is a "symbol-like" character, which
/// for now we define as anything in a private use area, and anything
/// in several unicode blocks:
/// - Arrows
/// - Dingbats
/// - Emoticons
/// - Miscellaneous Symbols
/// - Enclosed Alphanumerics
/// - Enclosed Alphanumeric Supplement
/// - Miscellaneous Symbols and Pictographs
/// - Transport and Map Symbols
///
/// In the future it may be prudent to expand this to encompass more
/// symbol-like characters, and/or exclude some PUA sections.
pub fn is_symbol(cp: u32) -> bool {
    ghostty_unicode::symbols_table::table().get(cp)
}

/// Returns the appropriate `constraint_width` for
/// the provided cell when rendering its glyph(s).
pub fn constraint_width(raw_slice: &[PageCell], x: usize, cols: usize) -> u8 {
    let cell = raw_slice[x];
    let cp = cell.codepoint();

    let grid_width = cell.grid_width();

    // If the grid width of the cell is 2, the constraint
    // width will always be 2, so we can just return early.
    if grid_width > 1 {
        return grid_width;
    }

    // We allow "symbol-like" glyphs to extend to 2 cells wide if there's
    // space, and if the previous glyph wasn't also a symbol. So if this
    // codepoint isn't a symbol then we can return the grid width.
    if !is_symbol(cp) {
        return grid_width;
    }

    // If we are at the end of the screen it must be constrained to one cell.
    if x == cols - 1 {
        return 1;
    }

    // If we have a previous cell and it was a symbol then we need
    // to also constrain. This is so that multiple PUA glyphs align.
    // This does not apply if the previous symbol is a graphics
    // element such as a block element or Powerline glyph.
    if x > 0 {
        let prev_cp = raw_slice[x - 1].codepoint();
        if is_symbol(prev_cp) && !is_graphics_element(prev_cp) {
            return 1;
        }
    }

    // If the next cell is whitespace, then we
    // allow the glyph to be up to two cells wide.
    let next_cp = raw_slice[x + 1].codepoint();
    if next_cp == 0 || is_space(next_cp) {
        return 2;
    }

    // Otherwise, this has to be 1 cell wide.
    1
}

/// Whether min contrast should be disabled for a given glyph. True
/// for graphics elements such as blocks and Powerline glyphs.
pub fn no_min_contrast(cp: u32) -> bool {
    is_graphics_element(cp)
}

// Some general spaces, others intentionally kept
// to force the font to render as a fixed width.
fn is_space(ch: u32) -> bool {
    matches!(
        ch,
        0x0020 // SPACE
        | 0x2002 // EN SPACE
    )
}

/// Returns true if the codepoint is used for terminal graphics, such
/// as box drawing characters, block elements, and Powerline glyphs.
fn is_graphics_element(ch: u32) -> bool {
    is_box_drawing(ch) || is_block_element(ch) || is_legacy_computing(ch) || is_powerline(ch)
}

// Returns true if the codepoint is a box drawing character.
fn is_box_drawing(ch: u32) -> bool {
    matches!(ch, 0x2500..=0x257F)
}

// Returns true if the codepoint is a block element.
fn is_block_element(ch: u32) -> bool {
    matches!(ch, 0x2580..=0x259F)
}

// Returns true if the codepoint is in a Symbols for Legacy
// Computing block, including supplements.
fn is_legacy_computing(ch: u32) -> bool {
    matches!(
        ch,
        0x1FB00..=0x1FBFF
        | 0x1CC00..=0x1CEBF // Supplement introduced in Unicode 16.0
    )
}

// Returns true if the codepoint is a part of the Powerline range.
fn is_powerline(ch: u32) -> bool {
    matches!(ch, 0xE0B0..=0xE0D7)
}

#[cfg(test)]
mod tests {
    use ghostty_terminal::{
        render::RenderState,
        terminal::{Options, Terminal},
    };

    use super::*;

    fn fg(x: u16, y: u16) -> CellText {
        CellText {
            atlas: 0,
            grid_pos: [x, y],
            color: [0, 0, 0, 1],
            ..CellText::default()
        }
    }

    #[test]
    fn contents() {
        let rows = 10;
        let cols = 10;

        let mut c = Contents::new();
        c.resize(GridSize {
            rows,
            columns: cols,
        });

        // We should start off empty after resizing.
        for y in 0..usize::from(rows) {
            assert_eq!(c.fg_rows[y + 1].len(), 0);
            for x in 0..usize::from(cols) {
                assert_eq!([0, 0, 0, 0], *c.bg_cell(y, x));
            }
        }
        // And the cursor row should have a capacity of 1 and also be empty.
        assert!(c.fg_rows[0].capacity() >= 1);
        assert_eq!(c.fg_rows[0].len(), 0);

        // Add some contents.
        let bg_cell: CellBg = [0, 0, 0, 1];
        let fg_cell = fg(4, 1);
        *c.bg_cell(1, 4) = bg_cell;
        c.add(Key::Text, fg_cell);
        assert_eq!(bg_cell, *c.bg_cell(1, 4));
        // The fg row index is offset by 1 because of the cursor list.
        assert_eq!(fg_cell, c.fg_rows[2][0]);

        // And we should be able to clear it.
        c.clear(1);
        for y in 0..usize::from(rows) {
            assert_eq!(c.fg_rows[y + 1].len(), 0);
            for x in 0..usize::from(cols) {
                assert_eq!([0, 0, 0, 0], *c.bg_cell(y, x));
            }
        }

        // Add a block cursor.
        let cursor_cell = CellText {
            bools: CellText::IS_CURSOR_GLYPH,
            ..fg(2, 3)
        };
        c.set_cursor(Some(cursor_cell), Some(CursorStyle::Block));
        assert_eq!(cursor_cell, c.fg_rows[0][0]);
        assert_eq!(cursor_cell, c.get_cursor_glyph().unwrap());

        // And remove it.
        c.set_cursor(None, None);
        assert_eq!(0, c.fg_rows[0].len());
        assert!(c.get_cursor_glyph().is_none());

        // Add a hollow cursor.
        c.set_cursor(Some(cursor_cell), Some(CursorStyle::BlockHollow));
        assert_eq!(cursor_cell, c.fg_rows[usize::from(rows) + 1][0]);
        assert_eq!(cursor_cell, c.get_cursor_glyph().unwrap());
    }

    #[test]
    fn contents_clear_retains_other_content() {
        let mut c = Contents::new();
        c.resize(GridSize {
            rows: 10,
            columns: 10,
        });

        // bg and fg cells in row 1
        let bg_cell_1: CellBg = [0, 0, 0, 1];
        let fg_cell_1 = fg(4, 1);
        *c.bg_cell(1, 4) = bg_cell_1;
        c.add(Key::Text, fg_cell_1);
        // bg and fg cells in row 2
        let bg_cell_2: CellBg = [0, 0, 0, 1];
        let fg_cell_2 = fg(4, 2);
        *c.bg_cell(2, 4) = bg_cell_2;
        c.add(Key::Text, fg_cell_2);

        // Clear row 1, this should leave row 2 untouched
        c.clear(1);

        // Row 2 should still contain its cells.
        assert_eq!(bg_cell_2, *c.bg_cell(2, 4));
        // Fg row index is +1 because of cursor list at start
        assert_eq!(fg_cell_2, c.fg_rows[3][0]);
    }

    #[test]
    fn contents_clear_last_added_content() {
        let mut c = Contents::new();
        c.resize(GridSize {
            rows: 10,
            columns: 10,
        });

        let bg_cell_1: CellBg = [0, 0, 0, 1];
        let fg_cell_1 = fg(4, 1);
        *c.bg_cell(1, 4) = bg_cell_1;
        c.add(Key::Text, fg_cell_1);
        let bg_cell_2: CellBg = [0, 0, 0, 1];
        let fg_cell_2 = fg(4, 2);
        *c.bg_cell(2, 4) = bg_cell_2;
        c.add(Key::Text, fg_cell_2);

        // Clear row 2, this should leave row 1 untouched
        c.clear(2);

        // Row 1 should still contain its cells.
        assert_eq!(bg_cell_1, *c.bg_cell(1, 4));
        assert_eq!(fg_cell_1, c.fg_rows[2][0]);
    }

    #[test]
    fn contents_with_zero_sized_screen() {
        let mut c = Contents::new();
        c.set_cursor(None, None);
        assert!(c.get_cursor_glyph().is_none());
    }

    #[test]
    fn contents_reset_clears_everything() {
        let mut c = Contents::new();
        c.resize(GridSize {
            rows: 3,
            columns: 3,
        });
        *c.bg_cell(1, 1) = [1, 2, 3, 4];
        c.add(Key::Underline, fg(1, 1));
        c.set_cursor(Some(fg(0, 0)), Some(CursorStyle::Bar));
        c.reset();
        assert_eq!(*c.bg_cell(1, 1), [0, 0, 0, 0]);
        assert!(c.fg_rows.iter().all(Vec::is_empty));
    }

    fn feed(t: &mut Terminal, s: &str) {
        let mut st = t.vt_stream();
        st.next_slice(s.as_bytes()).unwrap();
    }

    /// Reset, write `s`, update the render state and return the
    /// constraint width at each requested x.
    fn widths(t: &mut Terminal, state: &mut RenderState, s: &str, xs: &[usize]) -> Vec<u8> {
        t.full_reset();
        feed(t, s);
        state.update(t).unwrap();
        xs.iter()
            .map(|&x| constraint_width(&state.row_data[0].cells.raw, x, usize::from(state.cols)))
            .collect()
    }

    #[test]
    fn cell_constraint_widths() {
        let mut t = Terminal::new(Options::new(4, 1)).unwrap();
        let mut state = RenderState::new();
        let pua = "\u{e8ef}";

        // for each case, the numbers in the comment denote expected
        // constraint widths for the symbol-containing cells

        // symbol->nothing: 2
        assert_eq!(widths(&mut t, &mut state, pua, &[0]), [2]);
        // symbol->character: 1
        assert_eq!(widths(&mut t, &mut state, "\u{e8ef}z", &[0]), [1]);
        // symbol->space: 2
        assert_eq!(widths(&mut t, &mut state, "\u{e8ef} z", &[0]), [2]);
        // symbol->no-break space: 1
        assert_eq!(widths(&mut t, &mut state, "\u{e8ef}\u{00a0}z", &[0]), [1]);
        // symbol->end of row: 1
        assert_eq!(widths(&mut t, &mut state, "   \u{e8ef}", &[3]), [1]);
        // character->symbol: 2
        assert_eq!(widths(&mut t, &mut state, "z\u{e8ef}", &[1]), [2]);
        // symbol->symbol: 1,1
        assert_eq!(
            widths(&mut t, &mut state, "\u{e8ef}\u{e8ef}", &[0, 1]),
            [1, 1]
        );
        // symbol->space->symbol: 2,2
        assert_eq!(
            widths(&mut t, &mut state, "\u{e8ef} \u{e8ef}", &[0, 2]),
            [2, 2]
        );
        // symbol->powerline: 1
        assert_eq!(widths(&mut t, &mut state, "\u{e8ef}\u{e0b0}", &[0]), [1]);
        // powerline->symbol: 2
        assert_eq!(widths(&mut t, &mut state, "\u{e0b2}\u{e8ef}", &[1]), [2]);
        // powerline->nothing: 2
        assert_eq!(widths(&mut t, &mut state, "\u{e0b2}", &[0]), [2]);
        // powerline->space: 2
        assert_eq!(widths(&mut t, &mut state, "\u{e0b2} z", &[0]), [2]);
    }

    #[test]
    fn covering_and_min_contrast_helpers() {
        assert!(is_covering(0x2588));
        assert!(!is_covering(u32::from('a')));
        assert!(no_min_contrast(0x2500));
        assert!(no_min_contrast(0x2580));
        assert!(no_min_contrast(0xE0B0));
        assert!(no_min_contrast(0x1FB00));
        assert!(!no_min_contrast(u32::from('a')));
    }
}
