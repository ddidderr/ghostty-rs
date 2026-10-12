//! Text runs and the run iterator. Port of `font/shaper/run.zig`.
//!
//! A [`RunIterator`] walks one terminal row (given as a slice of
//! [`RunCell`]) and yields [`TextRun`]s: maximal sequences of cells that can
//! be shaped together (same font, same style apart from background, not
//! crossing a selection or cursor boundary, ...). The codepoints of each run
//! are fed to the shaper through the [`RunIteratorHook`] trait
//! (`prepare` / `add_codepoint` / `finalize`).

#![allow(clippy::collapsible_if)]

use std::hash::{DefaultHasher, Hash, Hasher};

use ghostty_config::types_a::FontShapingBreak;
use ghostty_terminal::{
    page::{Cell, ContentTag, Page, Wide},
    style::{Color, Style as TermStyle},
};

use crate::{
    Presentation,
    Style,
    collection::Index,
    shared_grid::{Error as GridError, SharedGrid},
};

/// The Kitty graphics unicode placeholder codepoint
/// (`terminal.kitty.graphics.unicode.placeholder`).
// TODO(integration): use the constant from ghostty-terminal once the kitty
// graphics module lands there.
pub const KITTY_PLACEHOLDER: u32 = 0x10EEEE;

/// A single text run. A text run is only valid for one Shaper instance and
/// until the next run is created. A text run never goes across multiple
/// rows in a terminal, so it is guaranteed to always be one line.
///
/// Deviation: the Zig struct carries a `grid: *SharedGrid` field. Here the
/// grid is not part of the run; shapers take it explicitly
/// (`harfbuzz::Shaper::shape_run(grid, run)`), which keeps `TextRun` `Copy`
/// and lifetime-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRun {
    /// A unique hash for this run. This can be used to cache the shaping
    /// results. We don't provide a means to compare actual values if the
    /// hash is the same, so we should continue to improve this hash to
    /// lower the chance of hash collisions if they become a problem. If
    /// there are hash collisions, it would result in rendering issues but
    /// the core data would be correct.
    ///
    /// The hash is position-independent within the row by using relative
    /// cluster positions. This allows identical runs in different positions
    /// to share the same cache entry, improving cache efficiency.
    pub hash: u64,

    /// The offset in the row where this run started. This is added to the
    /// X position of the final shaped cells to get the absolute position
    /// in the row where they belong.
    pub offset: u16,

    /// The total number of cells produced by this run.
    pub cells: u16,

    /// The font index to use for the glyphs of this run.
    pub font_index: Index,
}

/// One cell of a row to shape: the equivalent of one element of
/// `terminal.RenderState.Cell` (`raw`, `grapheme`, `style`).
// TODO(integration): replace with the render-state cell type from
// ghostty-terminal once `RenderState` is ported there.
#[derive(Debug, Clone, Default)]
pub struct RunCell {
    /// The raw page cell.
    pub raw: Cell,
    /// The extra codepoints of the grapheme (empty unless
    /// `raw.has_grapheme()`).
    pub grapheme: Vec<u32>,
    /// The resolved style (default style if the cell has no styling).
    pub style: TermStyle,
}

impl RunCell {
    /// Build the run cells for row `y` of a page.
    pub fn from_page_row(page: &Page, y: usize) -> Vec<RunCell> {
        let Some(cells) = page.get_cells(y) else {
            return Vec::new();
        };
        cells
            .iter()
            .enumerate()
            .map(|(x, &raw)| {
                let grapheme = if raw.has_grapheme() {
                    page.cell_index(x, y)
                        .ok()
                        .and_then(|i| page.lookup_grapheme(i))
                        .map(<[u32]>::to_vec)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let style = if raw.has_styling() {
                    page.style(raw)
                } else {
                    TermStyle::default()
                };
                RunCell {
                    raw,
                    grapheme,
                    style,
                }
            })
            .collect()
    }
}

/// Options for [`RunIterator`] (`shape.RunOptions`).
pub struct RunOptions<'a> {
    /// The font state for the terminal screen. Cached values may be updated
    /// during shaping (the grid has interior locking).
    pub grid: &'a SharedGrid,

    /// The cells for the row to shape.
    pub cells: &'a [RunCell],

    /// The x boundaries of the selection in this row.
    pub selection: Option<[u16; 2]>,

    /// The cursor position within this row. This is used to break shaping
    /// on cursor boundaries. This can be disabled by setting this to
    /// None.
    pub cursor_x: Option<usize>,
}

impl<'a> RunOptions<'a> {
    pub fn new(grid: &'a SharedGrid, cells: &'a [RunCell]) -> Self {
        RunOptions {
            grid,
            cells,
            selection: None,
            cursor_x: None,
        }
    }

    /// Apply the font break configuration to the run.
    pub fn apply_break_config(&mut self, config: FontShapingBreak) {
        if !config.cursor {
            self.cursor_x = None;
        }
    }
}

/// The hooks a shaper provides to the run iterator
/// (`font.Shaper.RunIteratorHook`).
pub trait RunIteratorHook {
    /// Called before a new run is started.
    fn prepare(&mut self);
    /// Called for each codepoint of the run, with the (run relative) cluster.
    fn add_codepoint(&mut self, cp: u32, cluster: u32);
    /// Called when the run is complete.
    fn finalize(&mut self);
}

impl RunIteratorHook for crate::shaper::harfbuzz::Shaper {
    fn prepare(&mut self) {
        Self::prepare(self);
    }
    fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        Self::add_codepoint(self, cp, cluster);
    }
    fn finalize(&mut self) {
        Self::finalize(self);
    }
}

impl RunIteratorHook for crate::shaper::noop::Shaper {
    fn prepare(&mut self) {
        Self::prepare(self);
    }
    fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        Self::add_codepoint(self, cp, cluster);
    }
    fn finalize(&mut self) {
        Self::finalize(self);
    }
}

/// RunIterator is an iterator that yields text runs. Unlike Zig, which
/// stores the hooks inside the iterator, the shaper is passed to
/// [`RunIterator::next`] so it can be used (to shape) between calls.
pub struct RunIterator<'a> {
    pub opts: RunOptions<'a>,
    i: usize,
}

impl<'a> RunIterator<'a> {
    pub fn new(opts: RunOptions<'a>) -> Self {
        RunIterator { opts, i: 0 }
    }

    pub fn next(&mut self, hooks: &mut dyn RunIteratorHook) -> Result<Option<TextRun>, GridError> {
        let cells = self.opts.cells;

        // Trim the right side of a row that might be empty
        let max: usize = (0..cells.len())
            .rev()
            .find(|&i| !cells[i].raw.is_empty())
            .map_or(0, |i| i + 1);

        // Invisible cells don't have any glyphs rendered,
        // so we explicitly skip them in the shaping process.
        while self.i < max
            && (cells[self.i].raw.has_styling() && cells[self.i].style.flags.invisible)
        {
            self.i += 1;
        }

        // We're over at the max
        if self.i >= max {
            return Ok(None);
        }

        // Track the font for our current run
        let mut current_font = Index::default();

        // Allow the hook to prepare
        hooks.prepare();

        // Initialize our hash for this run.
        let mut hasher = DefaultHasher::new();

        // Let's get our style that we'll expect for the run.
        let style: TermStyle = if cells[self.i].raw.has_styling() {
            cells[self.i].style
        } else {
            TermStyle::default()
        };

        // Go through cell by cell and accumulate while we build our run.
        let start = self.i;
        let end = 'run: {
            for j in start..max {
                // Use relative cluster positions (offset from run start) to make
                // the shaping cache position-independent. This ensures that runs
                // with identical content but different starting positions in the
                // row produce the same hash, enabling cache reuse.
                let cluster = (j - start) as u32;
                let cell = &cells[j];
                let raw = cell.raw;

                // If we have a selection and we're at a boundary point, then
                // we break the run here.
                if let Some(bounds) = self.opts.selection {
                    if j > start {
                        let (b0, b1) = (usize::from(bounds[0]), usize::from(bounds[1]));
                        if b0 > 0 && j == b0 {
                            break 'run j;
                        }
                        if b1 > 0 && j == b1 + 1 {
                            break 'run j;
                        }
                    }
                }

                // If we're a spacer, then we ignore it
                match raw.wide() {
                    Wide::Narrow | Wide::Wide => {}
                    Wide::SpacerHead | Wide::SpacerTail => continue,
                }

                // If our cell attributes are changing, then we split the run.
                // This prevents a single glyph for ">=" to be rendered with
                // one color when the two components have different styling.
                if j > start {
                    let prev_cell = cells[j - 1].raw;

                    // If the prev cell and this cell are both plain
                    // codepoints then we check if they are commonly "bad"
                    // ligatures and spit the run if they are.
                    if prev_cell.content_tag() == ContentTag::Codepoint
                        && raw.content_tag() == ContentTag::Codepoint
                    {
                        let prev_cp = prev_cell.codepoint();
                        let cp = raw.codepoint();
                        match char::from_u32(prev_cp) {
                            // fl, fi
                            Some('f') if cp == u32::from('l') || cp == u32::from('i') => {
                                break 'run j;
                            }
                            // st
                            Some('s') if cp == u32::from('t') => break 'run j,
                            _ => {}
                        }
                    }

                    // If the style is exactly the change then fast path out.
                    // Otherwise the style is different. We allow differing
                    // background styles but any other change results in a
                    // new run.
                    if prev_cell.style_id() != raw.style_id() {
                        let c1 = comparable_style(style);
                        let c2 = comparable_style(if raw.has_styling() {
                            cell.style
                        } else {
                            TermStyle::default()
                        });
                        if c1 != c2 {
                            break 'run j;
                        }
                    }
                }

                // Text runs break when font styles change so we need to get
                // the proper style.
                let font_style = if style.flags.bold {
                    if style.flags.italic {
                        Style::BoldItalic
                    } else {
                        Style::Bold
                    }
                } else if style.flags.italic {
                    Style::Italic
                } else {
                    Style::Regular
                };

                // Determine the presentation format for this glyph.
                let presentation: Option<Presentation> = if raw.has_grapheme() {
                    // We only check the FIRST codepoint because I believe the
                    // presentation format must be directly adjacent to the codepoint.
                    let cps = &cell.grapheme;
                    assert_ne!(cps.len(), 0);
                    match cps[0] {
                        0xFE0E => Some(Presentation::Text),
                        0xFE0F => Some(Presentation::Emoji),
                        _ => None,
                    }
                } else {
                    // If we're not a grapheme, our individual char could be
                    // an emoji so we want to check if we expect emoji presentation.
                    // The font grid index_for_codepoint we use below will do this
                    // automatically.
                    None
                };

                // If our cursor is on this line then we break the run around the
                // cursor. This means that any row with a cursor has at least
                // three breaks: before, exactly the cursor, and after.
                //
                // We do not break a cell that is exactly the grapheme. If there
                // are cells following that contain joiners, we allow those to
                // break. This creates an effect where hovering over an emoji
                // such as a skin-tone emoji is fine, but hovering over the
                // joiners will show the joiners allowing you to modify the
                // emoji.
                if !raw.has_grapheme() {
                    if let Some(cursor_x) = self.opts.cursor_x {
                        // Exactly: self.i is the cursor and we iterated once. This
                        // means that we started exactly at the cursor and did at
                        // exactly one iteration. Why exactly one? Because we may
                        // start at our cursor but do many if our cursor is exactly
                        // on an emoji.
                        if start == cursor_x && j == start + 1 {
                            break 'run j;
                        }

                        // Before: up to and not including the cursor. This means
                        // that we started before the cursor (self.i < cursor_x)
                        // and j is now at the cursor meaning we haven't yet processed
                        // the cursor.
                        if start < cursor_x && j == cursor_x {
                            assert!(j > 0);
                            break 'run j;
                        }

                        // After: after the cursor. We don't need to do anything
                        // special, we just let the run complete.
                    }
                }

                // We need to find a font that supports this character. If
                // there are additional zero-width codepoints (to form a single
                // grapheme, i.e. combining characters), we need to find a font
                // that supports all of them.
                let (font_idx, fallback): (Index, Option<u32>) = 'font_info: {
                    // If we find a font that supports this entire grapheme
                    // then we use that.
                    if let Some(idx) = self.index_for_cell(cell, font_style, presentation)? {
                        break 'font_info (idx, None);
                    }

                    // Otherwise we need a fallback character. Prefer the
                    // official replacement character.
                    if let Some(idx) = self.opts.grid.get_index(0xFFFD, font_style, presentation)? {
                        break 'font_info (idx, Some(0xFFFD));
                    }

                    // Fallback to space
                    if let Some(idx) =
                        self.opts
                            .grid
                            .get_index(u32::from(' '), font_style, presentation)?
                    {
                        break 'font_info (idx, Some(u32::from(' ')));
                    }

                    // We can't render at all. This is a bug, we should always
                    // have a font that can render a space.
                    unreachable!("no font can render a space");
                };

                if j == start {
                    current_font = font_idx;
                }

                // If our fonts are not equal, then we're done with our run.
                if font_idx != current_font {
                    break 'run j;
                }

                // If we're a fallback character, add that and continue; we
                // don't want to add the entire grapheme.
                if let Some(cp) = fallback {
                    add_codepoint(hooks, &mut hasher, cp, cluster);
                    continue;
                }

                // If we're a Kitty unicode placeholder then we add a blank.
                if raw.codepoint() == KITTY_PLACEHOLDER {
                    add_codepoint(hooks, &mut hasher, u32::from(' '), cluster);
                    continue;
                }

                // Add all the codepoints for our grapheme
                add_codepoint(
                    hooks,
                    &mut hasher,
                    if raw.codepoint() == 0 {
                        u32::from(' ')
                    } else {
                        raw.codepoint()
                    },
                    cluster,
                );
                if raw.has_grapheme() {
                    for &cp in &cell.grapheme {
                        // Do not send presentation modifiers
                        if cp == 0xFE0E || cp == 0xFE0F {
                            continue;
                        }
                        add_codepoint(hooks, &mut hasher, cp, cluster);
                    }
                }
            }
            max
        };

        // Finalize our buffer
        hooks.finalize();

        // Add our length to the hash as an additional mechanism to avoid collisions
        (end - start).hash(&mut hasher);

        // Add our font index
        current_font.hash(&mut hasher);

        // Move our cursor.
        self.i = end;

        Ok(Some(TextRun {
            hash: hasher.finish(),
            offset: start as u16,
            cells: (end - start) as u16,
            font_index: current_font,
        }))
    }

    /// Find a font index that supports the grapheme for the given cell,
    /// or None if no such font exists.
    ///
    /// This is used to find a font that supports the entire grapheme.
    /// We look for fonts that support each individual codepoint and then
    /// find the common font amongst all candidates.
    fn index_for_cell(
        &self,
        cell: &RunCell,
        style: Style,
        presentation: Option<Presentation>,
    ) -> Result<Option<Index>, GridError> {
        let raw = cell.raw;
        let grid = self.opts.grid;
        if raw.is_empty() || raw.codepoint() == 0 || raw.codepoint() == KITTY_PLACEHOLDER {
            return grid.get_index(u32::from(' '), style, presentation);
        }

        // Get the font index for the primary codepoint.
        let primary_cp = raw.codepoint();
        let Some(primary) = grid.get_index(primary_cp, style, presentation)? else {
            return Ok(None);
        };

        // Easy, and common: we aren't a multi-codepoint grapheme, so
        // we just return whatever index for the cell codepoint.
        if !raw.has_grapheme() {
            return Ok(Some(primary));
        }

        // If this is a grapheme, we need to find a font that supports
        // all of the codepoints in the grapheme.
        let graphemes = &cell.grapheme;
        let mut candidates: Vec<Index> = Vec::with_capacity(graphemes.len() + 1);
        candidates.push(primary);

        for &cp in graphemes {
            // Ignore Emoji ZWJs
            if cp == 0xFE0E || cp == 0xFE0F || cp == 0x200D {
                continue;
            }

            // Find a font that supports this codepoint. If none support this
            // then the whole grapheme can't be rendered so we return None.
            //
            // We explicitly do not require the additional grapheme components
            // to support the base presentation, since it is common for emoji
            // fonts to support the base emoji with emoji presentation but not
            // certain ZWJ-combined characters like the male and female signs.
            let Some(idx) = grid.get_index(cp, style, None)? else {
                return Ok(None);
            };
            candidates.push(idx);
        }

        // We need to find a candidate that has ALL of our codepoints
        for &idx in &candidates {
            if !grid.has_codepoint(idx, primary_cp, presentation) {
                continue;
            }
            let all = graphemes.iter().all(|&cp| {
                // Ignore Emoji ZWJs
                cp == 0xFE0E || cp == 0xFE0F || cp == 0x200D || grid.has_codepoint(idx, cp, None)
            });
            if all {
                // We have a candidate that supports all of our codepoints.
                return Ok(Some(idx));
            }
        }

        Ok(None)
    }
}

fn add_codepoint(
    hooks: &mut dyn RunIteratorHook,
    hasher: &mut DefaultHasher,
    cp: u32,
    cluster: u32,
) {
    cp.hash(hasher);
    cluster.hash(hasher);
    hooks.add_codepoint(cp, cluster);
}

/// Returns a style that when compared must be identical for a run to
/// continue.
fn comparable_style(style: TermStyle) -> TermStyle {
    let mut s = style;

    // We allow background colors to differ because we'll just paint the
    // cell background whatever the style is, and wherever the glyph
    // lands on top of it will be the color of the glyph.
    s.bg_color = Color::None;

    s
}
