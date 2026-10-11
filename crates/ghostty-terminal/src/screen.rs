//! A single terminal screen: a page list plus the cursor, saved cursor,
//! selection, charset state, kitty keyboard flags and semantic prompt state.
//!
//! Port of `ZIG/src/terminal/Screen.zig`.
//!
//! # Differences from the Zig API
//!
//! * The cursor's `page_pin` is a [`TrackedPin`] handle into `screen.pages`;
//!   the cached `page_row` / `page_cell` pointers are replaced by on-demand
//!   accessors ([`Screen::cursor_row`], [`Screen::cursor_cell`], ...).
//! * Errors use [`page::Error`] (which covers Zig's `Allocator.Error`,
//!   `IncreaseCapacityError`, `SplitError` and the page insertion errors).
//! * Selection helpers take the screen's [`PageList`] explicitly (see
//!   `selection.rs`).
#![allow(
    clippy::manual_assert_eq,
    clippy::single_match_else,
    clippy::doc_overindented_list_items,
    clippy::items_after_statements
)]
use crate::{
    cursor::Style as CursorStyle,
    formatter::{Content as FormatContent, Options as FormatOptions, ScreenFormatter},
    hyperlink::{self, Hyperlink, HyperlinkId},
    kitty::graphics_storage::ImageStorage,
    page::{Cell, ContentTag, Page, Row, SemanticContent, SemanticPrompt as RowPrompt, Wide},
    page_list::{
        CellSubset, Chunk, Direction, IncreaseCapacity, NodeId, PageList, Pin,
        Scroll as ListScroll, TrackedPin, Viewport,
    },
    point::{Coordinate, Point, Tag},
    selection::{Bounds, Order, Selection},
    size,
    style::{self, Style},
};
use ghostty_vt::{
    ansi::ProtectedMode,
    charsets::{Charset, Slots},
    kitty::key::FlagStack,
    osc::semantic_prompt::{Click, PromptKind, Redraw},
    sgr,
};

pub use crate::page::Error;

/// The Unicode codepoint used by the Kitty graphics protocol for virtual
/// placements (`kitty.graphics.unicode.placeholder`).
use crate::kitty::graphics_unicode::PLACEHOLDER as KITTY_PLACEHOLDER;

/// See Terminal.Dirty. This behaves the same way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dirty {
    /// Set when the selection is set or unset, regardless of if the
    /// selection is changed or not.
    pub selection: bool,

    /// When an OSC8 hyperlink is hovered, we set the full screen as dirty
    /// because links can span multiple lines.
    pub hyperlink_hover: bool,
}

/// How click handling in a prompt is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticClick {
    None,
    ClickEvents,
    Cl(Click),
}

/// Semantic prompt (OSC133) state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPrompt {
    /// This is flipped to true when any sort of semantic content is
    /// seen. In particular, this is set to true only when a `prompt` type
    /// is ever set on our cursor.
    ///
    /// This is used to optimize away semantic content operations if we know
    /// we've never seen them.
    pub seen: bool,

    /// This is set on any `cl` or `click_events` option set on the
    /// most recent OSC 133 commands to specify how click handling in a
    /// prompt is handling.
    pub click: SemanticClick,
}

impl SemanticPrompt {
    pub const DISABLED: SemanticPrompt = SemanticPrompt {
        seen: false,
        click: SemanticClick::None,
    };
}

/// Legacy dirty flag value retained for source compatibility.
/// Screens now own the full [`ImageStorage`].
#[derive(Debug, Clone, Default)]
pub struct KittyImages {
    /// Dirty flag for the renderer.
    pub dirty: bool,
}

/// The cursor position and style.
#[derive(Debug, Clone)]
pub struct Cursor {
    /// The x position within the active area.
    pub x: size::CellCountInt,
    /// The y position within the active area.
    pub y: size::CellCountInt,

    /// The visual style of the cursor. This defaults to block because
    /// it has to default to something, but users of this struct are
    /// encouraged to set their own default.
    pub cursor_style: CursorStyle,

    /// The "last column flag (LCF)" as its called. If this is set then the
    /// next character print will force a soft-wrap.
    pub pending_wrap: bool,

    /// The protected mode state of the cursor. If this is true then
    /// all new characters printed will have the protected state set.
    pub protected: bool,

    /// The currently active style. This is the concrete style value
    /// that should be kept up to date. The style ID to use for cell writing
    /// is below.
    pub style: Style,

    /// The currently active style ID. The style is page-specific so when
    /// we change pages we need to ensure that we update that page with
    /// our style when used.
    pub style_id: style::Id,

    /// The hyperlink ID that is currently active for the cursor. A value
    /// of zero means no hyperlink is active. (Implements OSC8, saying that
    /// so code search can find it.).
    pub hyperlink_id: hyperlink::Id,

    /// This is the implicit ID to use for hyperlinks that don't specify
    /// an ID. We do an overflowing add to this so repeats can technically
    /// happen with carefully crafted inputs but for real workloads its
    /// highly unlikely -- and the fix is for the TUI program to use explicit
    /// IDs.
    pub hyperlink_implicit_id: size::OffsetInt,

    /// Heap-allocated hyperlink state so that we can recreate it when
    /// the cursor page pin changes. We can't get it from the old screen
    /// state because the page may be cleared.
    pub hyperlink: Option<Box<Hyperlink>>,

    /// The current semantic content type for the cursor that will be
    /// applied to any newly written cells.
    pub semantic_content: SemanticContent,
    pub semantic_content_clear_eol: bool,

    /// The tracked pin into the page list where the cursor is currently
    /// located. This makes it faster to move the cursor. Read it with
    /// [`Screen::cursor_pin`].
    pub page_pin: TrackedPin,
}

impl Cursor {
    fn new(page_pin: TrackedPin) -> Self {
        Self {
            x: 0,
            y: 0,
            cursor_style: CursorStyle::Block,
            pending_wrap: false,
            protected: false,
            style: Style::default(),
            style_id: style::DEFAULT_ID,
            hyperlink_id: 0,
            hyperlink_implicit_id: 0,
            hyperlink: None,
            semantic_content: SemanticContent::Output,
            semantic_content_clear_eol: false,
            page_pin,
        }
    }
}

/// The set of 4 charsets (G0-G3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharsetArray {
    pub g0: Charset,
    pub g1: Charset,
    pub g2: Charset,
    pub g3: Charset,
}

impl Default for CharsetArray {
    fn default() -> Self {
        Self {
            g0: Charset::Utf8,
            g1: Charset::Utf8,
            g2: Charset::Utf8,
            g3: Charset::Utf8,
        }
    }
}

impl CharsetArray {
    #[inline]
    pub fn get(&self, slot: Slots) -> Charset {
        match slot {
            Slots::G0 => self.g0,
            Slots::G1 => self.g1,
            Slots::G2 => self.g2,
            Slots::G3 => self.g3,
        }
    }

    #[inline]
    pub fn set(&mut self, slot: Slots, charset: Charset) {
        match slot {
            Slots::G0 => self.g0 = charset,
            Slots::G1 => self.g1 = charset,
            Slots::G2 => self.g2 = charset,
            Slots::G3 => self.g3 = charset,
        }
    }
}

/// State required for all charset operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharsetState {
    /// The list of graphical charsets by slot
    pub charsets: CharsetArray,

    /// GL is the slot to use when using a 7-bit printable char (up to 127)
    /// GR used for 8-bit printable chars.
    pub gl: Slots,
    pub gr: Slots,

    /// Single shift where a slot is used for exactly one char.
    pub single_shift: Option<Slots>,
}

impl Default for CharsetState {
    fn default() -> Self {
        Self {
            charsets: CharsetArray::default(),
            gl: Slots::G0,
            gr: Slots::G2,
            single_shift: None,
        }
    }
}

/// Saved cursor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedCursor {
    pub x: size::CellCountInt,
    pub y: size::CellCountInt,
    pub style: Style,
    pub protected: bool,
    pub pending_wrap: bool,
    pub origin: bool,
    pub charset: CharsetState,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub cols: size::CellCountInt,
    pub rows: size::CellCountInt,

    /// The maximum size of scrollback in bytes. Zero means no scrollback.
    /// Any other value will be clamped to support a minimum of the active
    /// area.
    pub max_scrollback: usize,

    /// The total storage limit for Kitty images in bytes for this
    /// screen. Kitty image storage is per-screen.
    pub kitty_image_storage_limit: usize,
}

impl Options {
    /// A simple, default terminal. If you rely on specific dimensions or
    /// scrollback (or lack of) then do not use this directly. This is just
    /// for callers that need some defaults.
    pub const DEFAULT: Options = Options {
        cols: 80,
        rows: 24,
        max_scrollback: 0,
        kitty_image_storage_limit: 320 * 1000 * 1000, // 320MB
    };

    pub fn new(cols: size::CellCountInt, rows: size::CellCountInt, max_scrollback: usize) -> Self {
        Self {
            cols,
            rows,
            max_scrollback,
            ..Self::DEFAULT
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Options for scrolling the viewport of the terminal grid. The reason
/// we have this in addition to PageList.Scroll is because we have additional
/// scroll behaviors that are not part of the PageList.Scroll enum.
#[derive(Debug, Clone, Copy)]
pub enum Scroll {
    /// For all of these, see PageList.Scroll.
    Active,
    Top,
    Pin(Pin),
    Row(usize),
    DeltaRow(isize),
    DeltaPrompt(isize),
}

#[derive(Debug, Clone, Copy)]
pub struct Resize {
    /// The new size to resize to
    pub cols: size::CellCountInt,
    pub rows: size::CellCountInt,

    /// Whether to reflow soft-wrapped text.
    ///
    /// This will reflow soft-wrapped text. If the screen size is getting
    /// smaller and the maximum scrollback size is exceeded, data will be
    /// lost from the top of the scrollback.
    pub reflow: bool,

    /// Set this to enable prompt redraw on resize. This signals
    /// that the running program can redraw the prompt if the cursor is
    /// currently at a prompt. This detects OSC133 prompts lines and clears
    /// them. If set to `Last`, only the most recent prompt line is cleared.
    pub prompt_redraw: Redraw,
}

impl Resize {
    pub fn new(cols: size::CellCountInt, rows: size::CellCountInt) -> Self {
        Self {
            cols,
            rows,
            reflow: true,
            prompt_redraw: Redraw::False,
        }
    }
}

/// The semantic content setting for `cursor_set_semantic_content`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputClear {
    ClearExplicit,
    ClearEol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticContentSet {
    Prompt(PromptKind),
    Output,
    Input(InputClear),
}

pub use crate::string_map::StringMap;

#[derive(Debug, Clone, Copy)]
pub struct SelectionString {
    /// The selection to convert to a string.
    pub sel: Selection,

    /// If true, trim whitespace around the selection.
    pub trim: bool,
}

impl SelectionString {
    pub fn new(sel: Selection) -> Self {
        Self { sel, trim: true }
    }
}

/// The default whitespace set for line/all selection: NUL, space and tab.
pub const DEFAULT_SELECT_WHITESPACE: &[u32] = &[0, ' ' as u32, '\t' as u32];

#[derive(Debug, Clone, Copy)]
pub struct SelectLine<'a> {
    /// The pin of some part of the line to select.
    pub pin: Pin,

    /// These are the codepoints to consider whitespace to trim
    /// from the ends of the selection.
    pub whitespace: Option<&'a [u32]>,

    /// If true, line selection will consider semantic prompt
    /// state changing a boundary. State changing is ANY state
    /// change.
    pub semantic_prompt_boundary: bool,
}

impl SelectLine<'_> {
    pub fn new(pin: Pin) -> SelectLine<'static> {
        SelectLine {
            pin,
            whitespace: Some(DEFAULT_SELECT_WHITESPACE),
            semantic_prompt_boundary: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PromptClickMove {
    pub left: usize,
    pub right: usize,
}

impl PromptClickMove {
    pub const ZERO: PromptClickMove = PromptClickMove { left: 0, right: 0 };
}

/// Options for [`Screen::dump_string`].
#[derive(Debug, Clone, Copy)]
pub struct DumpString {
    /// The start and end points of the dump, both inclusive. The x will
    /// be ignored and the full row will always be dumped.
    pub tl: Pin,
    pub br: Option<Pin>,

    /// If true, this will unwrap soft-wrapped lines. If false, this will
    /// dump the screen as it is visually seen in a rendered window.
    pub unwrap: bool,
}

impl DumpString {
    pub fn new(tl: Pin) -> Self {
        Self {
            tl,
            br: None,
            unwrap: true,
        }
    }
}

/// A screen is a single terminal grid with its own cursor and scrollback.
#[derive(Debug)]
pub struct Screen {
    /// The list of pages in the screen.
    pub pages: PageList,

    /// Special-case where we want no scrollback whatsoever. We have to flag
    /// this because max_size 0 in PageList gets rounded up to two pages so
    /// we can always have an active screen.
    pub no_scrollback: bool,

    /// The current cursor position
    pub cursor: Cursor,

    /// The saved cursor
    pub saved_cursor: Option<SavedCursor>,

    /// The selection for this screen (if any). This MUST be a tracked
    /// selection otherwise the selection will become invalid. Instead of
    /// accessing this directly to set it, use [`Screen::select`] which will
    /// automatically setup tracking.
    pub selection: Option<Selection>,

    /// The charset state
    pub charset: CharsetState,

    /// The current or most recent protected mode. Once a protection mode is
    /// set, this will never become "off" again until the screen is reset.
    /// The current state of whether protection attributes should be set is
    /// set on the Cell pen; this is only used to determine the most recent
    /// protection mode since some sequences such as ECH depend on this.
    pub protected_mode: ProtectedMode,

    /// The kitty keyboard settings.
    pub kitty_keyboard: FlagStack,

    /// Kitty graphics protocol state.
    pub kitty_images: ImageStorage,

    /// Semantic prompt (OSC133) state.
    pub semantic_prompt: SemanticPrompt,

    /// Dirty flags for the renderer.
    pub dirty: Dirty,
}

impl Drop for Screen {
    fn drop(&mut self) {
        self.kitty_images.deinit(&mut self.pages);
    }
}

impl Screen {
    /// Initialize a new screen.
    ///
    /// max_scrollback is the amount of scrollback to keep in bytes. This
    /// will be rounded UP to the nearest page size because our minimum
    /// allocation size is that anyways.
    ///
    /// If max scrollback is 0, then no scrollback is kept at all.
    pub fn new(opts: Options) -> Result<Screen, Error> {
        // Initialize our backing pages.
        let mut pages = PageList::init(opts.cols, opts.rows, Some(opts.max_scrollback))?;

        // Create our tracked pin for the cursor.
        let first = pages.first.ok_or(Error::InvalidId)?;
        let page_pin = pages.track_pin(Pin {
            node: first,
            y: 0,
            x: 0,
            garbage: false,
        })?;

        let mut result = Screen {
            pages,
            no_scrollback: opts.max_scrollback == 0,
            cursor: Cursor::new(page_pin),
            saved_cursor: None,
            selection: None,
            charset: CharsetState::default(),
            protected_mode: ProtectedMode::Off,
            kitty_keyboard: FlagStack::default(),
            kitty_images: ImageStorage::default(),
            semantic_prompt: SemanticPrompt::DISABLED,
            dirty: Dirty::default(),
        };
        result
            .kitty_images
            .set_limit(&mut result.pages, opts.kitty_image_storage_limit)
            .expect("empty image storage accepts any limit");
        Ok(result)
    }

    /// The page for a node (panics if the node is stale).
    pub fn page(&self, node: NodeId) -> &Page {
        &self.pages.node(node).expect("live page node").data
    }

    /// The mutable page for a node (panics if the node is stale).
    pub fn page_mut(&mut self, node: NodeId) -> &mut Page {
        &mut self.pages.node_mut(node).expect("live page node").data
    }

    /// The current location of the cursor in the page list.
    pub fn cursor_pin(&self) -> Pin {
        self.pages
            .tracked_pin(self.cursor.page_pin)
            .expect("cursor pin is always tracked")
    }

    fn set_cursor_pin(&mut self, p: Pin) {
        self.pages
            .set_tracked_pin(self.cursor.page_pin, p)
            .expect("cursor pin must be valid");
    }

    /// The page the cursor is currently on.
    pub fn cursor_page(&self) -> &Page {
        self.page(self.cursor_pin().node)
    }

    /// The page the cursor is currently on.
    pub fn cursor_page_mut(&mut self) -> &mut Page {
        let node = self.cursor_pin().node;
        self.page_mut(node)
    }

    /// The row the cursor is on (Zig: `cursor.page_row`).
    pub fn cursor_row(&self) -> Row {
        let p = self.cursor_pin();
        *self
            .page(p.node)
            .get_row(usize::from(p.y))
            .expect("cursor row")
    }

    pub fn cursor_row_mut(&mut self) -> &mut Row {
        let p = self.cursor_pin();
        self.page_mut(p.node)
            .get_row_mut(usize::from(p.y))
            .expect("cursor row")
    }

    /// The cell under the cursor (Zig: `cursor.page_cell`).
    pub fn cursor_cell(&self) -> Cell {
        let p = self.cursor_pin();
        *self
            .page(p.node)
            .get_row_and_cell(usize::from(p.x), usize::from(p.y))
            .expect("cursor cell")
            .1
    }

    pub fn cursor_cell_mut(&mut self) -> &mut Cell {
        let p = self.cursor_pin();
        let page = self.page_mut(p.node);
        let i = page
            .cell_index(usize::from(p.x), usize::from(p.y))
            .expect("cursor cell");
        &mut page.cells[i]
    }

    /// The cells of the cursor's row.
    pub fn cursor_row_cells(&self) -> &[Cell] {
        let p = self.cursor_pin();
        self.page(p.node)
            .get_cells(usize::from(p.y))
            .expect("cursor row")
    }

    /// Assert that the screen is in a consistent state. This doesn't check
    /// all pages in the page list because that is SO SLOW even just for
    /// tests. This only asserts the screen specific data so callers should
    /// ensure they're also calling page integrity checks if necessary.
    pub fn assert_integrity(&self) {
        if cfg!(debug_assertions) {
            assert!(self.cursor.x < self.pages.cols);
            assert!(self.cursor.y < self.pages.rows);

            // Our cursor x/y should always match the pin. If this doesn't
            // match then it indicates that the tracked pin moved and we
            // didn't account for it by either calling cursor_reload or
            // manually adjusting.
            let pt = self
                .pages
                .point_from_pin(Tag::Active, self.cursor_pin())
                .expect("cursor pin must be in the active area");
            assert_eq!(self.cursor.x, pt.coord().x);
            assert_eq!(u32::from(self.cursor.y), pt.coord().y);
        }
    }

    /// Reset the screen according to the logic of a DEC RIS sequence.
    ///
    /// - Clears the screen and attempts to reclaim memory.
    /// - Moves the cursor to the top-left.
    /// - Clears any cursor state: style, hyperlink, etc.
    /// - Resets the charset
    /// - Clears the selection
    /// - Deletes all Kitty graphics
    /// - Resets Kitty Keyboard settings
    /// - Disables protection mode
    pub fn reset(&mut self) -> Result<(), Error> {
        // Reset our pages
        self.pages.reset()?;

        // The above reset preserves tracked pins so we can still use
        // our cursor pin, which should be at the top-left already.
        let first = self.pages.first.ok_or(Error::InvalidId)?;
        let pin = self.cursor_pin();
        assert!(pin.node == first);
        assert!(pin.x == 0);
        assert!(pin.y == 0);
        self.set_cursor_pin(Pin {
            garbage: false,
            ..pin
        });
        self.cursor = Cursor::new(self.cursor.page_pin);

        // Reset kitty graphics storage
        self.kitty_images.deinit(&mut self.pages);
        self.kitty_images = ImageStorage {
            dirty: true,
            ..Default::default()
        };

        // Reset our basic state
        self.saved_cursor = None;
        self.charset = CharsetState::default();
        self.kitty_keyboard = FlagStack::default();
        self.protected_mode = ProtectedMode::Off;
        self.semantic_prompt = SemanticPrompt::DISABLED;
        self.clear_selection();
        Ok(())
    }

    /// Clone the screen.
    ///
    /// This will copy:
    ///
    ///   - Screen dimensions
    ///   - Screen data (cell state, etc.) for the region
    ///
    /// Anything not mentioned above is NOT copied. Some of this is for
    /// very good reason:
    ///
    ///   - Kitty images have a LOT of data. This is not efficient to copy.
    ///     Use a lock and access the image data. The dirty bit is there for
    ///     a reason.
    ///   - Cursor location can be expensive to calculate with respect to the
    ///     specified region. It is faster to grab the cursor from the old
    ///     screen and then move it to the new screen.
    ///   - Current hyperlink cursor state has heap allocations. Since clone
    ///     is only for read-only operations, it is better to not have any
    ///     hyperlink state. Note that already-written hyperlinks are cloned.
    ///
    /// If not mentioned above, then there isn't a specific reason right now
    /// to not copy some data other than we probably didn't need it and it
    /// isn't necessary for screen coherency.
    ///
    /// Other notes:
    ///
    ///   - The viewport will always be set to the active area of the new
    ///     screen. This is the bottom "rows" rows.
    ///   - If the clone region is smaller than a viewport area, blanks will
    ///     be filled in at the bottom.
    pub fn clone_screen(&self, top: Point, bot: Option<Point>) -> Result<Screen, Error> {
        // Create a tracked pin remapper for our selection and cursor.
        let (mut pages, pin_remap) = self.pages.clone_range(top, bot)?;

        // Find our cursor. If the cursor isn't in the cloned area, we move it
        // to the top-left arbitrarily because a screen must have SOME cursor.
        let mut cursor_xy = None;
        if let Some(&handle) = pin_remap.get(&self.cursor.page_pin)
            && let Some(p) = pages.tracked_pin(handle)
            && let Some(pt) = pages.point_from_pin(Tag::Active, p)
        {
            cursor_xy = Some((handle, pt.coord().x, pt.coord().y as u16));
        }
        let (page_pin, cx, cy) = match cursor_xy {
            Some(v) => v,
            None => {
                let first = pages.first.ok_or(Error::InvalidId)?;
                let h = pages.track_pin(Pin {
                    node: first,
                    y: 0,
                    x: 0,
                    garbage: false,
                })?;
                (h, 0, 0)
            }
        };
        let mut cursor = Cursor::new(page_pin);
        cursor.x = cx;
        cursor.y = cy;

        // Preserve our selection if we have one.
        let sel: Option<Selection> = if let Some(sel) = self.selection {
            assert!(sel.tracked());
            let Bounds::Tracked { start, end } = sel.bounds else {
                unreachable!()
            };

            let (tl, br) = match sel.order(&self.pages) {
                Order::Forward | Order::MirroredForward => (start, end),
                Order::Reverse | Order::MirroredReverse => (end, start),
            };
            let tl_pin = self.pages.tracked_pin(tl).expect("selection pin");
            let br_pin = self.pages.tracked_pin(br).expect("selection pin");

            let start_handle = match pin_remap.get(&tl).copied() {
                Some(h) => Some(h),
                None => 'start: {
                    // No start means it is outside the cloned area.

                    // If we have no end pin then either
                    // (1) our whole selection is outside the cloned area or
                    // (2) our cloned area is within the selection
                    if !pin_remap.contains_key(&br) {
                        // We check if the selection bottom right pin is above
                        // the cloned area or if the top left pin is below the
                        // cloned area, in either of these cases it means that
                        // the selection is fully out of bounds, so we have no
                        // selection in the cloned area and break out now.
                        let Some(clone_top) = self.pages.pin(top) else {
                            break 'start None;
                        };
                        let clone_top_y = self
                            .pages
                            .point_from_pin(Tag::Screen, clone_top)
                            .expect("pin in screen")
                            .coord()
                            .y;
                        if self
                            .pages
                            .point_from_pin(Tag::Screen, br_pin)
                            .expect("pin in screen")
                            .coord()
                            .y
                            < clone_top_y
                        {
                            break 'start None;
                        }
                        if self
                            .pages
                            .point_from_pin(Tag::Screen, tl_pin)
                            .expect("pin in screen")
                            .coord()
                            .y
                            > clone_top_y
                        {
                            break 'start None;
                        }
                    }

                    // We move the top pin back in bounds to the top row.
                    let first = pages.first.ok_or(Error::InvalidId)?;
                    Some(pages.track_pin(Pin {
                        node: first,
                        y: 0,
                        x: if sel.rectangle { tl_pin.x } else { 0 },
                        garbage: false,
                    })?)
                }
            };

            match start_handle {
                None => None,
                Some(start_handle) => {
                    // If we got to this point it means that the selection is
                    // not fully out of bounds, so we move the bottom right pin
                    // back in bounds if it isn't already.
                    let end_handle = match pin_remap.get(&br).copied() {
                        Some(h) => h,
                        None => {
                            let last = pages.last.ok_or(Error::InvalidId)?;
                            let rows = pages.node(last).expect("pin in screen").data.size.rows;
                            pages.track_pin(Pin {
                                node: last,
                                x: if sel.rectangle {
                                    br_pin.x
                                } else {
                                    pages.cols - 1
                                },
                                y: rows - 1,
                                garbage: false,
                            })?
                        }
                    };
                    Some(Selection {
                        bounds: Bounds::Tracked {
                            start: start_handle,
                            end: end_handle,
                        },
                        rectangle: sel.rectangle,
                    })
                }
            }
        } else {
            None
        };

        let result = Screen {
            pages,
            no_scrollback: self.no_scrollback,
            cursor,
            saved_cursor: None,
            selection: sel,
            charset: CharsetState::default(),
            protected_mode: ProtectedMode::Off,
            kitty_keyboard: FlagStack::default(),
            kitty_images: ImageStorage::default(),
            semantic_prompt: SemanticPrompt::DISABLED,
            dirty: self.dirty,
        };
        result.assert_integrity();
        Ok(result)
    }

    /// Increase the capacity of a page, keeping our cursor state (style and
    /// hyperlink reference counts) consistent when it is the cursor page.
    pub fn increase_capacity(
        &mut self,
        node: NodeId,
        adjustment: Option<IncreaseCapacity>,
    ) -> Result<NodeId, Error> {
        // If the page being modified isn't our cursor page then
        // this is a quick operation because we have no additional
        // accounting. We have to do this check here BEFORE calling
        // increase_capacity because increase_capacity will update all
        // our tracked pins (including our cursor).
        if node != self.cursor_pin().node {
            return self.pages.increase_capacity(node, adjustment);
        }

        // We're modifying the cursor page. When we increase the
        // capacity below it will be short the ref count on our
        // current style and hyperlink, so we need to init those.
        let new_node = self.pages.increase_capacity(node, adjustment)?;

        // Re-add the style, if the page somehow doesn't have enough
        // memory to add it, we emit a warning and gracefully degrade
        // to the default style for the cursor.
        if self.cursor.style_id != style::DEFAULT_ID {
            let st = self.cursor.style;
            self.cursor.style_id = match self.page_mut(new_node).styles.add(st) {
                Ok(id) => id,
                Err(err) => {
                    // TODO: Should we increase the capacity further in this case?
                    log::warn!(
                        "(Screen.increaseCapacity) Failed to add cursor style back to page, err={err:?}"
                    );

                    // Reset the cursor style.
                    self.cursor.style = Style::default();
                    style::DEFAULT_ID
                }
            };
        }

        // Re-add the hyperlink, if the page somehow doesn't have enough
        // memory to add it, we emit a warning and gracefully degrade to
        // no hyperlink.
        if let Some(link) = self.cursor.hyperlink.take() {
            // So we don't attempt to free any memory in the replaced page.
            self.cursor.hyperlink_id = 0;

            // Re-add
            if let Err(err) = self.start_hyperlink_once(&link) {
                // TODO: Should we increase the capacity further in this case?
                log::warn!(
                    "(Screen.increaseCapacity) Failed to add cursor hyperlink back to page, err={err:?}"
                );
            }
        }

        // Reload the cursor information because the pin changed.
        // So our page row/cell and so on are all off.
        self.cursor_reload();

        Ok(new_node)
    }

    /// The cell `n` columns right of the cursor.
    pub fn cursor_cell_right(&self, n: size::CellCountInt) -> Cell {
        assert!(self.cursor.x + n < self.pages.cols);
        self.cursor_row_cells()[usize::from(self.cursor.x + n)]
    }

    pub fn cursor_cell_right_mut(&mut self, n: size::CellCountInt) -> &mut Cell {
        assert!(self.cursor.x + n < self.pages.cols);
        let p = self.cursor_pin();
        let page = self.page_mut(p.node);
        let i = page
            .cell_index(usize::from(p.x + n), usize::from(p.y))
            .expect("cursor cell");
        &mut page.cells[i]
    }

    /// The cell `n` columns left of the cursor.
    pub fn cursor_cell_left(&self, n: size::CellCountInt) -> Cell {
        assert!(self.cursor.x >= n);
        self.cursor_row_cells()[usize::from(self.cursor.x - n)]
    }

    pub fn cursor_cell_left_mut(&mut self, n: size::CellCountInt) -> &mut Cell {
        assert!(self.cursor.x >= n);
        let p = self.cursor_pin();
        let page = self.page_mut(p.node);
        let i = page
            .cell_index(usize::from(p.x - n), usize::from(p.y))
            .expect("cursor cell");
        &mut page.cells[i]
    }

    /// The last cell of the row above the cursor row.
    pub fn cursor_cell_end_of_prev(&self) -> Cell {
        assert!(self.cursor.y > 0);

        let mut page_pin = self
            .cursor_pin()
            .up(&self.pages, 1)
            .expect("row above cursor");
        page_pin.x = self.pages.cols - 1;
        *page_pin
            .row_and_cell(&self.pages)
            .expect("end of prev cell")
            .1
    }

    /// Move the cursor right. This is a specialized function that is very fast
    /// if the caller can guarantee we have space to move right (no wrapping).
    pub fn cursor_right(&mut self, n: size::CellCountInt) {
        assert!(self.cursor.x + n < self.pages.cols);

        let mut p = self.cursor_pin();
        p.x += n;
        self.set_cursor_pin(p);
        self.cursor.x += n;
        self.assert_integrity();
    }

    /// Move the cursor left.
    pub fn cursor_left(&mut self, n: size::CellCountInt) {
        assert!(self.cursor.x >= n);

        let mut p = self.cursor_pin();
        p.x -= n;
        self.set_cursor_pin(p);
        self.cursor.x -= n;
        self.assert_integrity();
    }

    /// Move the cursor up.
    ///
    /// Precondition: The cursor is not at the top of the screen.
    pub fn cursor_up(&mut self, n: size::CellCountInt) {
        assert!(self.cursor.y >= n);

        self.cursor.y -= n; // Must be set before cursor_change_pin
        let new = self
            .cursor_pin()
            .up(&self.pages, usize::from(n))
            .expect("row above cursor");
        self.cursor_change_pin(new);
        self.assert_integrity();
    }

    /// The row `n` rows above the cursor row.
    pub fn cursor_row_up(&self, n: size::CellCountInt) -> Row {
        assert!(self.cursor.y >= n);

        let page_pin = self
            .cursor_pin()
            .up(&self.pages, usize::from(n))
            .expect("row above cursor");
        *page_pin.row_and_cell(&self.pages).expect("row").0
    }

    /// Move the cursor down.
    ///
    /// Precondition: The cursor is not at the bottom of the screen.
    pub fn cursor_down(&mut self, n: size::CellCountInt) {
        assert!(u32::from(self.cursor.y) + u32::from(n) < u32::from(self.pages.rows));

        self.cursor.y += n; // Must be set before cursor_change_pin

        // We move the offset into our page list to the next row and then
        // get the pointers to the row/cell and set all the cursor state up.
        let new = self
            .cursor_pin()
            .down(&self.pages, usize::from(n))
            .expect("row below cursor");
        self.cursor_change_pin(new);
        self.assert_integrity();
    }

    /// Move the cursor to some absolute horizontal position.
    pub fn cursor_horizontal_absolute(&mut self, x: size::CellCountInt) {
        assert!(x < self.pages.cols);

        let mut p = self.cursor_pin();
        p.x = x;
        self.set_cursor_pin(p);
        self.cursor.x = x;
        self.assert_integrity();
    }

    /// Move the cursor to some absolute position.
    pub fn cursor_absolute(&mut self, x: size::CellCountInt, y: size::CellCountInt) {
        assert!(x < self.pages.cols);
        assert!(y < self.pages.rows);

        let cur = self.cursor_pin();
        let mut page_pin = match y.cmp(&self.cursor.y) {
            std::cmp::Ordering::Less => cur
                .up(&self.pages, usize::from(self.cursor.y - y))
                .expect("row above cursor"),
            std::cmp::Ordering::Greater => cur
                .down(&self.pages, usize::from(y - self.cursor.y))
                .expect("row below cursor"),
            std::cmp::Ordering::Equal => cur,
        };
        page_pin.x = x;
        self.cursor.x = x; // Must be set before cursor_change_pin
        self.cursor.y = y;
        self.cursor_change_pin(page_pin);
        self.assert_integrity();
    }

    /// Reloads the cursor information into the screen. This is expensive
    /// so it should only be done in cases where the pointers are invalidated
    /// in such a way that its difficult to recover otherwise.
    pub fn cursor_reload(&mut self) {
        // Our tracked pin is ALWAYS accurate, so we derive the active
        // point from the pin. If this returns None it means our pin
        // points outside the active area. In that case, we update the
        // pin to be the top-left.
        let pt = match self.pages.point_from_pin(Tag::Active, self.cursor_pin()) {
            Some(pt) => pt,
            None => {
                let pin = self
                    .pages
                    .pin(Point::Active(Coordinate::default()))
                    .expect("active area exists");
                self.set_cursor_pin(pin);
                self.pages
                    .point_from_pin(Tag::Active, pin)
                    .expect("active area")
            }
        };

        self.cursor.x = pt.coord().x;
        self.cursor.y = pt.coord().y as size::CellCountInt;

        // If we have a style, we need to ensure it is in the page because this
        // method may also be called after a page change.
        if self.cursor.style_id != style::DEFAULT_ID
            && let Err(err) = self.manual_style_update()
        {
            // This failure should not happen because manual_style_update
            // handles page splitting, overflow, and more. This should only
            // happen if we're out of RAM. In this case, we'll just degrade
            // gracefully back to the default style.
            log::error!("failed to update style on cursor reload err={err:?}");
            self.cursor.style = Style::default();
            self.cursor.style_id = 0;
        }
        self.assert_integrity();
    }

    /// Scroll the active area and keep the cursor at the bottom of the screen.
    /// This is a very specialized function but it keeps it fast.
    pub fn cursor_down_scroll(&mut self) -> Result<(), Error> {
        assert!(self.cursor.y == self.pages.rows - 1);

        // Scrolling dirties the images because it updates their placements pins.
        self.kitty_images.dirty = true;

        // If we have no scrollback, then we shift all our rows instead.
        if self.no_scrollback {
            // If we have a single-row screen, we have no rows to shift
            // so our cursor is in the correct place we just have to clear
            // the cells.
            if self.pages.rows == 1 {
                let p = self.cursor_pin();
                let cols = usize::from(self.pages.cols);
                self.clear_cells(p.node, p.y, 0, cols);
                self.cursor_mark_dirty();
            } else {
                // The call to `erase_row` will move the tracked cursor pin up
                // by one row, but we don't actually want that, so we keep the
                // old pin and put it back after calling `erase_row`.
                let old_pin = self.cursor_pin();

                // erase_row will shift everything below it up.
                self.pages.erase_row(Point::Active(Coordinate::default()))?;

                // Note we don't need to mark anything dirty in this branch
                // because erase_row will mark all the rotated rows as dirty
                // in the entire page.

                // We don't use `cursor_change_pin` here because we aren't
                // actually changing the pin, we're keeping it the same.
                self.set_cursor_pin(old_pin);
            }
        } else {
            let old_pin = self.cursor_pin();

            // Grow our pages by one row. The PageList will handle if we need to
            // allocate, prune scrollback, whatever.
            self.pages.grow()?;

            let new_pin = {
                // If our pin page change it means that the page that the pin
                // was on was pruned. In this case, grow() moves the pin to
                // the top-left of the new page. This effectively moves it by
                // one already, we just need to fix up the x value.
                let cur = self.cursor_pin();
                let page_pin = if old_pin.node == cur.node {
                    cur.down(&self.pages, 1).expect("row below cursor")
                } else {
                    let mut pin = cur;
                    pin.x = self.cursor.x;
                    pin
                };

                // These assertions help catch some pagelist math errors. Our
                // x/y should be unchanged after the grow.
                if cfg!(debug_assertions) {
                    let active = self
                        .pages
                        .point_from_pin(Tag::Active, page_pin)
                        .expect("active")
                        .coord();
                    assert_eq!(active.x, self.cursor.x);
                    assert_eq!(active.y, u32::from(self.cursor.y));
                }

                page_pin
            };
            self.cursor_change_pin(new_pin);

            // Our new row is always dirty
            self.cursor_mark_dirty();

            // Clear the new row so it gets our bg color. We only do this
            // if we have a bg color at all.
            if self.cursor.style.bg_color != style::Color::None {
                let p = self.cursor_pin();
                let cols = usize::from(self.pages.cols);
                self.clear_cells(p.node, p.y, 0, cols);
            }
        }

        if self.cursor.style_id != style::DEFAULT_ID {
            // The newly created line needs to be styled according to
            // the bg color if it is set.
            self.fill_cursor_row_bg();
        }
        self.assert_integrity();
        Ok(())
    }

    /// If the cursor style has a background, fill the whole cursor row with
    /// the blank background cell.
    fn fill_cursor_row_bg(&mut self) {
        if let Some(blank_cell) = self.cursor.style.bg_cell() {
            self.cursor_page_row_cells_mut().fill(blank_cell);
        }
    }

    fn cursor_page_row_cells_mut(&mut self) -> &mut [Cell] {
        let p = self.cursor_pin();
        self.page_mut(p.node)
            .get_cells_mut(usize::from(p.y))
            .expect("cursor row")
    }

    /// This scrolls the active area at and above the cursor.
    /// The lines below the cursor are not scrolled.
    pub fn cursor_scroll_above(&mut self) -> Result<(), Error> {
        // We unconditionally mark the cursor row as dirty here because
        // the cursor always changes page rows inside this function, and
        // when that happens it can mean the text in the old row needs to
        // be re-shaped because the cursor splits runs to break ligatures.
        self.cursor_mark_dirty();

        // If the cursor is on the bottom of the screen, its faster to use
        // our specialized function for that case.
        if self.cursor.y == self.pages.rows - 1 {
            return self.cursor_down_scroll();
        }

        // Logic below assumes we always have at least one row that isn't moving
        assert!(self.cursor.y < self.pages.rows - 1);

        // Explanation:
        //  We don't actually move everything that's at or above the cursor row,
        //  since this would require us to shift up our ENTIRE scrollback, which
        //  would be ridiculously expensive. Instead, we insert a new row at the
        //  end of the pagelist (`grow()`), and move everything BELOW the cursor
        //  DOWN by one row. This has the same practical result but it's a whole
        //  lot cheaper in 99% of cases.

        let old_pin = self.cursor_pin();
        if self.pages.grow()?.is_some() {
            self.cursor_scroll_above_rotate()?;
        } else {
            // In this case, it means grow() didn't allocate a new page.

            if Some(self.cursor_pin().node) == self.pages.last {
                // If we're on the last page we can do a very fast path because
                // all the rows we need to move around are within a single page.

                // Note: we don't need to call cursor_change_pin here because
                // the pin page is the same so there is no accounting to do
                // for styles or any of that.
                assert!(old_pin.node == self.cursor_pin().node);
                let down = self
                    .cursor_pin()
                    .down(&self.pages, 1)
                    .expect("row below cursor");
                self.set_cursor_pin(down);

                let page = self.page_mut(down.node);

                // Rotate the rows so that the newly created empty row is at the
                // beginning. e.g. [ 0 1 2 3 ] in to [ 3 0 1 2 ].
                let n = usize::from(page.size.rows);
                page.rows[usize::from(down.y)..n].rotate_right(1);

                // Mark the whole page as dirty.
                //
                // Technically we only need to mark from the cursor row to the
                // end but this is a hot function, so we want to minimize work.
                page.dirty = true;
            } else {
                // We didn't grow pages but our cursor isn't on the last page.
                // In this case we need to do more work because we need to copy
                // elements between pages.
                self.cursor_scroll_above_rotate()?;
            }
        }

        if self.cursor.style_id != style::DEFAULT_ID {
            // The newly created line needs to be styled according to
            // the bg color if it is set.
            self.fill_cursor_row_bg();
        }
        self.assert_integrity();
        Ok(())
    }

    fn cursor_scroll_above_rotate(&mut self) -> Result<(), Error> {
        let down = self
            .cursor_pin()
            .down(&self.pages, 1)
            .expect("row below cursor");
        self.cursor_change_pin(down);
        let cursor_pin = self.cursor_pin();

        // Go through each of the pages following our pin, shift all rows
        // down by one, and copy the last row of the previous page.
        let mut current = self.pages.last.expect("last page");
        while current != cursor_pin.node {
            let prev = self
                .pages
                .node(current)
                .and_then(|n| n.prev)
                .expect("prev page");
            let prev_rows = self.page(prev).size.rows;

            // Rotate the pages down: [ 0 1 2 3 ] => [ 3 0 1 2 ]
            let cur_page = self.page_mut(current);
            let n = usize::from(cur_page.size.rows);
            cur_page.rows[..n].rotate_right(1);

            // Copy the last row of the previous page to the top of current.
            self.pages.clone_row_between(
                Pin {
                    node: prev,
                    y: prev_rows - 1,
                    x: 0,
                    garbage: false,
                },
                Pin {
                    node: current,
                    y: 0,
                    x: 0,
                    garbage: false,
                },
            )?;

            // Mark dirty on the page, since we are dirtying all rows with this.
            self.page_mut(current).dirty = true;
            current = prev;
        }

        // Our current is our cursor page, we need to rotate down from
        // our cursor and clear our row.
        assert!(current == cursor_pin.node);
        let cols = usize::from(self.pages.cols);
        let cur_page = self.page_mut(current);
        let n = usize::from(cur_page.size.rows);
        cur_page.rows[usize::from(cursor_pin.y)..n].rotate_right(1);
        self.clear_cells(current, cursor_pin.y, 0, cols);

        // Mark the whole page as dirty.
        //
        // Technically we only need to mark from the cursor row to the
        // end but this is a hot function, so we want to minimize work.
        self.page_mut(current).dirty = true;
        Ok(())
    }

    /// Move the cursor down if we're not at the bottom of the screen. Otherwise
    /// scroll. Currently only used for testing.
    pub fn cursor_down_or_scroll(&mut self) -> Result<(), Error> {
        if self.cursor.y + 1 < self.pages.rows {
            self.cursor_down(1);
            Ok(())
        } else {
            self.cursor_down_scroll()
        }
    }

    /// Copy another cursor. The cursor can be on any screen but the x/y
    /// must be within our screen bounds.
    ///
    /// `hyperlink`: Copy the hyperlink from the other cursor. If not set,
    /// this will clear our current hyperlink.
    pub fn cursor_copy(&mut self, other: &Cursor, hyperlink: bool) -> Result<(), Error> {
        assert!(other.x < self.pages.cols);
        assert!(other.y < self.pages.rows);

        // End any currently active hyperlink on our cursor.
        self.end_hyperlink();

        let old = self.cursor.clone();
        let mut new = other.clone();

        // Keep our old style ID so it can be properly cleaned up below.
        new.style_id = old.style_id;

        // Hyperlinks will be managed separately below.
        new.hyperlink_id = 0;
        new.hyperlink = None;

        // Keep our old page pin and X/Y because:
        // 1. The old style will need to be cleaned up from the page it's from.
        // 2. The new position navigated to by `cursor_absolute` needs to be in our
        //    own screen.
        new.page_pin = old.page_pin;
        new.x = old.x;
        new.y = old.y;
        self.cursor = new;

        // Call manual style update in order to clean up our old style, if we have
        // one, and also to load the style from the other cursor, if it had one.
        if let Err(e) = self.manual_style_update() {
            self.cursor = old;
            return Err(e);
        }

        // Move to the correct location to match the other cursor.
        self.cursor_absolute(other.x, other.y);

        // If the other cursor had a hyperlink, add it to ours.
        if hyperlink
            && other.hyperlink_id != 0
            && let Some(other_link) = &other.hyperlink
        {
            // And it to our cursor.
            let id = match &other_link.id {
                HyperlinkId::Explicit(id) => Some(id.clone()),
                HyperlinkId::Implicit(_) => None,
            };
            if let Err(err) = self.start_hyperlink(&other_link.uri, id.as_deref()) {
                // This shouldn't happen because start_hyperlink should handle
                // resizing. This only happens if we're truly out of RAM. Degrade
                // to forgetting the hyperlink.
                log::error!("failed to update hyperlink on cursor change err={err:?}");
            }
        }
        Ok(())
    }

    /// Always use this to write to the cursor pin.
    ///
    /// This specifically handles the case when the new pin is on a different
    /// page than the old AND we have a style or hyperlink set. In that case,
    /// we must release our old one and insert the new one, since styles are
    /// stored per-page.
    ///
    /// Note that this can change the cursor pin AGAIN if the process of
    /// setting up our cursor forces a capacity adjustment of the underlying
    /// cursor page, so any references to the page pin should be re-read
    /// from `cursor_pin()` after calling this.
    fn cursor_change_pin(&mut self, new: Pin) {
        let cur = self.cursor_pin();

        // Moving the cursor affects text run splitting (ligatures) so
        // we must mark the old and new page dirty. We do this as long
        // as the pins are not equal
        if !cur.eql(new) {
            self.cursor_mark_dirty();
            new.mark_dirty(&mut self.pages);
        }

        // If our pin is on the same page, then we can just update the pin.
        // We don't need to migrate any state.
        if cur.node == new.node {
            self.set_cursor_pin(new);
            return;
        }

        // If we have an old style then we need to release it from the old page.
        let old_style: Option<Style> = if self.cursor.style_id == style::DEFAULT_ID {
            None
        } else {
            Some(self.cursor.style)
        };
        if old_style.is_some() {
            // Release the style directly from the old page instead of going through
            // manual_style_update, because the cursor position may have already been
            // updated but the pin has not, which would fail integrity checks.
            let id = self.cursor.style_id;
            let _ = self.page_mut(cur.node).styles.release(id);
            self.cursor.style = Style::default();
            self.cursor.style_id = style::DEFAULT_ID;
        }

        // If we have a hyperlink then we need to release it from the old page.
        if self.cursor.hyperlink.is_some() {
            let id = self.cursor.hyperlink_id;
            let _ = self.page_mut(cur.node).hyperlink_set.release(id);
        }

        // Update our pin to the new page
        self.set_cursor_pin(new);

        // On the new page, we need to migrate our style
        if let Some(old_style) = old_style {
            self.cursor.style = old_style;
            if let Err(err) = self.manual_style_update() {
                // This failure should not happen because manual_style_update
                // handles page splitting, overflow, and more. This should only
                // happen if we're out of RAM. In this case, we'll just degrade
                // gracefully back to the default style.
                log::error!("failed to update style on cursor change err={err:?}");
                self.cursor.style = Style::default();
                self.cursor.style_id = 0;
            }
        }

        // On the new page, we need to migrate our hyperlink
        if let Some(link) = self.cursor.hyperlink.take() {
            // So we don't attempt to free any memory in the replaced page.
            self.cursor.hyperlink_id = 0;

            // Re-add
            let id = match &link.id {
                HyperlinkId::Explicit(v) => Some(v.as_slice()),
                HyperlinkId::Implicit(_) => None,
            };
            if let Err(err) = self.start_hyperlink(&link.uri, id) {
                // This shouldn't happen because start_hyperlink should handle
                // resizing. This only happens if we're truly out of RAM. Degrade
                // to forgetting the hyperlink.
                log::error!("failed to update hyperlink on cursor change err={err:?}");
            }
        }
    }

    /// Mark the cursor position as dirty.
    pub fn cursor_mark_dirty(&mut self) {
        let pin = self.cursor_pin();
        // A split can temporarily restore the old cursor pin beyond the live
        // row count. Its allocated row still exists, like Zig's cached page_row.
        self.page_mut(pin.node).rows[usize::from(pin.y)].set_dirty(true);
    }

    /// Reset the cursor row's soft-wrap state and the cursor's pending wrap.
    /// Also handles clearing the spacer head on the cursor row and resetting
    /// the wrap_continuation flag on the next row if necessary.
    ///
    /// NOTE(qwerasd): This method is not scrolling region aware, and cannot be
    /// since it's on Screen not Terminal. This needs to be addressed down the
    /// line. Not an extremely urgent issue since it's an edge case of an edge
    /// case, but not ideal.
    pub fn cursor_reset_wrap(&mut self) {
        // Reset the cursor's pending wrap state
        self.cursor.pending_wrap = false;

        if !self.cursor_row().wrap() {
            return;
        }

        // This row does not wrap and the next row is not wrapped to
        self.cursor_row_mut().set_wrap(false);

        let pin = self.cursor_pin();
        if let Some(next_row) = pin.down(&self.pages, 1) {
            self.page_mut(next_row.node)
                .get_row_mut(usize::from(next_row.y))
                .expect("next row")
                .set_wrap_continuation(false);
        }

        // If the last cell in the row is a spacer head we need to clear it.
        let cols = usize::from(self.page(pin.node).size.cols);
        let cell = self.cursor_row_cells()[cols - 1];
        if cell.wide() == Wide::SpacerHead {
            self.clear_cells(pin.node, pin.y, cols - 1, cols);
        }
    }

    /// Scroll the viewport of the terminal grid.
    pub fn scroll(&mut self, behavior: Scroll) {
        // No matter what, scrolling marks our image state as dirty since
        // it could move placements. If there are no placements or no images
        // this is still a very cheap operation.
        self.kitty_images.dirty = true;

        self.pages.scroll(match behavior {
            Scroll::Active => ListScroll::Active,
            Scroll::Top => ListScroll::Top,
            Scroll::Pin(p) => ListScroll::Pin(p),
            Scroll::Row(v) => ListScroll::Row(v),
            Scroll::DeltaRow(v) => ListScroll::DeltaRow(v),
            Scroll::DeltaPrompt(v) => ListScroll::DeltaPrompt(v),
        });
        self.assert_integrity();
    }

    /// See PageList.scroll_clear. In addition to that, we reset the cursor
    /// to be on top.
    pub fn scroll_clear(&mut self) -> Result<(), Error> {
        self.pages.scroll_clear()?;
        self.cursor_reload();

        // No matter what, scrolling marks our image state as dirty since
        // it could move placements. If there are no placements or no images
        // this is still a very cheap operation.
        self.kitty_images.dirty = true;
        self.assert_integrity();
        Ok(())
    }

    /// Returns true if the viewport is scrolled to the bottom of the screen.
    pub fn viewport_is_bottom(&self) -> bool {
        self.pages.viewport == Viewport::Active
    }

    /// Erase the region specified by tl and br, inclusive. This will physically
    /// erase the rows meaning the memory will be reclaimed (if the underlying
    /// page is empty) and other rows will be shifted up.
    ///
    /// Like Zig's `PageList.eraseRows` this requires that `tl` is valid.
    pub fn erase_rows(&mut self, tl: Point, bl: Option<Point>) {
        // Erase the rows
        self.pages.erase_rows(tl, bl).expect("valid erase range");

        // Just to be safe, reset our cursor since it is possible depending
        // on the points that our active area shifted so our pointers are
        // invalid.
        self.cursor_reload();
        self.assert_integrity();
    }

    /// Clear the region specified by tl and bl, inclusive. Cleared cells are
    /// colored with the current style background color. This will clear all
    /// cells in the rows.
    ///
    /// If protected is true, the protected flag will be respected and only
    /// unprotected cells will be cleared. Otherwise, all cells will be cleared.
    pub fn clear_rows(&mut self, tl: Point, bl: Option<Point>, protected: bool) {
        let chunks: Vec<Chunk> = self
            .pages
            .page_iterator(Direction::RightDown, tl, bl)
            .collect();
        let cols = usize::from(self.pages.cols);
        for chunk in chunks {
            for y in chunk.start..chunk.end {
                let cells_offset = self.page(chunk.node).rows[usize::from(y)].cells();

                // Clear all cells
                if protected {
                    self.clear_unprotected_cells(chunk.node, y, 0, cols);
                    // We need to preserve other row attributes since we only
                    // cleared unprotected cells.
                    self.page_mut(chunk.node).rows[usize::from(y)].set_cells(cells_offset);
                } else {
                    self.clear_cells(chunk.node, y, 0, cols);
                    let mut row = Row::default();
                    row.set_cells(cells_offset);
                    self.page_mut(chunk.node).rows[usize::from(y)] = row;
                }

                self.page_mut(chunk.node).rows[usize::from(y)].set_dirty(true);
            }
        }
        self.assert_integrity();
    }

    /// Clear the cells `[x0, x1)` of row `y` of the given page with the blank
    /// cell.
    ///
    /// This takes care to handle cleaning up graphemes and styles.
    pub fn clear_cells(&mut self, node: NodeId, y: u16, x0: usize, x1: usize) {
        let blank = self.blank_cell();
        let full_row = x1 - x0 == usize::from(self.pages.cols);
        let page = self.page_mut(node);
        let y = usize::from(y);
        let base = page.rows[y].cells();
        assert!(x0 <= x1 && x1 <= usize::from(page.size.cols));
        let row = page.rows[y];

        // If we have managed memory (styles, graphemes, or hyperlinks)
        // in this row then we go cell by cell and clear them if present.
        if row.grapheme() {
            for i in base + x0..base + x1 {
                if page.cells[i].has_grapheme() {
                    page.clear_grapheme(i);
                }
            }

            // If we have no left/right scroll region we can be sure
            // that we've cleared all the graphemes, so we clear the
            // flag, otherwise we ask the page to update the flag.
            if full_row {
                page.rows[y].set_grapheme(false);
            } else {
                page.update_row_grapheme_flag(y);
            }
        }

        if row.hyperlink() {
            for i in base + x0..base + x1 {
                if page.cells[i].hyperlink() {
                    let _ = page.clear_hyperlink(i);
                }
            }

            if full_row {
                page.rows[y].set_hyperlink(false);
            } else {
                page.update_row_hyperlink_flag(y);
            }
        }

        if row.styled() {
            for i in base + x0..base + x1 {
                if page.cells[i].has_styling() {
                    let id = page.cells[i].style_id();
                    let _ = page.styles.release(id);
                }
            }

            if full_row {
                page.rows[y].set_styled(false);
            } else {
                page.update_row_styled_flag(y);
            }
        }

        if row.kitty_virtual_placeholder()
            && full_row
            && !page.cells[base + x0..base + x1]
                .iter()
                .any(|c| c.codepoint() == KITTY_PLACEHOLDER)
        {
            page.rows[y].set_kitty_virtual_placeholder(false);
        }

        page.cells[base + x0..base + x1].fill(blank);
    }

    /// Clear cells but only if they are not protected.
    pub fn clear_unprotected_cells(&mut self, node: NodeId, y: u16, x0: usize, x1: usize) {
        let base = self.page(node).rows[usize::from(y)].cells();
        let mut i = x0;

        while i < x1 {
            while self.page(node).cells[base + i].protected() {
                i += 1;
                if i >= x1 {
                    self.assert_integrity();
                    return;
                }
            }
            let mut j = i + 1;
            while j < x1 && !self.page(node).cells[base + j].protected() {
                j += 1;
            }
            self.clear_cells(node, y, i, j);
            i = j;
        }

        self.assert_integrity();
    }

    /// Clean up boundary conditions where a cell will become discontiguous with
    /// a neighboring cell because either one of them will be moved and/or cleared.
    ///
    /// For performance reasons this is specialized to operate on the cursor row.
    ///
    /// Handles the boundary between the cell at `x` and the cell at `x - 1`.
    ///
    /// So, for example, when moving a region of cells [a, b] (inclusive), call this
    /// function with `x = a` and `x = b + 1`. It is okay if `x` is out of bounds by
    /// 1, this will be interpreted correctly.
    ///
    /// DOES NOT MODIFY ROW WRAP STATE! See `cursor_reset_wrap` for that.
    ///
    /// The following boundary conditions are handled:
    ///
    /// - `x - 1` is a wide character and `x` is a spacer tail:
    ///   o Both cells will be cleared.
    ///   o If `x - 1` is the start of the row and was wrapped from a previous row
    ///     then the previous row is checked for a spacer head, which is cleared if
    ///     present.
    ///
    /// - `x == 0` and is a wide character:
    ///   o If the row is a wrap continuation then the previous row will be checked
    ///     for a spacer head, which is cleared if present.
    ///
    /// - `x == cols` and `x - 1` is a spacer head:
    ///   o `x - 1` will be cleared.
    ///
    /// NOTE(qwerasd): This method is not scrolling region aware, and cannot be
    /// since it's on Screen not Terminal. This needs to be addressed down the
    /// line. Not an extremely urgent issue since it's an edge case of an edge
    /// case, but not ideal.
    pub fn split_cell_boundary(&mut self, x: size::CellCountInt) {
        let pin = self.cursor_pin();
        let cols = self.page(pin.node).size.cols;

        // `x` may be up to an INCLUDING `cols`, since that signifies splitting
        // the boundary to the right of the final cell in the row.
        assert!(x <= cols);

        // [ A B C D E F|]
        //              ^ Boundary between final cell and row end.
        if x == cols {
            if !self.cursor_row().wrap() {
                return;
            }

            // Spacer head at end of wrapped row.
            if self.cursor_row_cells()[usize::from(cols - 1)].wide() == Wide::SpacerHead {
                self.clear_cells(pin.node, pin.y, usize::from(cols - 1), usize::from(cols));
            }

            return;
        }

        // [|A B C D E F ]
        //  ^ Boundary between first cell and row start.
        //
        //  OR
        //
        // [ A|B C D E F ]
        //    ^ Boundary between first cell and second cell.
        //
        // First cell may be a wrapped wide cell with a spacer
        // head on the previous row that needs to be cleared.
        if (x == 0 || x == 1) && self.cursor_row().wrap_continuation() {
            // If the first cell in a row is wide the previous row
            // may have a spacer head which needs to be cleared.
            if self.cursor_row_cells()[0].wide() == Wide::Wide
                && let Some(p_row) = pin.up(&self.pages, 1)
            {
                let p_cols = self.page(p_row.node).size.cols;
                let p_cell = self
                    .page(p_row.node)
                    .get_cells(usize::from(p_row.y))
                    .expect("row")[usize::from(p_cols - 1)];
                if p_cell.wide() == Wide::SpacerHead {
                    self.clear_cells(
                        p_row.node,
                        p_row.y,
                        usize::from(p_cols - 1),
                        usize::from(p_cols),
                    );
                }
            }
        }

        // If x is 0 then we're done.
        if x == 0 {
            return;
        }

        // [ ... X|Y ... ]
        //        ^ Boundary between two cells in the middle of the row.
        {
            assert!(x > 0);
            assert!(x < cols);

            let left = self.cursor_row_cells()[usize::from(x - 1)];
            match left.wide() {
                // There should not be spacer heads in the middle of the row.
                Wide::SpacerHead => unreachable!("spacer head in the middle of a row"),

                // We don't need to do anything for narrow cells or spacer tails.
                Wide::Narrow | Wide::SpacerTail => {}

                // A wide char would be split, so must be cleared.
                Wide::Wide => {
                    self.clear_cells(pin.node, pin.y, usize::from(x - 1), usize::from(x + 1));
                }
            }
        }
    }

    /// Returns the blank cell to use when doing terminal operations that
    /// require preserving the bg color.
    #[inline]
    pub fn blank_cell(&self) -> Cell {
        if self.cursor.style_id == style::DEFAULT_ID {
            return Cell::default();
        }
        self.cursor.style.bg_cell().unwrap_or_default()
    }
}

impl Screen {
    /// Resize the screen. The rows or cols can be bigger or smaller.
    ///
    /// If this returns an error, the screen is left in a likely garbage state.
    /// It is very hard to undo this operation without blowing up our memory
    /// usage. The only way to recover is to reset the screen. The only way
    /// this really fails is if page allocation is required and fails, which
    /// probably means the system is in trouble anyways. I'd like to improve this
    /// in the future but it is not a priority particularly because this scenario
    /// (resize) is difficult.
    pub fn resize(&mut self, opts: Resize) -> Result<(), Error> {
        // No matter what we mark our image state as dirty
        self.kitty_images.dirty = true;

        // Release the cursor style while resizing just
        // in case the cursor ends up on a different page.
        let cursor_style = self.cursor.style;
        self.cursor.style = Style::default();
        self.manual_style_update()
            .expect("releasing to the default style cannot fail");

        // We need to insert a tracked pin for our saved cursor so we can
        // modify its X/Y for reflow.
        let saved_cursor_pin = match self.saved_cursor {
            Some(sc) => match self.pages.pin(Point::Active(Coordinate {
                x: sc.x,
                y: u32::from(sc.y),
            })) {
                Some(pin) => match self.pages.track_pin(pin) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        self.restore_cursor_style(cursor_style);
                        self.assert_integrity();
                        return Err(e);
                    }
                },
                None => None,
            },
            None => None,
        };

        let result = self.resize_inner(opts, saved_cursor_pin);
        if let Some(p) = saved_cursor_pin {
            self.pages.untrack_pin(p);
        }

        // Restore the cursor style.
        self.restore_cursor_style(cursor_style);
        self.assert_integrity();
        result
    }

    fn restore_cursor_style(&mut self, cursor_style: Style) {
        self.cursor.style = cursor_style;
        if let Err(err) = self.manual_style_update() {
            // This failure should not happen because manual_style_update
            // handles page splitting, overflow, and more. This should only
            // happen if we're out of RAM. In this case, we'll just degrade
            // gracefully back to the default style.
            log::error!("failed to update style on cursor reload err={err:?}");
            self.cursor.style = Style::default();
            self.cursor.style_id = 0;
        }
    }

    fn resize_inner(
        &mut self,
        opts: Resize,
        saved_cursor_pin: Option<TrackedPin>,
    ) -> Result<(), Error> {
        // If we have a hyperlink, release it from the old page
        // and then we need to re-add it to the new page. This needs
        // to happen because resize below typically reallocates a
        // new page so the old hyperlink is invalid.
        let hyperlink_ = self.cursor.hyperlink.take();
        if self.cursor.hyperlink_id != 0 {
            // Note we do NOT use end_hyperlink because we want to keep
            // our allocated hyperlink valid.
            let id = self.cursor.hyperlink_id;
            let node = self.cursor_pin().node;
            let _ = self.page_mut(node).hyperlink_set.release(id);
            self.cursor.hyperlink_id = 0;
        }

        // If our cursor is on a prompt or input line, clear it so the shell can
        // redraw it. This works with OSC 133 semantic prompts.
        //
        // We check cursor.semantic_content rather than page_row.semantic_prompt
        // because some shells (e.g., Nu) mark input areas with OSC 133 B but don't
        // mark continuation lines with k=s. If the input spans multiple lines and
        // continuation lines are unmarked, checking only page_row.semantic_prompt
        // would miss them. By checking semantic_content, we assume that if the
        // cursor is on anything other than command output, we're at a prompt/input
        // line and should clear from there.
        if opts.prompt_redraw != Redraw::False
            && self.cursor.semantic_content != SemanticContent::Output
        {
            let cols = usize::from(self.pages.cols);
            match opts.prompt_redraw {
                Redraw::False => unreachable!(),

                // For `Last`, only clear the current line where the cursor is.
                // For `True`, clear all prompt lines starting from the beginning.
                Redraw::Last => {
                    let p = self.cursor_pin();
                    self.clear_cells(p.node, p.y, 0, cols);
                }

                Redraw::True => {
                    let start = self
                        .cursor_pin()
                        .prompt_iterator(&self.pages, Direction::LeftUp, None)
                        .next();
                    if let Some(start) = start {
                        // Clear cells from our start down. We replace it with spaces,
                        // and do not physically erase the rows (erase_rows) because the
                        // shell is going to expect this space to be available.
                        let rows: Vec<Pin> = start
                            .row_iterator(&self.pages, Direction::RightDown, None)
                            .collect();
                        for pin in rows {
                            self.clear_cells(pin.node, pin.y, 0, cols);
                        }
                    } else {
                        // This should never happen because prompt_iterator should always
                        // find a prompt if we already verified our row is some kind of
                        // prompt.
                        log::warn!("cursor on prompt line but promptIterator found no prompt");
                    }
                }
            }
        }

        // Perform the resize operation.
        self.pages.resize(crate::page_list::Resize {
            rows: Some(opts.rows),
            cols: Some(opts.cols),
            reflow: opts.reflow,
            cursor: Some(crate::page_list::Cursor {
                x: self.cursor.x,
                y: self.cursor.y,
            }),
        })?;

        // If we have no scrollback and we shrunk our rows, we must explicitly
        // erase our history. This is because PageList always keeps at least
        // a page size of history.
        if self.no_scrollback {
            self.pages
                .erase_rows(Point::History(Coordinate::default()), None)
                .expect("valid history range");
        }

        // If our cursor was updated, we do a full reload so all our cursor
        // state is correct.
        self.cursor_reload();

        // If we reflowed a saved cursor, update it.
        if let Some(p) = saved_cursor_pin {
            // This should never fail because a non-null saved_cursor_pin
            // implies a non-null saved_cursor.
            let pin = self.pages.tracked_pin(p);
            let pt = pin.and_then(|pin| self.pages.point_from_pin(Tag::Active, pin));
            if let Some(sc) = self.saved_cursor.as_mut() {
                if let Some(pt) = pt {
                    sc.x = pt.coord().x;
                    sc.y = pt.coord().y as size::CellCountInt;

                    // If we had pending wrap set and we're no longer at the end of
                    // the line, we unset the pending wrap and move the cursor to
                    // reflect the correct next position.
                    if sc.pending_wrap && sc.x != opts.cols - 1 {
                        sc.pending_wrap = false;
                        sc.x += 1;
                    }
                } else {
                    // I think this can happen if the screen is resized to be
                    // less rows or less cols and our saved cursor moves outside
                    // the active area. In this case, there isn't anything really
                    // reasonable we can do so we just move the cursor to the
                    // top-left. It may be reasonable to also move the cursor to
                    // match the primary cursor. Any behavior is fine since this is
                    // totally unspecified.
                    sc.x = 0;
                    sc.y = 0;
                    sc.pending_wrap = false;
                }
            }
        }

        // Fix up our hyperlink if we had one.
        if let Some(link) = hyperlink_ {
            let id = match &link.id {
                HyperlinkId::Explicit(v) => Some(v.as_slice()),
                HyperlinkId::Implicit(_) => None,
            };
            if let Err(err) = self.start_hyperlink(&link.uri, id) {
                // This shouldn't happen because start_hyperlink should handle
                // resizing. This only happens if we're truly out of RAM. Degrade
                // to forgetting the hyperlink.
                log::error!("failed to update hyperlink on resize err={err:?}");
            }
        }

        Ok(())
    }

    /// Set a style attribute for the current cursor.
    ///
    /// If the style can't be set due to any internal errors (memory-related),
    /// then this will revert back to the existing style and return an error.
    pub fn set_attribute(&mut self, attr: sgr::Attribute<'_>) -> Result<(), Error> {
        // If we fail to set our style for any reason, we should revert
        // back to the old style. If we fail to do that, we revert back to
        // the default style.
        let old_style = self.cursor.style;

        use sgr::Attribute as A;
        let st = &mut self.cursor.style;
        match attr {
            A::Unset => *st = Style::default(),
            A::Bold => st.flags.bold = true,
            A::ResetBold => {
                // Bold and faint share the same SGR code for this
                st.flags.bold = false;
                st.flags.faint = false;
            }
            A::Italic => st.flags.italic = true,
            A::ResetItalic => st.flags.italic = false,
            A::Faint => st.flags.faint = true,
            A::Underline(v) => st.flags.underline = v,
            A::UnderlineColor(rgb) => st.underline_color = style::Color::Rgb(rgb),
            A::UnderlineColor256(idx) => st.underline_color = style::Color::Palette(idx),
            A::ResetUnderlineColor => st.underline_color = style::Color::None,
            A::Overline => st.flags.overline = true,
            A::ResetOverline => st.flags.overline = false,
            A::Blink => st.flags.blink = true,
            A::ResetBlink => st.flags.blink = false,
            A::Inverse => st.flags.inverse = true,
            A::ResetInverse => st.flags.inverse = false,
            A::Invisible => st.flags.invisible = true,
            A::ResetInvisible => st.flags.invisible = false,
            A::Strikethrough => st.flags.strikethrough = true,
            A::ResetStrikethrough => st.flags.strikethrough = false,
            A::DirectColorFg(rgb) => st.fg_color = style::Color::Rgb(rgb),
            A::DirectColorBg(rgb) => st.bg_color = style::Color::Rgb(rgb),
            A::Fg8(n) => st.fg_color = style::Color::Palette(n.0),
            A::Bg8(n) => st.bg_color = style::Color::Palette(n.0),
            A::ResetFg => st.fg_color = style::Color::None,
            A::ResetBg => st.bg_color = style::Color::None,
            A::Fg8Bright(n) => st.fg_color = style::Color::Palette(n.0),
            A::Bg8Bright(n) => st.bg_color = style::Color::Palette(n.0),
            A::Fg256(idx) => st.fg_color = style::Color::Palette(idx),
            A::Bg256(idx) => st.bg_color = style::Color::Palette(idx),
            A::Unknown(_) => return Ok(()),
        }

        if let Err(e) = self.manual_style_update() {
            self.cursor.style = old_style;
            if let Err(err) = self.manual_style_update() {
                log::warn!("setAttribute error restoring old style after failure err={err:?}");
                self.cursor.style = Style::default();
                let _ = self.manual_style_update();
            }
            return Err(e);
        }
        Ok(())
    }

    /// Call this whenever you manually change the cursor style.
    ///
    /// This function can NOT fail if the cursor style is changing to the
    /// default style.
    ///
    /// If this returns an error, the style change did not take effect and
    /// the cursor style is reverted back to the default. The only scenario
    /// this returns an error is if there is a physical memory allocation failure
    /// or if there is no possible way to increase style capacity to store
    /// the style.
    ///
    /// This function WILL split pages as necessary to accommodate the new style.
    /// So if OutOfSpace is returned, it means that even after splitting the page
    /// there was still no room for the new style.
    pub fn manual_style_update(&mut self) -> Result<(), Error> {
        let result = self.manual_style_update_inner();
        self.assert_integrity();
        result
    }

    fn manual_style_update_inner(&mut self) -> Result<(), Error> {
        let node = self.cursor_pin().node;

        // Release our previous style if it was not default.
        if self.cursor.style_id != style::DEFAULT_ID {
            let id = self.cursor.style_id;
            let _ = self.page_mut(node).styles.release(id);
        }

        // If our new style is the default, just reset to that
        if self.cursor.style.is_default() {
            self.cursor.style_id = style::DEFAULT_ID;
            return Ok(());
        }

        // Clear the cursor style ID to prevent weird things from happening
        // if the page capacity has to be adjusted which would end up calling
        // manual_style_update again.
        //
        // This also ensures that if anything fails below, we fall back to
        // clearing our style.
        self.cursor.style_id = style::DEFAULT_ID;

        // After setting the style, we need to update our style map.
        // Note that we COULD lazily do this in print. We should look into
        // if that makes a meaningful difference. Our priority is to keep print
        // fast because setting a ton of styles that do nothing is uncommon
        // and weird.
        let st = self.cursor.style;
        let id = match self.page_mut(node).styles.add(st) {
            Ok(id) => id,
            Err(err) => {
                // Our style map is full or needs to be rehashed, so we need to
                // increase style capacity (or rehash).
                let adjustment = match err {
                    crate::ref_counted_set::AddError::OutOfMemory => Some(IncreaseCapacity::Styles),
                    crate::ref_counted_set::AddError::NeedsRehash => None,
                    other => return Err(other.into()),
                };
                let node = match self.increase_capacity(self.cursor_pin().node, adjustment) {
                    Ok(n) => n,
                    Err(Error::OutOfSpace) => {
                        // Out of space, we need to split the page. Split wherever
                        // is using less capacity and hope that works. If it doesn't
                        // work, we tried.
                        self.split_for_capacity(self.cursor_pin())?;
                        self.cursor_pin().node
                    }
                    Err(e) => return Err(e),
                };

                match self.page_mut(node).styles.add(st) {
                    Ok(id) => id,
                    Err(crate::ref_counted_set::AddError::NeedsRehash) => {
                        // This should be impossible because we rehash above
                        // and rehashing should never result in a duplicate. But
                        // we don't want to simply hard crash so log it and
                        // clear our style.
                        log::error!("style rehash resulted in needs rehash");
                        return Ok(());
                    }
                    Err(crate::ref_counted_set::AddError::OutOfMemory) => {
                        // This shouldn't happen because increase_capacity is
                        // guaranteed to increase our capacity by at least one and
                        // we only need one space, but again, I don't want to crash
                        // here so let's log loudly and reset.
                        log::error!("style addition failed after capacity increase");
                        return Err(Error::OutOfMemory);
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        };

        self.cursor.style_id = id;
        Ok(())
    }

    /// Split at the given pin so that the pinned row moves to the page
    /// with less used capacity after the split.
    ///
    /// The primary use case for this is to handle IncreaseCapacityError
    /// OutOfSpace conditions where we need to split the page in order
    /// to make room for more managed memory.
    ///
    /// If the caller cares about where the pin moves to, they should
    /// setup a tracked pin before calling this and then check that.
    /// In many calling cases, the input pin is tracked (e.g. the cursor
    /// pin).
    ///
    /// If this returns OOM then its a system OOM. If this returns OutOfSpace
    /// then it means the page can't be split further.
    fn split_for_capacity(&mut self, pin: Pin) -> Result<(), Error> {
        // Get our capacities. We include our target row because its
        // capacity will be preserved.
        let (bytes_above, bytes_below) = {
            let page = self.page(pin.node);
            let above =
                Page::layout(page.exact_row_capacity(0, usize::from(pin.y) + 1)?).total_size;
            let below = Page::layout(
                page.exact_row_capacity(usize::from(pin.y), usize::from(page.size.rows))?,
            )
            .total_size;
            (above, below)
        };

        // We need to track the old cursor pin because if our split
        // moves the cursor pin we need to update our accounting.
        let old_cursor = self.cursor_pin();

        // If our bytes above are less than bytes below, we move the pin
        // to split down one since splitting includes the pinned row in
        // the new node.
        self.pages.split(if bytes_above < bytes_below {
            pin.down(&self.pages, 1).unwrap_or(pin)
        } else {
            pin
        })?;

        // Cursor didn't change nodes, we're done.
        if self.cursor_pin().node == old_cursor.node {
            return Ok(());
        }

        // Cursor changed, we need to restore the old pin then use
        // cursor_change_pin to move to the new pin. The old node is guaranteed
        // to still exist, just not the row.
        let new_cursor = self.cursor_pin();
        self.set_cursor_pin(old_cursor);
        self.cursor_change_pin(new_cursor);
        Ok(())
    }

    /// Append a grapheme to the given cell (column `x`) within the current
    /// cursor row.
    pub fn append_grapheme(&mut self, x: size::CellCountInt, cp: u32) -> Result<(), Error> {
        let result = {
            let p = self.cursor_pin();
            self.page_mut(p.node)
                .append_grapheme(usize::from(x), usize::from(p.y), cp)
        };
        match result {
            Err(Error::GraphemeAllocOutOfMemory | Error::GraphemeMapOutOfMemory) => {
                // Adjust our capacity. This will update our cursor page pin and
                // force us to reload. The cell is addressed by column so it stays
                // valid.
                let node = self.cursor_pin().node;
                self.increase_capacity(node, Some(IncreaseCapacity::GraphemeBytes))?;

                let p = self.cursor_pin();
                self.page_mut(p.node)
                    .append_grapheme(usize::from(x), usize::from(p.y), cp)
                    .inspect_err(|_| {
                        // This should never happen because we just increased capacity.
                        // Log loudly but still return an error so we don't just
                        // crash.
                        log::error!("grapheme append failed after capacity increase");
                    })
            }
            other => other,
        }
    }

    /// Start the hyperlink state. Future cells will be marked as hyperlinks with
    /// this state. Note that various terminal operations may clear the hyperlink
    /// state, such as switching screens (alt screen).
    pub fn start_hyperlink(&mut self, uri: &[u8], id: Option<&[u8]>) -> Result<(), Error> {
        // Create our pending entry.
        let implicit = id.is_none();
        let link = Hyperlink {
            uri: uri.to_vec(),
            id: match id {
                Some(id) => HyperlinkId::Explicit(id.to_vec()),
                None => {
                    let v = self.cursor.hyperlink_implicit_id;
                    self.cursor.hyperlink_implicit_id = v.wrapping_add(1);
                    HyperlinkId::Implicit(v)
                }
            },
        };

        // Loop until we have enough page memory to add the hyperlink
        loop {
            let adjustment = match self.start_hyperlink_once(&link) {
                Ok(()) => return Ok(()),
                // strings table is out of memory, adjust it up
                Err(Error::StringsOutOfMemory) => Some(IncreaseCapacity::StringBytes),
                // hyperlink set is out of memory, adjust it up
                Err(Error::SetOutOfMemory) => Some(IncreaseCapacity::HyperlinkBytes),
                // hyperlink set is too full, rehash it
                Err(Error::SetNeedsRehash) => None,
                Err(e) => {
                    if implicit {
                        self.cursor.hyperlink_implicit_id =
                            self.cursor.hyperlink_implicit_id.wrapping_sub(1);
                    }
                    return Err(e);
                }
            };

            let node = self.cursor_pin().node;
            if let Err(e) = self.increase_capacity(node, adjustment) {
                if implicit {
                    self.cursor.hyperlink_implicit_id =
                        self.cursor.hyperlink_implicit_id.wrapping_sub(1);
                }
                return Err(e);
            }

            self.assert_integrity();
        }
    }

    /// This is like start_hyperlink but if we have to adjust page capacities
    /// this returns an error. This is useful so that we unwind all the
    /// previous state and try again.
    fn start_hyperlink_once(&mut self, source: &Hyperlink) -> Result<(), Error> {
        // End any prior hyperlink
        self.end_hyperlink();

        // Allocate our new Hyperlink entry in non-page memory. This
        // lets us quickly get access to URI, ID.
        let link = Box::new(source.dupe());

        // Insert the hyperlink into page memory
        let node = self.cursor_pin().node;
        let id: hyperlink::Id = self.page_mut(node).insert_hyperlink((*link).clone())?;

        // Save it all
        self.cursor.hyperlink = Some(link);
        self.cursor.hyperlink_id = id;
        Ok(())
    }

    /// End the hyperlink state so that future cells aren't part of the
    /// current hyperlink (if any). This is safe to call multiple times.
    pub fn end_hyperlink(&mut self) {
        // If we have no hyperlink state then do nothing
        if self.cursor.hyperlink_id == 0 {
            assert!(self.cursor.hyperlink.is_none());
            return;
        }

        // Release the old hyperlink state. If there are cells using the
        // hyperlink this will work because the creation creates a reference
        // and all additional cells create a new reference. This release will
        // just release our initial reference.
        //
        // If the ref count reaches zero the set will not delete the item
        // immediately; it is kept around in case it is used again (this is
        // how RefCountedSet works). This causes some memory fragmentation but
        // is fine because if it is ever pruned the context deleted callback
        // will be called.
        let id = self.cursor.hyperlink_id;
        let node = self.cursor_pin().node;
        let _ = self.page_mut(node).hyperlink_set.release(id);
        self.cursor.hyperlink_id = 0;
        self.cursor.hyperlink = None;
    }

    /// Set the current hyperlink state on the current cell.
    pub fn cursor_set_hyperlink(&mut self) -> Result<(), Error> {
        assert!(self.cursor.hyperlink_id != 0);

        loop {
            let p = self.cursor_pin();
            let id = self.cursor.hyperlink_id;
            let page = self.page_mut(p.node);
            match page.set_hyperlink(usize::from(p.x), usize::from(p.y), id) {
                Ok(()) => {
                    // Success, increase the refcount for the hyperlink.
                    let _ = page.hyperlink_set.r#use(id);
                    return Ok(());
                }

                // hyperlink_map is out of space, realloc the page to be larger
                Err(Error::HyperlinkMapOutOfMemory) => {
                    // Probe the URI allocation before rebuilding, just as
                    // cursorSetHyperlink does in the reference implementation.
                    while let Some(link) = &self.cursor.hyperlink {
                        let len = link.uri.len();
                        let node = self.cursor_pin().node;
                        if self.page_mut(node).can_allocate_string(len) {
                            break;
                        }
                        self.increase_capacity(node, Some(IncreaseCapacity::StringBytes))?;
                    }
                    let node = self.cursor_pin().node;
                    self.increase_capacity(node, Some(IncreaseCapacity::HyperlinkBytes))?;

                    // Retry
                    //
                    // We check that the cursor hyperlink hasn't been destroyed
                    // by the capacity adjustment first though- since despite the
                    // terrible code above, that can still apparently happen ._.
                    if self.cursor.hyperlink_id > 0 {
                        continue;
                    }
                    return Ok(());
                }

                Err(e) => return Err(e),
            }
        }
    }

    /// Modify the semantic content type of the cursor. This should
    /// be preferred over setting it manually since it handles all the
    /// proper accounting.
    pub fn cursor_set_semantic_content(&mut self, t: SemanticContentSet) {
        match t {
            SemanticContentSet::Output => {
                self.cursor.semantic_content = SemanticContent::Output;
                self.cursor.semantic_content_clear_eol = false;
            }

            SemanticContentSet::Input(clear) => {
                self.cursor.semantic_content = SemanticContent::Input;
                self.cursor.semantic_content_clear_eol = match clear {
                    InputClear::ClearExplicit => false,
                    InputClear::ClearEol => true,
                };
            }

            SemanticContentSet::Prompt(kind) => {
                self.semantic_prompt.seen = true;
                self.cursor.semantic_content = SemanticContent::Prompt;
                self.cursor.semantic_content_clear_eol = false;
                self.cursor_row_mut().set_semantic_prompt(match kind {
                    PromptKind::Initial | PromptKind::Right => RowPrompt::Prompt,
                    PromptKind::Continuation | PromptKind::Secondary => {
                        RowPrompt::PromptContinuation
                    }
                });
            }
        }
    }

    /// Set the selection to the given selection. If this is a tracked selection
    /// then the screen will take ownership of the selection. If this is untracked
    /// then the screen will convert it to tracked internally. This will automatically
    /// untrack the prior selection (if any).
    ///
    /// Set the selection to None to clear any previous selection.
    ///
    /// This is always recommended over setting `selection` directly. Beyond
    /// managing memory for you, it also performs safety checks that the selection
    /// is always tracked.
    pub fn select(&mut self, sel: Option<Selection>) -> Result<(), Error> {
        let Some(sel) = sel else {
            self.clear_selection();
            return Ok(());
        };

        // If this selection is untracked then we track it.
        let tracked_sel = if sel.tracked() {
            sel
        } else {
            sel.track(&mut self.pages)?
        };

        // Untrack prior selection
        if let Some(old) = self.selection {
            old.deinit(&mut self.pages);
        }
        self.selection = Some(tracked_sel);
        self.dirty.selection = true;
        Ok(())
    }

    /// Same as select(None) but can't fail.
    pub fn clear_selection(&mut self) {
        if let Some(sel) = self.selection {
            sel.deinit(&mut self.pages);
            self.dirty.selection = true;
        }
        self.selection = None;
    }
}

fn is_ws(whitespace: &[u32], cp: u32) -> bool {
    whitespace.contains(&cp)
}

impl Screen {
    fn pin_cell(&self, p: Pin) -> Cell {
        *p.row_and_cell(&self.pages).expect("valid pin").1
    }

    fn pin_row(&self, p: Pin) -> Row {
        *p.row_and_cell(&self.pages).expect("valid pin").0
    }

    fn pin_row_cells(&self, p: Pin) -> &[Cell] {
        p.cells(&self.pages, CellSubset::All).expect("valid pin")
    }

    fn pin_cols(&self, p: Pin) -> u16 {
        self.page(p.node).size.cols
    }

    /// Returns the raw text associated with a selection. This will unwrap
    /// soft-wrapped edges.
    ///
    /// For more flexibility, use a ScreenFormatter directly.
    pub fn selection_string(&self, opts: SelectionString) -> String {
        self.selection_string_impl(opts, None)
    }

    /// Same as `selection_string` but also returns a map from every byte of
    /// the string to the pin it came from (Zig: `SelectionString.map`).
    pub fn selection_string_map(&self, opts: SelectionString) -> StringMap {
        let mut pins = Vec::new();
        let string = self.selection_string_impl(opts, Some(&mut pins));
        StringMap { string, map: pins }
    }

    fn selection_string_impl(
        &self,
        opts: SelectionString,
        pin_map: Option<&mut Vec<Pin>>,
    ) -> String {
        // Create a formatter and use that to emit our text.
        let mut formatter = ScreenFormatter::new(
            self,
            FormatOptions {
                unwrap: true,
                trim: opts.trim,
                ..FormatOptions::plain()
            },
        );
        formatter.content = FormatContent::Selection(Some(opts.sel));

        // If we have a string map, we need to set that up.
        formatter.pin_map = pin_map;

        formatter
            .format_string()
            .expect("formatting to memory cannot fail")
    }

    /// Select the line under the given point. This will select across soft-wrapped
    /// lines and will omit the leading and trailing whitespace. If the point is
    /// over whitespace but the line has non-whitespace characters elsewhere, the
    /// line will be selected.
    pub fn select_line(&self, opts: SelectLine<'_>) -> Option<Selection> {
        let list = &self.pages;

        // Get the current point semantic prompt state since that determines
        // boundary conditions too. This makes it so that line selection can
        // only happen within the same prompt state. For example, if you triple
        // click output, but the shell uses spaces to soft-wrap to the prompt
        // then the selection will stop prior to the prompt. See issue #1329.
        let semantic_prompt_state: Option<SemanticContent> = if opts.semantic_prompt_boundary {
            Some(self.pin_cell(opts.pin).semantic_content())
        } else {
            None
        };

        // The real start of the row is the first row in the soft-wrap.
        let start_pin: Pin = 'start_pin: {
            let mut it = opts.pin.row_iterator(list, Direction::LeftUp, None);
            let mut it_prev: Pin = it.next().expect("self row"); // skip self

            // First, check the current row for semantic boundaries before the clicked position.
            if let Some(v) = semantic_prompt_state {
                let cells = self.pin_row_cells(it_prev);
                // Scan backwards from clicked position to find where our content starts
                for i in 0..=usize::from(opts.pin.x) {
                    let x_rev = usize::from(opts.pin.x) - i;
                    if cells[x_rev].semantic_content() != v {
                        let mut copy = it_prev;
                        copy.x = (x_rev + 1) as u16;
                        break 'start_pin copy;
                    }
                }

                // No boundary found before clicked position on current row.
                // If row doesn't wrap from above, start is at column 0.
                // Otherwise, continue checking previous rows.
            }

            for p in it {
                let row = self.pin_row(p);

                if !row.wrap() {
                    let mut copy = it_prev;
                    copy.x = 0;
                    break 'start_pin copy;
                }

                if let Some(v) = semantic_prompt_state {
                    // We need to check every cell in this row in reverse
                    // order since we're going up and back.
                    let cells = self.pin_row_cells(p);
                    for x in 0..cells.len() {
                        let x_rev = cells.len() - 1 - x;
                        let cell = cells[x_rev];
                        if cell.semantic_content() != v {
                            break 'start_pin it_prev;
                        }
                        it_prev = p;
                        it_prev.x = x_rev as u16;
                    }

                    continue;
                }

                it_prev = p;
            }

            let mut copy = it_prev;
            copy.x = 0;
            copy
        };

        // The real end of the row is the final row in the soft-wrap.
        let end_pin: Pin = 'end_pin: {
            for p in opts.pin.row_iterator(list, Direction::RightDown, None) {
                let row = self.pin_row(p);
                let cols = self.pin_cols(p);

                if let Some(v) = semantic_prompt_state {
                    // We need to check every cell in this row
                    let cells = self.pin_row_cells(p);

                    // If this is our pin row we can start from our x because
                    // the start_pin logic already found the real start.
                    let start_offset = if p.node == opts.pin.node && p.y == opts.pin.y {
                        usize::from(opts.pin.x)
                    } else {
                        0
                    };

                    // Handle the zero case specially because if the first
                    // col doesn't match then we end at the end of the prior
                    // row. But if this is the first row, we can't go back,
                    // so we scan forward to find where our content ends.
                    if start_offset == 0 && cells[0].semantic_content() != v {
                        let mut prev = p.up(list, 1)?;
                        prev.x = cols - 1;
                        break 'end_pin prev;
                    }

                    // For every other case, we end at the prior cell.
                    for (x, cell) in cells.iter().enumerate().skip(start_offset) {
                        if cell.semantic_content() != v {
                            let mut copy = p;
                            copy.x = (x - 1) as u16;
                            break 'end_pin copy;
                        }
                    }
                }

                if !row.wrap() {
                    let mut copy = p;
                    copy.x = cols - 1;
                    break 'end_pin copy;
                }
            }

            return None;
        };

        // Go forward from the start to find the first non-whitespace character.
        let start: Pin = 'start: {
            let Some(whitespace) = opts.whitespace else {
                break 'start start_pin;
            };
            for p in start_pin.cell_iterator(list, Direction::RightDown, Some(end_pin)) {
                let cell = self.pin_cell(p);
                if !cell.has_text() {
                    continue;
                }

                // Non-empty means we found it.
                if is_ws(whitespace, cell.codepoint()) {
                    continue;
                }

                break 'start p;
            }

            return None;
        };

        // Go backward from the end to find the first non-whitespace character.
        let end: Pin = 'end: {
            let Some(whitespace) = opts.whitespace else {
                break 'end end_pin;
            };
            for p in end_pin.cell_iterator(list, Direction::LeftUp, Some(start_pin)) {
                let cell = self.pin_cell(p);
                if !cell.has_text() {
                    continue;
                }

                // Non-empty means we found it.
                if is_ws(whitespace, cell.codepoint()) {
                    continue;
                }

                break 'end p;
            }

            return None;
        };

        Some(Selection::new(start, end, false))
    }

    /// Return the selection for all contents on the screen. Surrounding
    /// whitespace is omitted. If there is no selection, this returns None.
    pub fn select_all(&self) -> Option<Selection> {
        let whitespace = DEFAULT_SELECT_WHITESPACE;
        let screen = Point::Screen(Coordinate::default());

        let start = self
            .pages
            .cell_iterator(Direction::RightDown, screen, None)
            .find(|p| {
                let cell = self.pin_cell(*p);
                cell.has_text() && !is_ws(whitespace, cell.codepoint())
            })?;

        let end = self
            .pages
            .cell_iterator(Direction::LeftUp, screen, None)
            .find(|p| {
                let cell = self.pin_cell(*p);
                cell.has_text() && !is_ws(whitespace, cell.codepoint())
            })?;

        Some(Selection::new(start, end, false))
    }

    /// Select the nearest word to start point that is between start_pt and
    /// end_pt (inclusive). Because it selects "nearest" to start point, start
    /// point can be before or after end point.
    ///
    /// The boundary_codepoints parameter should be a slice of codepoints that
    /// mark word boundaries, passed through to select_word.
    ///
    /// TODO: test this
    pub fn select_word_between(
        &self,
        start: Pin,
        end: Pin,
        boundary_codepoints: &[u32],
    ) -> Option<Selection> {
        let list = &self.pages;
        let dir = if start.before(list, end) {
            Direction::RightDown
        } else {
            Direction::LeftUp
        };
        for pin in start.cell_iterator(list, dir, Some(end)) {
            // Boundary conditions
            match dir {
                Direction::RightDown => {
                    if end.before(list, pin) {
                        return None;
                    }
                }
                Direction::LeftUp => {
                    if pin.before(list, end) {
                        return None;
                    }
                }
            }

            // If we found a word, then return it
            if let Some(sel) = self.select_word(pin, boundary_codepoints) {
                return Some(sel);
            }
        }

        None
    }

    /// Select the word under the given point. A word is any consecutive series
    /// of characters that are exclusively whitespace or exclusively non-whitespace.
    /// A selection can span multiple physical lines if they are soft-wrapped.
    ///
    /// This will return None if a selection is impossible. The only scenario
    /// this happens is if the point pt is outside of the written screen space.
    ///
    /// The boundary_codepoints parameter should be a slice of codepoints that
    /// mark word boundaries. This is expected to be pre-parsed from the config.
    pub fn select_word(&self, pin: Pin, boundary_codepoints: &[u32]) -> Option<Selection> {
        let list = &self.pages;

        // If our cell is empty we can't select a word, because we can't select
        // areas where the screen is not yet written.
        let start_cell = self.pin_cell(pin);
        if !start_cell.has_text() {
            return None;
        }

        // Determine if we are a boundary or not to determine what our boundary is.
        let expect_boundary = is_ws(boundary_codepoints, start_cell.codepoint());

        // Go forwards to find our end boundary
        let end: Pin = 'end: {
            let mut it = pin.cell_iterator(list, Direction::RightDown, None);
            let mut prev = it.next().expect("start cell"); // Consume one, our start
            for p in it {
                let (row, cell) = {
                    let (r, c) = p.row_and_cell(list).expect("valid pin");
                    (*r, *c)
                };

                // If we reached an empty cell its always a boundary
                if !cell.has_text() {
                    break 'end prev;
                }

                // If we do not match our expected set, we hit a boundary
                let this_boundary = is_ws(boundary_codepoints, cell.codepoint());
                if this_boundary != expect_boundary {
                    break 'end prev;
                }

                // If we are going to the next row and it isn't wrapped, we
                // return the previous.
                if p.x == self.pin_cols(p) - 1 && !row.wrap() {
                    break 'end p;
                }

                prev = p;
            }

            prev
        };

        // Go backwards to find our start boundary
        let start: Pin = 'start: {
            let mut it = pin.cell_iterator(list, Direction::LeftUp, None);
            let mut prev = it.next().expect("start cell"); // Consume one, our start
            for p in it {
                let (row, cell) = {
                    let (r, c) = p.row_and_cell(list).expect("valid pin");
                    (*r, *c)
                };

                // If we are going to the next row and it isn't wrapped, we
                // return the previous.
                if p.x == self.pin_cols(p) - 1 && !row.wrap() {
                    break 'start prev;
                }

                // If we reached an empty cell its always a boundary
                if !cell.has_text() {
                    break 'start prev;
                }

                // If we do not match our expected set, we hit a boundary
                let this_boundary = is_ws(boundary_codepoints, cell.codepoint());
                if this_boundary != expect_boundary {
                    break 'start prev;
                }

                prev = p;
            }

            prev
        };

        Some(Selection::new(start, end, false))
    }

    /// Select the command output under the given point. The limits of the output
    /// are determined by semantic prompt information provided by shell integration.
    /// A selection can span multiple physical lines if they are soft-wrapped.
    ///
    /// This will return None if a selection is impossible:
    ///  - the point pt is outside of the written screen space.
    ///  - the point pt is on a prompt / input line.
    pub fn select_output(&self, pin: Pin) -> Option<Selection> {
        let list = &self.pages;

        // If our pin right now is not on output, then we return nothing.
        if self.pin_cell(pin).semantic_content() != SemanticContent::Output {
            return None;
        }

        // Get the post prior prompt from this pin. This is the prompt whose
        // output we'll be capturing.
        let prompt_pin: Pin = 'prompt: {
            // If we have a prompt above this point (including this point),
            // then thats the prompt we want to capture output from.
            if let Some(p) = pin.prompt_iterator(list, Direction::LeftUp, None).next() {
                break 'prompt p;
            }

            // If we don't have a prompt, then we assume that we're
            // capturing all the output up to the next prompt.
            let next = pin
                .prompt_iterator(list, Direction::RightDown, None)
                .next()?;

            // We'll capture from the start of the screen to just above
            // the prompt and will trim the trailing whitespace.
            let start_pin = self.pages.get_top_left(Tag::Screen)?;
            let mut end_pin = next.up(list, 1)?;
            end_pin.x = self.pin_cols(end_pin) - 1;
            for p in end_pin.cell_iterator(list, Direction::LeftUp, Some(start_pin)) {
                end_pin = p;
                if self.pin_cell(p).has_text() {
                    break;
                }
            }

            return Some(Selection::new(start_pin, end_pin, false));
        };

        // Grab our content
        let (hl_start, mut hl_end) = self
            .pages
            .highlight_semantic_content(prompt_pin, SemanticContent::Output)?;

        // Trim our trailing whitespace
        for p in hl_end.cell_iterator(list, Direction::LeftUp, Some(hl_start)) {
            hl_end = p;
            if self.pin_cell(p).has_text() {
                break;
            }
        }

        Some(Selection::new(hl_start, hl_end, false))
    }

    /// Returns an iterator to move through the soft-wrapped lines starting
    /// from pin.
    pub fn line_iterator(&self, start: Pin) -> LineIterator<'_> {
        LineIterator {
            screen: self,
            current: Some(start),
        }
    }

    /// Determine the inputs necessary to move the cursor to the given
    /// click location within a prompt input area.
    ///
    /// If the cursor isn't currently at a prompt input location, this
    /// returns no movement.
    ///
    /// This feature depends on well-behaved OSC133 shell integration. Specifically,
    /// this only moves over designated input areas (OSC 133 B). It is assumed
    /// that the shell will only move the cursor to input cells, so prompt cells
    /// and other blank cells are ignored as part of the movement calculation.
    pub fn prompt_click_move(&self, click_pin: Pin) -> PromptClickMove {
        // If we're not at an input cell with our cursor, no movement will
        // ever be possible.
        if self.cursor.semantic_content != SemanticContent::Input
            && self.cursor_cell().semantic_content() != SemanticContent::Input
        {
            return PromptClickMove::ZERO;
        }

        match self.semantic_prompt.click {
            // None doesn't support movement and click_events must use a
            // different mechanism (SGR mouse events) that callers must handle.
            SemanticClick::None | SemanticClick::ClickEvents => PromptClickMove::ZERO,
            SemanticClick::Cl(cl) => match cl {
                // All of these currently use dumb line-based navigation.
                // But eventually we'll support more.
                Click::Line
                | Click::Multiple
                | Click::ConservativeVertical
                | Click::SmartVertical => self.prompt_click_line(click_pin),
            },
        }
    }

    /// Determine the inputs required to move from the cursor to the given
    /// click location. If the cursor isn't currently at a prompt input
    /// location, this will return zero.
    ///
    /// This currently only supports moving a single line.
    fn prompt_click_line(&self, click_pin: Pin) -> PromptClickMove {
        let list = &self.pages;

        // If our click pin is our cursor pin, no movement is needed.
        // Do this early so we can assume later that they are different.
        let cursor_pin = self.cursor_pin();
        if cursor_pin.eql(click_pin) {
            return PromptClickMove::ZERO;
        }

        // If our cursor is before our click, we're only emitting right inputs.
        if cursor_pin.before(list, click_pin) {
            let mut count: usize = 0;

            // We go row-by-row because soft-wrapped rows are still a single
            // line to a shell, so we can't just look at our page row.
            'row_it: for row_pin in
                cursor_pin.row_iterator(list, Direction::RightDown, Some(click_pin))
            {
                let row = self.pin_row(row_pin);
                let cells = self.pin_row_cells(row_pin);

                // Determine if this row is our cursor.
                let is_cursor_row = row_pin.node == cursor_pin.node && row_pin.y == cursor_pin.y;

                // If this is not the cursor row, verify it's still part of the
                // continuation of our starting prompt.
                if !is_cursor_row && row.semantic_prompt() != RowPrompt::PromptContinuation {
                    break;
                }

                // Determine where our input starts.
                let start_x: usize = 'start_x: {
                    // If this is our cursor row then we start after the cursor.
                    if is_cursor_row {
                        break 'start_x usize::from(cursor_pin.x) + 1;
                    }

                    // Otherwise, we start at the first input cell, because
                    // we expect the shell to properly translate arrows across
                    // lines to the start of the input. Some shells indent
                    // where input starts on subsequent lines so we must do
                    // this.
                    for (x, cell) in cells.iter().enumerate() {
                        if cell.semantic_content() == SemanticContent::Input {
                            break 'start_x x;
                        }
                    }

                    // We never found an input cell, so we need to move to the
                    // next row.
                    cells.len()
                };

                // Iterate over the input cells and assume arrow keys only
                // jump to input cells.
                for (x, cell) in cells.iter().enumerate().skip(start_x) {
                    // Ignore non-input cells, but allow breaks. We assume
                    // the shell will translate arrow keys to only input
                    // areas.
                    if cell.semantic_content() != SemanticContent::Input {
                        continue;
                    }

                    // Increment our input count
                    count += 1;

                    // If this is our target, we're done.
                    if row_pin.node == click_pin.node
                        && row_pin.y == click_pin.y
                        && x == usize::from(click_pin.x)
                    {
                        break 'row_it;
                    }
                }

                // If this row isn't soft-wrapped, we need to break out
                // because line based moving only handles single lines.
                // We're done!
                if !row.wrap() {
                    // If we never found our pin, that means we clicked further
                    // right/beyond it. If we're already on a non-empty input cell
                    // then we add one so we can move to the newest, empty cell
                    // at the end, matching typical editor behavior.
                    if self.cursor_cell().semantic_content() == SemanticContent::Input {
                        count += 1;
                    }

                    break;
                }
            }

            return PromptClickMove {
                left: 0,
                right: count,
            };
        }

        // Otherwise, cursor is after click, so we're emitting left inputs.
        let mut count: usize = 0;

        // We go row-by-row because soft-wrapped rows are still a single
        // line to a shell, so we can't just look at our page row.
        'row_it: for row_pin in cursor_pin.row_iterator(list, Direction::LeftUp, Some(click_pin)) {
            let row = self.pin_row(row_pin);
            let cells = self.pin_row_cells(row_pin);

            // Determine the length of the cells we look at in this row.
            let end_len: usize =
                // If this is our cursor row then we end before the cursor.
                if row_pin.node == cursor_pin.node && row_pin.y == cursor_pin.y {
                    usize::from(cursor_pin.x)
                } else {
                    // Otherwise, we end at the last cell in the row.
                    cells.len()
                };

            // Iterate backwards over the input cells.
            for rev_x in 0..end_len {
                let x = end_len - 1 - rev_x;
                let cell = cells[x];

                // Ignore non-input cells.
                if cell.semantic_content() != SemanticContent::Input {
                    continue;
                }

                // Increment our input count
                count += 1;

                // If this is our target, we're done.
                if row_pin.node == click_pin.node
                    && row_pin.y == click_pin.y
                    && x == usize::from(click_pin.x)
                {
                    break 'row_it;
                }
            }

            // If this row is not a wrap continuation, then break out
            if !row.wrap_continuation() {
                break;
            }
        }

        PromptClickMove {
            left: count,
            right: 0,
        }
    }

    /// Dump the screen to a string.
    pub fn dump_string(&self, out: &mut String, opts: DumpString) {
        // Create a formatter and use that to emit our text.
        let mut formatter = ScreenFormatter::new(
            self,
            FormatOptions {
                unwrap: opts.unwrap,
                trim: false,
                ..FormatOptions::plain()
            },
        );

        // Set up the selection based on the pins
        let tl = opts.tl;
        let br = opts
            .br
            .unwrap_or_else(|| self.pages.get_bottom_right(Tag::Screen).expect("screen"));
        formatter.content = FormatContent::Selection(Some(Selection::new(
            tl, br, false, // not rectangle
        )));

        // Emit
        out.push_str(
            &formatter
                .format_string()
                .expect("formatting to memory cannot fail"),
        );
    }

    /// You should use dump_string, this is a restricted version mostly for
    /// legacy and convenience reasons for unit tests.
    pub fn dump_string_alloc(&self, tl: Point) -> Result<String, Error> {
        let mut out = String::new();
        self.dump_string(
            &mut out,
            DumpString {
                tl: self
                    .pages
                    .get_top_left(tl.tag())
                    .ok_or(Error::InvalidCoordinate)?,
                br: Some(
                    self.pages
                        .get_bottom_right(tl.tag())
                        .ok_or(Error::InvalidCoordinate)?,
                ),
                unwrap: false,
            },
        );
        Ok(out)
    }

    /// You should use dump_string, this is a restricted version mostly for
    /// legacy and convenience reasons for unit tests.
    pub fn dump_string_alloc_unwrapped(&self, tl: Point) -> Result<String, Error> {
        let mut out = String::new();
        self.dump_string(
            &mut out,
            DumpString {
                tl: self
                    .pages
                    .get_top_left(tl.tag())
                    .ok_or(Error::InvalidCoordinate)?,
                br: Some(
                    self.pages
                        .get_bottom_right(tl.tag())
                        .ok_or(Error::InvalidCoordinate)?,
                ),
                unwrap: true,
            },
        );
        Ok(out)
    }

    /// This is basically a really jank version of Terminal.printString. We
    /// have to reimplement it here because we want a way to print to the screen
    /// to test it but don't want all the features of Terminal.
    pub fn test_write_string(&mut self, text: &str) -> Result<(), Error> {
        for c in text.chars() {
            let c = c as u32;

            // Explicit newline forces a new row
            if c == u32::from('\n') {
                self.cursor_down_or_scroll()?;
                self.cursor_horizontal_absolute(0);
                self.cursor.pending_wrap = false;
                if self.cursor.semantic_content_clear_eol {
                    self.cursor_set_semantic_content(SemanticContentSet::Output);
                } else {
                    match self.cursor.semantic_content {
                        SemanticContent::Output => {}
                        SemanticContent::Prompt | SemanticContent::Input => {
                            self.cursor_row_mut()
                                .set_semantic_prompt(RowPrompt::PromptContinuation);
                        }
                    }
                }
                continue;
            }

            let width: usize = if c <= 0xFF {
                1
            } else {
                usize::from(ghostty_unicode::table().get(c).width)
            };
            if width == 0 {
                let mut x = self.cursor.x - 1;
                match self.cursor_cell_left(1).wide() {
                    Wide::Narrow | Wide::Wide => {}
                    Wide::SpacerHead => unreachable!(),
                    Wide::SpacerTail => x = self.cursor.x - 2,
                }

                let p = self.cursor_pin();
                self.page_mut(p.node)
                    .append_grapheme(usize::from(x), usize::from(p.y), c)?;
                continue;
            }

            if self.cursor.pending_wrap {
                assert!(self.cursor.x == self.pages.cols - 1);
                self.cursor.pending_wrap = false;
                self.cursor_row_mut().set_wrap(true);
                self.cursor_down_or_scroll()?;
                self.cursor_horizontal_absolute(0);
                self.cursor_row_mut().set_wrap_continuation(true);
                match self.cursor.semantic_content {
                    SemanticContent::Output => {}
                    SemanticContent::Input | SemanticContent::Prompt => {
                        self.cursor_row_mut()
                            .set_semantic_prompt(RowPrompt::PromptContinuation);
                    }
                }
            }

            assert!(width == 1 || width == 2);
            match width {
                1 => {
                    let mut cell = Cell::default();
                    cell.set_content_tag(ContentTag::Codepoint);
                    cell.set_codepoint(c);
                    cell.set_style_id(self.cursor.style_id);
                    cell.set_protected(self.cursor.protected);
                    cell.set_semantic_content(self.cursor.semantic_content);
                    *self.cursor_cell_mut() = cell;

                    // If we have a ref-counted style, increase.
                    if self.cursor.style_id != style::DEFAULT_ID {
                        let id = self.cursor.style_id;
                        let _ = self.cursor_page_mut().styles.r#use(id);
                        self.cursor_row_mut().set_styled(true);
                    }

                    // If we have a hyperlink, add it to the cell.
                    if self.cursor.hyperlink_id > 0 {
                        self.cursor_set_hyperlink()?;
                    }
                }

                2 => {
                    // Need a wide spacer head
                    if self.cursor.x == self.pages.cols - 1 {
                        let mut cell = Cell::default();
                        cell.set_content_tag(ContentTag::Codepoint);
                        cell.set_codepoint(0);
                        cell.set_wide(Wide::SpacerHead);
                        cell.set_protected(self.cursor.protected);
                        cell.set_semantic_content(self.cursor.semantic_content);
                        *self.cursor_cell_mut() = cell;

                        // If we have a hyperlink, add it to the cell.
                        if self.cursor.hyperlink_id > 0 {
                            self.cursor_set_hyperlink()?;
                        }

                        self.cursor_row_mut().set_wrap(true);
                        self.cursor_down_or_scroll()?;
                        self.cursor_horizontal_absolute(0);
                        self.cursor_row_mut().set_wrap_continuation(true);
                    }

                    // Write our wide char
                    let mut cell = Cell::default();
                    cell.set_content_tag(ContentTag::Codepoint);
                    cell.set_codepoint(c);
                    cell.set_style_id(self.cursor.style_id);
                    cell.set_wide(Wide::Wide);
                    cell.set_protected(self.cursor.protected);
                    cell.set_semantic_content(self.cursor.semantic_content);
                    *self.cursor_cell_mut() = cell;

                    // If we have a hyperlink, add it to the cell.
                    if self.cursor.hyperlink_id > 0 {
                        self.cursor_set_hyperlink()?;
                    }

                    // Write our tail
                    self.cursor_right(1);
                    let mut cell = Cell::default();
                    cell.set_content_tag(ContentTag::Codepoint);
                    cell.set_codepoint(0);
                    cell.set_wide(Wide::SpacerTail);
                    cell.set_protected(self.cursor.protected);
                    cell.set_semantic_content(self.cursor.semantic_content);
                    *self.cursor_cell_mut() = cell;

                    // If we have a hyperlink, add it to the cell.
                    if self.cursor.hyperlink_id > 0 {
                        self.cursor_set_hyperlink()?;
                    }

                    // If we have a ref-counted style, increase twice.
                    if self.cursor.style_id != style::DEFAULT_ID {
                        let id = self.cursor.style_id;
                        let _ = self.cursor_page_mut().styles.r#use(id);
                        let _ = self.cursor_page_mut().styles.r#use(id);
                        self.cursor_row_mut().set_styled(true);
                    }
                }

                _ => unreachable!(),
            }

            if self.cursor.x + 1 < self.pages.cols {
                self.cursor_right(1);
            } else {
                self.cursor.pending_wrap = true;
            }
        }
        Ok(())
    }
}

/// An iterator over the soft-wrapped lines of a screen.
pub struct LineIterator<'a> {
    screen: &'a Screen,
    current: Option<Pin>,
}

impl Iterator for LineIterator<'_> {
    type Item = Selection;

    fn next(&mut self) -> Option<Selection> {
        let current = self.current?;
        let Some(result) = self.screen.select_line(SelectLine {
            pin: current,
            whitespace: None,
            semantic_prompt_boundary: false,
        }) else {
            self.current = None;
            return None;
        };

        self.current = result.end(&self.screen.pages).down(&self.screen.pages, 1);
        Some(result)
    }
}

#[cfg(test)]
mod tests;
