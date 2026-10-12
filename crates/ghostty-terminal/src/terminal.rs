//! The primary terminal emulation structure. This represents a single
//! "terminal" containing a grid of characters and exposes various operations
//! on that grid. This also maintains the scrollback buffer.
//!
//! Port of `ZIG/src/terminal/Terminal.zig`.
//!
//! # Differences from the Zig API
//!
//! * `screens.active` is `screens.active()` / `screens.active_mut()`.
//! * Allocators are dropped; fallible operations return
//!   [`Result<_, Error>`](Error) (`page::Error`).
//! * `switch_screen` returns the *key* of the previous screen instead of a
//!   pointer to it (`self.screens.get(old)` gives the screen back).
//! * `print_attributes` returns a `String` instead of writing into a buffer.
#![allow(
    clippy::manual_assert_eq,
    clippy::single_match_else,
    clippy::items_after_statements,
    clippy::too_many_lines
)]
use ghostty_vt::{
    ansi::{ProtectedMode, StatusDisplay},
    charsets::{self, Charset, Slots},
    color::{DynamicPalette, DynamicRgb},
    csi,
    kitty::graphics_command as kitty_graphics_command,
    modes::{Mode, ModePacked, ModeState},
    mouse_shape::MouseShape,
    osc::{
        self,
        semantic_prompt::{Action as PromptAction, Redraw},
    },
    sgr,
};

pub use crate::page::Error;
use crate::{
    page::{Cell, ContentTag, Page, SemanticContent, SemanticPrompt as RowPrompt, Wide},
    page_list::{Direction, IncreaseCapacity, Pin},
    point::{Coordinate, Point},
    screen::{
        CharsetState,
        InputClear,
        Options as ScreenOptions,
        Resize as ScreenResize,
        SavedCursor,
        Screen,
        Scroll,
        SemanticClick,
        SemanticContentSet as SetSemantic,
    },
    screen_set::{Key as ScreenKey, ScreenSet},
    size,
    style::{self, Color as StyleColor, Style},
    tabstops::Tabstops,
};

/// Default tabstop interval
const TABSTOP_INTERVAL: usize = 8;

/// The unicode codepoint for kitty graphics placeholders.
use crate::kitty::graphics_unicode::PLACEHOLDER as KITTY_PLACEHOLDER;

/// The set of screens behind this terminal (e.g. primary vs alternate).
#[derive(Debug)]
pub struct Terminal {
    /// The set of screens behind this terminal (e.g. primary vs alternate).
    pub screens: ScreenSet,

    /// Whether we're currently writing to the status line (DECSASD and DECSSDT).
    /// We don't support a status line currently so we just black hole this
    /// data so that it doesn't mess up our main display.
    pub status_display: StatusDisplay,

    /// Where the tabstops are.
    pub tabstops: Tabstops,

    /// The size of the terminal.
    pub rows: size::CellCountInt,
    pub cols: size::CellCountInt,

    /// The size of the screen in pixels. This is used for pty events and images
    pub width_px: u32,
    pub height_px: u32,

    /// The current scrolling region.
    pub scrolling_region: ScrollingRegion,

    /// The last reported pwd, if any.
    pub pwd: Vec<u8>,

    /// The color state for this terminal.
    pub colors: Colors,

    /// The previous printed character. This is used for the repeat previous
    /// char CSI (ESC [ <n> b).
    pub previous_char: Option<u32>,

    /// The modes that this terminal currently has active.
    pub modes: ModeState,

    /// The most recently set mouse shape for the terminal.
    pub mouse_shape: MouseShape,

    /// These are just a packed set of flags we may set on the terminal.
    pub flags: Flags,
}

/// The `flags` struct of the terminal (a packed struct in Zig).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    /// This supports a Kitty extension where programs using semantic
    /// prompts (OSC133) can annotate their new prompts with `redraw=0` to
    /// disable clearing the prompt on resize.
    pub shell_redraws_prompt: Redraw,

    /// This is set via ESC[4;2m. Any other modify key mode just sets
    /// this to false and we act in mode 1 by default.
    pub modify_other_keys_2: bool,

    /// The mouse event mode and format. These are set to the last
    /// set mode in modes. You can't get the right event/format to use
    /// based on modes alone because modes don't show you what order
    /// this was called so we have to track it separately.
    pub mouse_event: MouseEvents,
    pub mouse_format: MouseFormat,

    /// Set via the XTSHIFTESCAPE sequence. If true (XTSHIFTESCAPE = 1)
    /// then we want to capture the shift key for the mouse protocol
    /// if the configuration allows it.
    pub mouse_shift_capture: MouseShiftCapture,

    /// True if the window is focused.
    pub focused: bool,

    /// True if the terminal is in a password entry mode. This is set
    /// to true based on termios state.
    pub password_input: bool,

    /// True if the terminal should perform selection scrolling.
    pub selection_scroll: bool,

    /// Dirty flag used only by the search thread. The renderer is expected
    /// to set this to true if the viewport was dirty as it was rendering.
    /// This is used by the search thread to more efficiently re-search the
    /// viewport and active area.
    ///
    /// Since the renderer is going to inspect the viewport/active area ANYWAYS,
    /// this lets our search thread do less work and hold the lock less time,
    /// resulting in more throughput for everything.
    pub search_viewport_dirty: bool,

    /// Dirty flags for the renderer.
    pub dirty: Dirty,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            shell_redraws_prompt: Redraw::True,
            modify_other_keys_2: false,
            mouse_event: MouseEvents::None,
            mouse_format: MouseFormat::X10,
            mouse_shift_capture: MouseShiftCapture::Null,
            focused: true,
            password_input: false,
            selection_scroll: false,
            search_viewport_dirty: false,
            dirty: Dirty::default(),
        }
    }
}

/// `mouse_shift_capture` tri-state (Zig `enum(u2) { null, false, true }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseShiftCapture {
    #[default]
    Null,
    False,
    True,
}

/// The various color configurations a terminal maintains and that can
/// be set dynamically via OSC, with defaults usually coming from a
/// configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub background: DynamicRgb,
    pub foreground: DynamicRgb,
    pub cursor: DynamicRgb,
    pub palette: DynamicPalette,
}

impl Colors {
    pub const DEFAULT: Colors = Colors {
        background: DynamicRgb::UNSET,
        foreground: DynamicRgb::UNSET,
        cursor: DynamicRgb::UNSET,
        palette: DynamicPalette::DEFAULT,
    };
}

impl Default for Colors {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// This is a set of dirty flags the renderer can use to determine
/// what parts of the screen need to be redrawn. It is up to the renderer
/// to clear these flags.
///
/// This only contains dirty flags for terminal state, not for the screen
/// state. The screen state has its own dirty flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dirty {
    /// Set when the color palette is modified in any way.
    pub palette: bool,

    /// Set when the reverse colors mode is modified.
    pub reverse_colors: bool,

    /// Screen clear of some kind. This can be due to a screen change,
    /// erase display, etc.
    pub clear: bool,

    /// Set when the pre-edit is modified.
    pub preedit: bool,
}

/// The event types that can be reported for mouse-related activities.
/// These are all mutually exclusive (hence in a single enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseEvents {
    #[default]
    None = 0,
    X10 = 1,    // 9
    Normal = 2, // 1000
    Button = 3, // 1002
    Any = 4,    // 1003
}

impl MouseEvents {
    /// Returns true if this event sends motion events.
    pub fn motion(self) -> bool {
        self == MouseEvents::Button || self == MouseEvents::Any
    }
}

/// The format of mouse events when enabled.
/// These are all mutually exclusive (hence in a single enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseFormat {
    #[default]
    X10 = 0,
    Utf8 = 1,      // 1005
    Sgr = 2,       // 1006
    Urxvt = 3,     // 1015
    SgrPixels = 4, // 1016
}

/// Scrolling region is the area of the screen designated where scrolling
/// occurs. When scrolling the screen, only this viewport is scrolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollingRegion {
    // Top and bottom of the scroll region (0-indexed)
    // Precondition: top < bottom
    pub top: size::CellCountInt,
    pub bottom: size::CellCountInt,

    // Left/right scroll regions.
    // Precondition: right > left
    // Precondition: right <= cols - 1
    pub left: size::CellCountInt,
    pub right: size::CellCountInt,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub cols: size::CellCountInt,
    pub rows: size::CellCountInt,
    pub max_scrollback: usize,
    pub colors: Colors,

    /// The default mode state. When the terminal gets a reset, it
    /// will revert back to this state.
    pub default_modes: ModePacked,
}

impl Options {
    pub fn new(cols: size::CellCountInt, rows: size::CellCountInt) -> Self {
        Self {
            cols,
            rows,
            max_scrollback: 10_000,
            colors: Colors::DEFAULT,
            default_modes: ModePacked::default(),
        }
    }
}

/// The semantic prompt type. This is used when tracking a line type and
/// requires integration with the shell. By default, we mark a line as "none"
/// meaning we don't know what type it is.
///
/// See: <https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticPrompt {
    Prompt,
    PromptContinuation,
    Input,
    Command,
}

/// Options for scrolling the viewport of the terminal grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollViewport {
    /// Scroll to the top of the scrollback
    Top,

    /// Scroll to the bottom, i.e. the top of the active area
    Bottom,

    /// Scroll by some delta amount, up is negative.
    Delta(isize),
}

/// The modes for DECCOLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeccolmMode {
    Cols80 = 0,
    Cols132 = 1,
}

/// Modal screen changes. These map to the literal terminal
/// modes to enable or disable alternate screen modes. They each
/// have subtle behaviors so we define them as an enum here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchScreenMode {
    /// Legacy alternate screen mode. This goes to the alternate
    /// screen or primary screen and only copies the cursor. The
    /// screen is not erased.
    M47,

    /// Alternate screen mode where the alternate screen is cleared
    /// on exit. The primary screen is never cleared. The cursor is
    /// copied.
    M1047,

    /// Save primary screen cursor, switch to alternate screen,
    /// and clear the alternate screen on entry. On exit,
    /// do not clear the screen, and restore the cursor on the
    /// primary screen.
    M1049,
}

fn map_tabstops_err(_: std::collections::TryReserveError) -> Error {
    Error::OutOfMemory
}

/// The adjustment needed to retry a failed partial row clone (insert/delete
/// lines cross-page slow path).
#[allow(clippy::option_option)]
fn capacity_adjustment(err: Error) -> Option<Option<IncreaseCapacity>> {
    Some(match err {
        // Rehash the sets
        Error::StyleSetNeedsRehash | Error::SetNeedsRehash => None,
        // Increase style memory
        Error::StyleSetOutOfMemory => Some(IncreaseCapacity::Styles),
        // Increase string memory
        Error::StringsOutOfMemory => Some(IncreaseCapacity::StringBytes),
        // Increase hyperlink memory
        Error::SetOutOfMemory | Error::HyperlinkMapOutOfMemory => {
            Some(IncreaseCapacity::HyperlinkBytes)
        }
        // Increase grapheme memory
        Error::GraphemeMapOutOfMemory | Error::GraphemeAllocOutOfMemory => {
            Some(IncreaseCapacity::GraphemeBytes)
        }
        _ => return None,
    })
}

/// To be called before shifting a row (as in insert_lines and delete_lines)
///
/// Takes care of boundary conditions such as potentially split wide chars
/// across scrolling region boundaries and orphaned spacer heads at line
/// ends.
fn row_will_be_shifted(sr: ScrollingRegion, cols: size::CellCountInt, page: &mut Page, y: usize) {
    let base = page.rows[y].cells();

    // If our scrolling region includes the rightmost column then we
    // need to turn any spacer heads in to normal empty cells, since
    // once we move them they no longer correspond with soft-wrapped
    // wide characters.
    //
    // If it contains either of the 2 leftmost columns, then the wide
    // characters in the first column which may be associated with a
    // spacer head will be either moved or cleared, so we also need
    // to turn the spacer heads in to empty cells in that case.
    if sr.right == cols - 1 || sr.left < 2 {
        let end = base + usize::from(page.size.cols) - 1;
        if page.cells[end].wide() == Wide::SpacerHead {
            page.cells[end].set_wide(Wide::Narrow);
        }
    }

    // If the leftmost or rightmost cells of our scrolling region
    // are parts of wide chars, we need to clear the cells' contents
    // since they'd be split by the move.
    let left = base + usize::from(sr.left);
    let right = base + usize::from(sr.right);

    if page.cells[left].wide() == Wide::SpacerTail {
        let wide = left - 1;
        if page.cells[wide].has_grapheme() {
            page.clear_grapheme(wide);
            page.update_row_grapheme_flag(y);
        }
        page.cells[wide].set_codepoint(0);
        page.cells[wide].set_wide(Wide::Narrow);
        page.cells[left].set_wide(Wide::Narrow);
    }

    if page.cells[right].wide() == Wide::Wide {
        let tail = right + 1;
        if page.cells[right].has_grapheme() {
            page.clear_grapheme(right);
            page.update_row_grapheme_flag(y);
        }
        page.cells[right].set_codepoint(0);
        page.cells[right].set_wide(Wide::Narrow);
        page.cells[tail].set_wide(Wide::Narrow);
    }
}

impl Terminal {
    /// Initialize a new terminal.
    pub fn new(opts: Options) -> Result<Terminal, Error> {
        let cols = opts.cols;
        let rows = opts.rows;

        let screens = ScreenSet::new(ScreenOptions::new(cols, rows, opts.max_scrollback))?;

        Ok(Terminal {
            cols,
            rows,
            screens,
            status_display: StatusDisplay::Main,
            tabstops: Tabstops::new(usize::from(cols), TABSTOP_INTERVAL)
                .map_err(map_tabstops_err)?,
            width_px: 0,
            height_px: 0,
            scrolling_region: ScrollingRegion {
                top: 0,
                bottom: rows - 1,
                left: 0,
                right: cols - 1,
            },
            pwd: Vec::new(),
            colors: opts.colors,
            previous_char: None,
            modes: ModeState {
                values: opts.default_modes,
                saved: ModePacked::default(),
                default: opts.default_modes,
            },
            mouse_shape: MouseShape::Text,
            flags: Flags::default(),
        })
    }

    /// Return a stream that can process VT streams and update this
    /// terminal state. The streams will only process read-only data that
    /// modifies terminal state. Sequences that query or otherwise require
    /// output will be ignored.
    pub fn vt_stream(&mut self) -> crate::stream_readonly::Stream<'_> {
        crate::stream_readonly::Stream::new_alloc(self.vt_handler())
    }

    /// This is the handler-side only for vt_stream.
    pub fn vt_handler(&mut self) -> crate::stream_readonly::Handler<'_> {
        crate::stream_readonly::Handler::new(self)
    }

    /// Shorthand for `self.screens.active()`.
    #[inline]
    pub fn screen(&self) -> &Screen {
        self.screens.active()
    }

    /// Shorthand for `self.screens.active_mut()`.
    #[inline]
    pub fn screen_mut(&mut self) -> &mut Screen {
        self.screens.active_mut()
    }

    /// Print UTF-8 encoded string to the terminal.
    pub fn print_string(&mut self, s: &str) -> Result<(), Error> {
        for ch in s.chars() {
            match ch {
                '\n' => {
                    self.carriage_return();
                    self.linefeed()?;
                }

                _ => self.print(u32::from(ch))?,
            }
        }
        Ok(())
    }

    /// Print the previous printed character a repeated amount of times.
    pub fn print_repeat(&mut self, count_req: usize) -> Result<(), Error> {
        if let Some(c) = self.previous_char {
            let count = count_req.max(1);
            for _ in 0..count {
                self.print(c)?;
            }
        }
        Ok(())
    }

    pub fn print(&mut self, c: u32) -> Result<(), Error> {
        // If we're not on the main display, do nothing for now
        if self.status_display != StatusDisplay::Main {
            return Ok(());
        }

        // After doing any printing, wrapping, scrolling, etc. we want to ensure
        // that our screen remains in a consistent state.
        let result = self.print_inner(c);
        self.screen().assert_integrity();
        result
    }

    fn print_inner(&mut self, c: u32) -> Result<(), Error> {
        // Our right margin depends where our cursor is now.
        let right_limit = if self.screen().cursor.x > self.scrolling_region.right {
            self.cols
        } else {
            self.scrolling_region.right + 1
        };

        // Perform grapheme clustering if grapheme support is enabled (mode 2027).
        // This is MUCH slower than the normal path so the conditional below is
        // purposely ordered in least-likely to most-likely so we can drop out
        // as quickly as possible.
        if c > 255
            && self.modes.get(Mode::GraphemeCluster)
            && self.screen().cursor.x > 0
            && self.print_grapheme(c, right_limit)?
        {
            return Ok(());
        }

        // Determine the width of this character so we can handle
        // non-single-width characters properly. We have a fast-path for
        // byte-sized characters since they're so common. We can ignore
        // control characters because they're always filtered prior.
        let width: usize = if c <= 0xFF {
            1
        } else {
            usize::from(ghostty_unicode::table().get(c).width)
        };

        // Note: it is possible to have a width of "3" and a width of "-1" from
        // uucode.x's wcwidth. We should look into those cases and handle them
        // appropriately.
        assert!(width <= 2);

        // Attach zero-width characters to our cell as grapheme data.
        if width == 0 {
            // If we have grapheme clustering enabled, we don't blindly attach
            // any zero width character to our cells and we instead just ignore
            // it.
            if self.modes.get(Mode::GraphemeCluster) {
                return Ok(());
            }

            // If we're at cell zero, then this is malformed data and we don't
            // print anything or even store this. Zero-width characters are ALWAYS
            // attached to some other non-zero-width character at the time of
            // writing.
            if self.screen().cursor.x == 0 {
                log::warn!("zero-width character with no prior character, ignoring");
                return Ok(());
            }

            // Find our previous cell
            let (prev_x, prev) = {
                let scr = self.screen();
                let immediate = scr.cursor_cell_left(1);
                if immediate.wide() == Wide::SpacerTail {
                    (scr.cursor.x - 2, scr.cursor_cell_left(2))
                } else {
                    (scr.cursor.x - 1, immediate)
                }
            };

            // If our previous cell has no text, just ignore the zero-width character
            if !prev.has_text() {
                log::warn!("zero-width character with no prior character, ignoring");
                return Ok(());
            }

            // If this is a emoji variation selector, prev must be an emoji
            if c == 0xFE0F || c == 0xFE0E {
                let prev_props = ghostty_unicode::table().get(prev.codepoint());
                let emoji = prev_props.grapheme_break
                    == ghostty_unicode::GraphemeBreakNoControl::ExtendedPictographic;
                if !emoji {
                    return Ok(());
                }
            }

            return self.screen_mut().append_grapheme(prev_x, c);
        }

        // We have a printable character, save it
        self.previous_char = Some(c);

        // Fast path: the overwhelmingly common case of printing a single-width
        // character in the middle of a line onto a plain narrow cell.
        if width == 1 && !self.modes.get(Mode::Insert) && self.print_fast(c, right_limit) {
            return Ok(());
        }

        // If we're soft-wrapping, then handle that first.
        if self.screen().cursor.pending_wrap && self.modes.get(Mode::Wraparound) {
            self.print_wrap()?;
        }

        // If we have insert mode enabled then we need to handle that. We
        // only do insert mode if we're not at the end of the line.
        if self.modes.get(Mode::Insert)
            && usize::from(self.screen().cursor.x) + width < usize::from(self.cols)
        {
            self.insert_blanks(width);
        }

        match width {
            // Single cell is very easy: just write in the cell
            1 => {
                self.screen_mut().cursor_mark_dirty();
                self.print_cell(c, Wide::Narrow);
            }

            // Wide character requires a spacer. We print this by
            // using two cells: the first is flagged "wide" and has the
            // wide char. The second is guaranteed to be a spacer if
            // we're not at the end of the line.
            2 => {
                if (right_limit - self.scrolling_region.left) > 1 {
                    // If we don't have space for the wide char, we need
                    // to insert spacers and wrap. Then we just print the wide
                    // char as normal.
                    if self.screen().cursor.x == right_limit - 1 {
                        // If we don't have wraparound enabled then we don't print
                        // this character at all and don't move the cursor. This is
                        // how xterm behaves.
                        if !self.modes.get(Mode::Wraparound) {
                            return Ok(());
                        }

                        // We only create a spacer head if we're at the real edge
                        // of the screen. Otherwise, we clear the space with a narrow.
                        // This allows soft wrapping to work correctly.
                        if right_limit == self.cols {
                            // Special-case: we need to set wrap to true even
                            // though we call print_wrap below because if there is
                            // a page resize during print_cell then it'll fail
                            // integrity checks.
                            self.screen_mut().cursor_row_mut().set_wrap(true);
                            self.print_cell(0, Wide::SpacerHead);
                        } else {
                            self.print_cell(0, Wide::Narrow);
                        }
                        self.print_wrap()?;
                    }

                    self.screen_mut().cursor_mark_dirty();
                    self.print_cell(c, Wide::Wide);
                    self.screen_mut().cursor_right(1);
                    self.print_cell(0, Wide::SpacerTail);
                } else {
                    // This is pretty broken, terminals should never be only 1-wide.
                    // We should prevent this downstream.
                    self.screen_mut().cursor_mark_dirty();
                    self.print_cell(0, Wide::Narrow);
                }
            }

            _ => unreachable!(),
        }

        // If we're at the column limit, then we need to wrap the next time.
        // In this case, we don't move the cursor.
        if self.screen().cursor.x == right_limit - 1 {
            self.screen_mut().cursor.pending_wrap = true;
            return Ok(());
        }

        // Move the cursor
        self.screen_mut().cursor_right(1);
        Ok(())
    }

    /// Fast path of `print` for a single-width character that does not hit
    /// the cursor's right limit. Performs exactly what the general path does
    /// (mark dirty, write the cell, move right) with a single page lookup.
    /// Returns `false` without touching anything if any precondition for the
    /// shortcut does not hold; the caller then takes the general path.
    #[inline]
    fn print_fast(&mut self, c: u32, right_limit: size::CellCountInt) -> bool {
        let scr = self.screens.active_mut();
        if scr.cursor.pending_wrap
            || scr.cursor.hyperlink_id != 0
            || scr.charset.single_shift.is_some()
            || c == KITTY_PLACEHOLDER
            || usize::from(scr.cursor.x) + 1 >= usize::from(right_limit)
        {
            return false;
        }
        let set = scr.charset.charsets.get(scr.charset.gl);
        if set != Charset::Utf8 && set != Charset::Ascii {
            return false;
        }

        let style_id = scr.cursor.style_id;
        let protected = scr.cursor.protected;
        let semantic = scr.cursor.semantic_content;
        let pin_handle = scr.cursor.page_pin;
        let Some(pin) = scr.pages.tracked_pin(pin_handle) else {
            return false;
        };
        let (x, y) = (usize::from(pin.x), usize::from(pin.y));
        let page = scr.page_mut(pin.node);
        let Ok(idx) = page.cell_index(x, y) else {
            return false;
        };
        let old = page.cells[idx];
        if old.wide() != Wide::Narrow
            || old.has_grapheme()
            || old.hyperlink()
            || old.style_id() != style_id
        {
            return false;
        }

        // Same cell value as `print_cell` produces.
        let mut new_cell = Cell::init(c);
        new_cell.set_content_tag(ContentTag::Codepoint);
        new_cell.set_style_id(style_id);
        new_cell.set_wide(Wide::Narrow);
        new_cell.set_protected(protected);
        new_cell.set_semantic_content(semantic);
        page.cells[idx] = new_cell;
        page.rows[y].set_dirty(true);

        // Move the cursor right (`Screen::cursor_right(1)`).
        if let Some(p) = scr.pages.tracked_pin_x_mut(pin_handle) {
            p.x += 1;
        }
        scr.cursor.x += 1;
        true
    }

    /// The grapheme-clustering section of `print`. Returns `true` if the
    /// character was fully handled (Zig: `return`), `false` if printing should
    /// continue on the normal path (Zig: `break :grapheme`).
    fn print_grapheme(&mut self, c: u32, right_limit: size::CellCountInt) -> Result<bool, Error> {
        // We need the previous cell to determine if we're at a grapheme
        // break or not. If we are NOT, then we are still combining the
        // same grapheme, and will be appending to prev.cell. Otherwise, we are
        // in a new cell.
        let wraparound = self.modes.get(Mode::Wraparound);
        let (mut prev_left, mut prev_cell) = {
            let scr = self.screen();
            let left: size::CellCountInt = if wraparound {
                // If we have wraparound, then we use the prev col unless
                // there's a pending wrap, in which case we use the current.
                size::CellCountInt::from(!scr.cursor.pending_wrap)
            } else if scr.cursor.x != right_limit - 1 {
                // If we do not have wraparound, the logic is trickier. If
                // we're not on the last column, then we just use the previous
                // column. Otherwise, we need to check if there is text to
                // figure out if we're attaching to the prev or current.
                1
            } else {
                size::CellCountInt::from(scr.cursor_cell().codepoint() == 0)
            };

            // If the previous cell is a wide spacer tail, then we actually
            // want to use the cell before that because that has the actual
            // content.
            let immediate = scr.cursor_cell_left(left);
            if immediate.wide() == Wide::SpacerTail {
                (left + 1, scr.cursor_cell_left(left + 1))
            } else {
                (left, immediate)
            }
        };

        // If our cell has no content, then this is a new cell and
        // necessarily a grapheme break.
        if prev_cell.codepoint() == 0 {
            return Ok(false);
        }

        let grapheme_break = {
            let mut state = ghostty_unicode::BreakState::default();
            let mut cp1 = prev_cell.codepoint();
            if prev_cell.has_grapheme() {
                let scr = self.screen();
                let pin = scr.cursor_pin();
                let page = scr.page(pin.node);
                let idx = page
                    .cell_index(usize::from(scr.cursor.x - prev_left), usize::from(pin.y))
                    .expect("prev cell");
                let cps = page.lookup_grapheme(idx).expect("grapheme data");
                for &cp2 in cps {
                    let b = ghostty_unicode::grapheme_break(cp1, cp2, &mut state);
                    debug_assert!(!b);
                    cp1 = cp2;
                }
            }

            ghostty_unicode::grapheme_break(cp1, c, &mut state)
        };

        // If we can NOT break, this means that "c" is part of a grapheme
        // with the previous char.
        if grapheme_break {
            return Ok(false);
        }

        #[derive(PartialEq, Eq)]
        enum DesiredWide {
            NoChange,
            Wide,
            Narrow,
        }
        let mut desired_wide = DesiredWide::NoChange;

        // The column of the previous cell on the cursor row.
        let mut prev_x = self.screen().cursor.x - prev_left;

        // If this is an emoji variation selector then we need to modify
        // the cell width accordingly. VS16 makes the character wide and
        // VS15 makes it narrow.
        if c == 0xFE0F || c == 0xFE0E {
            let prev_props = ghostty_unicode::table().get(prev_cell.codepoint());
            // Check if it is a valid variation sequence in
            // emoji-variation-sequences.txt, and if not, ignore the char.
            if !prev_props.emoji_vs_base {
                return Ok(true);
            }

            desired_wide = if c == 0xFE0F {
                DesiredWide::Wide
            } else {
                DesiredWide::Narrow
            };
        } else if !ghostty_unicode::table().get(c).width_zero_in_grapheme {
            // If we have a code point that contributes to the width of a
            // grapheme, it necessarily means that we're at least at width
            // 2, since the first code point must be at least width 1 to
            // start. (Note that Prepend code points could effectively mean
            // the first code point should be width 0, but we don't handle
            // that yet.)
            desired_wide = DesiredWide::Wide;
        }

        match desired_wide {
            DesiredWide::Wide => 'wide: {
                if prev_cell.wide() == Wide::Wide {
                    break 'wide;
                }

                // Move our cursor back to the previous. We'll move
                // the cursor within this block to the proper location.
                self.screen_mut().cursor_left(prev_left);

                // If we don't have space for the wide char, we need to
                // insert spacers and wrap. We need special handling if the
                // previous cell has grapheme data.
                if self.screen().cursor.x == right_limit - 1 {
                    if !wraparound {
                        return Ok(true);
                    }

                    // This path can write a spacer_head before print_wrap
                    // which can trigger integrity violations so mark
                    // the wrap first to keep the intermediary state valid
                    // if we're wrapping.
                    let row_wrap = right_limit == self.cols;
                    if row_wrap {
                        self.screen_mut().cursor_row_mut().set_wrap(true);
                    }

                    let prev_cp = prev_cell.codepoint();
                    if prev_cell.has_grapheme() {
                        // This is like print_cell but without clearing the
                        // grapheme data from the cell, so we can move it
                        // later.
                        // Zig keeps a raw pointer (`prev.cell`) to this cell;
                        // remember its page cell index, because the wrap below
                        // may scroll (rows move, cell memory does not) and the
                        // grapheme move must still use THIS cell.
                        let prev_idx = {
                            let scr = self.screen_mut();
                            let (x, pin) = (scr.cursor.x, scr.cursor_pin());
                            scr.page(pin.node)
                                .cell_index(usize::from(x), usize::from(pin.y))
                                .expect("prev cell")
                        };
                        {
                            let cell = self.screen_mut().cursor_cell_mut();
                            cell.set_wide(if row_wrap {
                                Wide::SpacerHead
                            } else {
                                Wide::Narrow
                            });
                            cell.set_codepoint(0);
                        }

                        self.print_wrap()?;
                        self.print_cell(prev_cp, Wide::Wide);

                        let new_pin = self.screen().cursor_pin();

                        'transfer: {
                            let Some(mut old_pin) = new_pin.up(&self.screen().pages, 1) else {
                                break 'transfer;
                            };
                            old_pin.x = right_limit - 1;
                            let (old_x, old_y) = (usize::from(old_pin.x), usize::from(old_pin.y));
                            let (new_x, new_y) = (usize::from(new_pin.x), usize::from(new_pin.y));

                            if new_pin.node == old_pin.node {
                                let page = self.screen_mut().page_mut(new_pin.node);
                                let new_idx = page.cell_index(new_x, new_y).expect("new cell");
                                // Zig moves from `prev.cell`, NOT from `old_pin`
                                // (they differ when margins keep the row from
                                // scrolling the cell out of `old_pin`).
                                let old_idx = prev_idx;
                                page.move_grapheme(old_idx, new_idx)?;
                                page.cells[old_idx].set_content_tag(ContentTag::Codepoint);
                                page.cells[new_idx].set_content_tag(ContentTag::CodepointGrapheme);
                                page.rows[new_y].set_grapheme(true);
                                page.update_row_grapheme_flag(old_y);
                            } else {
                                let cps: Vec<u32> = {
                                    let page = self.screen().page(old_pin.node);
                                    let old_idx = page.cell_index(old_x, old_y).expect("old cell");
                                    page.lookup_grapheme(old_idx).expect("grapheme").to_vec()
                                };
                                for cp in cps {
                                    let x = self.screen().cursor.x;
                                    self.screen_mut().append_grapheme(x, cp)?;
                                }
                                let page = self.screen_mut().page_mut(old_pin.node);
                                let old_idx = page.cell_index(old_x, old_y).expect("old cell");
                                page.clear_grapheme(old_idx);
                                page.update_row_grapheme_flag(old_y);
                            }
                        }

                        // Point prev.cell to our new previous cell that
                        // we'll be appending graphemes to
                        prev_x = self.screen().cursor.x;
                    } else {
                        self.print_cell(
                            0,
                            if row_wrap {
                                Wide::SpacerHead
                            } else {
                                Wide::Narrow
                            },
                        );
                        self.print_wrap()?;
                        self.print_cell(prev_cp, Wide::Wide);

                        // Point prev.cell to our new previous cell that
                        // we'll be appending graphemes to
                        prev_x = self.screen().cursor.x;
                    }
                } else {
                    self.screen_mut().cursor_cell_mut().set_wide(Wide::Wide);
                }

                // Write our spacer, since prev.cell is now wide
                self.screen_mut().cursor_right(1);
                self.print_cell(0, Wide::SpacerTail);

                // Move the cursor again so we're beyond our spacer
                if self.screen().cursor.x == right_limit - 1 {
                    self.screen_mut().cursor.pending_wrap = true;
                } else {
                    self.screen_mut().cursor_right(1);
                }
            }

            DesiredWide::Narrow => 'narrow: {
                // Prev cell is no longer wide
                if prev_cell.wide() != Wide::Wide {
                    break 'narrow;
                }
                {
                    let scr = self.screen_mut();
                    let x = scr.cursor.x;
                    let pin = scr.cursor_pin();
                    let page = scr.page_mut(pin.node);
                    let idx = page
                        .cell_index(usize::from(x - prev_left), usize::from(pin.y))
                        .expect("prev cell");
                    page.cells[idx].set_wide(Wide::Narrow);
                }
                prev_cell.set_wide(Wide::Narrow);

                // Remove the wide spacer tail
                self.screen_mut()
                    .cursor_cell_left_mut(prev_left - 1)
                    .set_wide(Wide::Narrow);

                // Back track the cursor so that we don't end up with
                // an extra space after the character. Since xterm is
                // not VS aware, it cannot be used as a reference for
                // this behavior; but it does follow the principle of
                // least surprise, and also matches the behavior that
                // can be observed in Kitty, which is one of the only
                // other VS aware terminals.
                if self.screen().cursor.x == right_limit - 1 {
                    // If we're already at the right edge, we stay
                    // here and set the pending wrap to false since
                    // when we pend a wrap, we only move our cursor once
                    // even for wide chars (tests verify).
                    self.screen_mut().cursor.pending_wrap = false;
                } else {
                    // Otherwise, move back.
                    self.screen_mut().cursor_left(1);
                }
            }

            DesiredWide::NoChange => {}
        }
        let _ = &mut prev_left;

        log::debug!(
            "c={c:X} grapheme attach to left={prev_left} primary_cp={:X}",
            prev_cell.codepoint()
        );
        self.screen_mut().cursor_mark_dirty();
        self.screen_mut().append_grapheme(prev_x, c)?;
        Ok(true)
    }

    fn print_cell(&mut self, unmapped_c: u32, wide: Wide) {
        let cols = self.cols;
        let scr = self.screens.active_mut();

        // TODO: spacers should use a bgcolor only cell

        let c: u32 = 'c: {
            // TODO: non-utf8 handling, gr

            // If we're single shifting, then we use the key exactly once.
            let key = if let Some(key_once) = scr.charset.single_shift {
                scr.charset.single_shift = None;
                key_once
            } else {
                scr.charset.gl
            };

            let set = scr.charset.charsets.get(key);

            // UTF-8 or ASCII is used as-is
            if set == Charset::Utf8 || set == Charset::Ascii {
                break 'c unmapped_c;
            }

            // If we're outside of ASCII range this is an invalid value in
            // this table so we just return space.
            if unmapped_c > u32::from(u8::MAX) {
                break 'c u32::from(' ');
            }

            // Get our lookup table and map it
            let table = charsets::table(set);
            u32::from(table[unmapped_c as usize])
        };

        let cell = scr.cursor_cell();

        // If the wide property of this cell is the same, then we don't
        // need to do the special handling here because the structure will
        // be the same. If it is NOT the same, then we may need to clear some
        // cells.
        if cell.wide() != wide {
            match cell.wide() {
                // Previous cell was narrow. Do nothing.
                Wide::Narrow => {}

                // Previous cell was wide. We need to clear the tail and head.
                Wide::Wide => 'wide: {
                    if scr.cursor.x >= cols - 1 {
                        break 'wide;
                    }

                    let pin = scr.cursor_pin();
                    scr.clear_cells(
                        pin.node,
                        pin.y,
                        usize::from(scr.cursor.x) + 1,
                        usize::from(scr.cursor.x) + 2,
                    );

                    // If we're near the left edge, a wide char may have
                    // wrapped from the previous row, leaving a spacer_head
                    // at the end of that row. Clear it so the previous row
                    // doesn't keep a stale spacer_head.
                    if scr.cursor.y > 0 && scr.cursor.x <= 1 {
                        clear_prev_row_spacer_head(scr, cols);
                    }
                }

                Wide::SpacerTail => {
                    assert!(scr.cursor.x > 0);

                    // So integrity checks pass. We fix this up later so we don't
                    // need to do this without safety checks.
                    if cfg!(debug_assertions) {
                        scr.cursor_cell_mut().set_wide(Wide::Narrow);
                    }

                    let pin = scr.cursor_pin();
                    scr.clear_cells(
                        pin.node,
                        pin.y,
                        usize::from(scr.cursor.x) - 1,
                        usize::from(scr.cursor.x),
                    );
                    // If we're near the left edge, a wide char may have
                    // wrapped from the previous row, leaving a spacer_head
                    // at the end of that row. Clear it so the previous row
                    // doesn't keep a stale spacer_head.
                    if scr.cursor.y > 0 && scr.cursor.x <= 1 {
                        clear_prev_row_spacer_head(scr, cols);
                    }
                }

                // TODO: this case was not handled in the old terminal implementation
                // but it feels like we should do something. investigate other
                // terminals (xterm mainly) and see what's up.
                Wide::SpacerHead => {}
            }
        }

        let pin = scr.cursor_pin();
        let (x, y) = (usize::from(pin.x), usize::from(pin.y));
        let cursor_style_id = scr.cursor.style_id;
        let cursor_protected = scr.cursor.protected;
        let cursor_semantic = scr.cursor.semantic_content;

        // Re-read the cell (clearing above may have changed it).
        let page = scr.page_mut(pin.node);
        let idx = page.cell_index(x, y).expect("cursor cell");
        let cell = page.cells[idx];

        // If the prior value had graphemes, clear those
        if cell.has_grapheme() {
            page.clear_grapheme(idx);
            page.update_row_grapheme_flag(y);
        }

        // We don't need to update the style refs unless the
        // cell's new style will be different after writing.
        let style_changed = cell.style_id() != cursor_style_id;
        if style_changed {
            // Release the old style.
            if cell.style_id() != style::DEFAULT_ID {
                assert!(page.rows[y].styled());
                let _ = page.styles.release(cell.style_id());
            }
        }

        // Keep track if we had a hyperlink so we can unset it.
        let had_hyperlink = cell.hyperlink();

        // Write
        let mut new_cell = Cell::init(c);
        new_cell.set_content_tag(ContentTag::Codepoint);
        new_cell.set_style_id(cursor_style_id);
        new_cell.set_wide(wide);
        new_cell.set_protected(cursor_protected);
        new_cell.set_semantic_content(cursor_semantic);
        page.cells[idx] = new_cell;

        if style_changed {
            // Use the new style.
            if cursor_style_id != style::DEFAULT_ID {
                let _ = page.styles.r#use(cursor_style_id);
                page.rows[y].set_styled(true);
            }
        }

        // If this is a Kitty unicode placeholder then we need to mark the
        // row so that the renderer can lookup rows with these much faster.
        if c == KITTY_PLACEHOLDER {
            page.rows[y].set_kitty_virtual_placeholder(true);
        }

        // We check for an active hyperlink first because setHyperlink
        // handles clearing the old hyperlink and an optimization if we're
        // overwriting the same hyperlink.
        if scr.cursor.hyperlink_id > 0 {
            if let Err(err) = scr.cursor_set_hyperlink() {
                log::warn!(
                    "error reallocating for more hyperlink space, ignoring hyperlink err={err:?}"
                );
                debug_assert!(!scr.cursor_cell().hyperlink());
            }
        } else if had_hyperlink {
            // If the previous cell had a hyperlink then we need to clear it.
            let page = scr.page_mut(pin.node);
            let _ = page.clear_hyperlink(idx);
            page.update_row_hyperlink_flag(y);
        }

        scr.assert_integrity();
    }

    fn print_wrap(&mut self) -> Result<(), Error> {
        // We only mark that we soft-wrapped if we're at the edge of our
        // full screen. We don't mark the row as wrapped if we're in the
        // middle due to a right margin.
        let mark_wrap = self.screen().cursor.x == self.cols - 1;
        if mark_wrap {
            self.screen_mut().cursor_row_mut().set_wrap(true);
        }

        // Get the old semantic prompt so we can extend it to the next
        // line. We need to do this before we index() because we may
        // modify memory.
        let old_semantic = self.screen().cursor.semantic_content;
        let old_semantic_clear = self.screen().cursor.semantic_content_clear_eol;

        // Move to the next line
        self.index()?;
        let left = self.scrolling_region.left;
        self.screen_mut().cursor_horizontal_absolute(left);

        // We always reset our semantic prompt state
        let scr = self.screen_mut();
        scr.cursor.semantic_content = old_semantic;
        scr.cursor.semantic_content_clear_eol = old_semantic_clear;
        match old_semantic {
            SemanticContent::Output | SemanticContent::Input => {}
            SemanticContent::Prompt => scr
                .cursor_row_mut()
                .set_semantic_prompt(RowPrompt::PromptContinuation),
        }

        if mark_wrap {
            // Always mark the row as a continuation
            scr.cursor_row_mut().set_wrap_continuation(true);
        }

        // Assure that our screen is consistent
        scr.assert_integrity();
        Ok(())
    }

    /// Set the charset into the given slot.
    pub fn configure_charset(&mut self, slot: Slots, set: Charset) {
        self.screen_mut().charset.charsets.set(slot, set);
    }

    /// Invoke the charset in slot into the active slot. If single is true,
    /// then this will only be invoked for a single character.
    pub fn invoke_charset(&mut self, active: charsets::ActiveSlot, slot: Slots, single: bool) {
        if single {
            assert!(active == charsets::ActiveSlot::GL);
            self.screen_mut().charset.single_shift = Some(slot);
            return;
        }

        match active {
            charsets::ActiveSlot::GL => self.screen_mut().charset.gl = slot,
            charsets::ActiveSlot::GR => self.screen_mut().charset.gr = slot,
        }
    }

    /// Carriage return moves the cursor to the first column.
    pub fn carriage_return(&mut self) {
        // Always reset pending wrap state
        self.screen_mut().cursor.pending_wrap = false;

        // In origin mode we always move to the left margin
        let x = if self.modes.get(Mode::Origin)
            || self.screen().cursor.x >= self.scrolling_region.left
        {
            self.scrolling_region.left
        } else {
            0
        };
        self.screen_mut().cursor_horizontal_absolute(x);
    }

    /// Linefeed moves the cursor to the next line.
    pub fn linefeed(&mut self) -> Result<(), Error> {
        self.index()?;
        if self.modes.get(Mode::Linefeed) {
            self.carriage_return();
        }
        Ok(())
    }

    /// Backspace moves the cursor back a column (but not less than 0).
    pub fn backspace(&mut self) {
        self.cursor_left(1);
    }

    /// Move the cursor up amount lines. If amount is greater than the maximum
    /// move distance then it is internally adjusted to the maximum. If amount is
    /// 0, adjust it to 1.
    pub fn cursor_up(&mut self, count_req: usize) {
        // Always resets pending wrap
        self.screen_mut().cursor.pending_wrap = false;

        // The maximum amount the cursor can move up depends on scrolling regions
        let y = self.screen().cursor.y;
        let max = if y >= self.scrolling_region.top {
            y - self.scrolling_region.top
        } else {
            y
        };
        let count = usize::from(max).min(count_req.max(1));

        // We can safely cast below because of the min/max clamping we did above.
        self.screen_mut().cursor_up(count as size::CellCountInt);
    }

    /// Move the cursor down amount lines. If amount is greater than the maximum
    /// move distance then it is internally adjusted to the maximum. This sequence
    /// will not scroll the screen or scroll region. If amount is 0, adjust it to 1.
    pub fn cursor_down(&mut self, count_req: usize) {
        // Always resets pending wrap
        self.screen_mut().cursor.pending_wrap = false;

        // The max the cursor can move to depends where the cursor currently is
        let y = self.screen().cursor.y;
        let max = if y <= self.scrolling_region.bottom {
            self.scrolling_region.bottom - y
        } else {
            self.rows - y - 1
        };
        let count = usize::from(max).min(count_req.max(1));
        self.screen_mut().cursor_down(count as size::CellCountInt);
    }

    /// Move the cursor right amount columns. If amount is greater than the
    /// maximum move distance then it is internally adjusted to the maximum.
    /// This sequence will not scroll the screen or scroll region. If amount is
    /// 0, adjust it to 1.
    pub fn cursor_right(&mut self, count_req: usize) {
        // Always resets pending wrap
        self.screen_mut().cursor.pending_wrap = false;

        // The max the cursor can move to depends where the cursor currently is
        let x = self.screen().cursor.x;
        let max = if x <= self.scrolling_region.right {
            self.scrolling_region.right - x
        } else {
            self.cols - x - 1
        };
        let count = usize::from(max).min(count_req.max(1));
        self.screen_mut().cursor_right(count as size::CellCountInt);
    }

    /// Move the cursor to the left amount cells. If amount is 0, adjust it to 1.
    pub fn cursor_left(&mut self, count_req: usize) {
        // Wrapping behavior depends on various terminal modes
        #[derive(PartialEq, Eq, Clone, Copy)]
        enum WrapMode {
            None,
            Reverse,
            ReverseExtended,
        }
        let wrap_mode = if !self.modes.get(Mode::Wraparound) {
            WrapMode::None
        } else if self.modes.get(Mode::ReverseWrapExtended) {
            WrapMode::ReverseExtended
        } else if self.modes.get(Mode::ReverseWrap) {
            WrapMode::Reverse
        } else {
            WrapMode::None
        };

        let mut count = count_req.max(1);

        // If we are in no wrap mode, then we move the cursor left and exit
        // since this is the fastest and most typical path.
        if wrap_mode == WrapMode::None {
            let x = self.screen().cursor.x;
            self.screen_mut()
                .cursor_left(count.min(usize::from(x)) as size::CellCountInt);
            self.screen_mut().cursor.pending_wrap = false;
            return;
        }

        // If we have a pending wrap state and we are in either reverse wrap
        // modes then we decrement the amount we move by one to match xterm.
        if self.screen().cursor.pending_wrap {
            count -= 1;
            self.screen_mut().cursor.pending_wrap = false;
        }

        // The margins we can move to.
        let top = self.scrolling_region.top;
        let bottom = self.scrolling_region.bottom;
        let right_margin = self.scrolling_region.right;
        let left_margin = if self.screen().cursor.x < self.scrolling_region.left {
            0
        } else {
            self.scrolling_region.left
        };

        // Handle some edge cases when our cursor is already on the left margin.
        if self.screen().cursor.x == left_margin {
            match wrap_mode {
                // In reverse mode, if we're already before the top margin
                // then we just set our cursor to the top-left and we're done.
                WrapMode::Reverse => {
                    if self.screen().cursor.y <= top {
                        self.screen_mut().cursor_absolute(left_margin, top);
                        return;
                    }
                }

                // Handled in while loop
                WrapMode::ReverseExtended => {}

                // Handled above
                WrapMode::None => unreachable!(),
            }
        }

        loop {
            // We can move at most to the left margin.
            let max = usize::from(self.screen().cursor.x - left_margin);

            // We want to move at most the number of columns we have left
            // or our remaining count. Do the move.
            let amount = max.min(count);
            count -= amount;
            self.screen_mut().cursor_left(amount as size::CellCountInt);

            // If we have no more to move, then we're done.
            if count == 0 {
                break;
            }

            // If we are at the top, then we are done.
            if self.screen().cursor.y == top {
                if wrap_mode != WrapMode::ReverseExtended {
                    break;
                }

                self.screen_mut().cursor_absolute(right_margin, bottom);
                count -= 1;
                continue;
            }

            // UNDEFINED TERMINAL BEHAVIOR. This situation is not handled in xterm
            // and currently results in a crash in xterm. Given no other known
            // terminal [to me] implements XTREVWRAP2, I decided to just mimic
            // the behavior of xterm up and not including the crash by wrapping
            // up to the (0, 0) and stopping there. My reasoning is that for an
            // appropriately sized value of "count" this is the behavior that xterm
            // would have. This is unit tested.
            if self.screen().cursor.y == 0 {
                assert!(self.screen().cursor.x == left_margin);
                break;
            }

            // If our previous line is not wrapped then we are done.
            if wrap_mode != WrapMode::ReverseExtended {
                let prev_row = self.screen().cursor_row_up(1);
                if !prev_row.wrap() {
                    break;
                }
            }

            let y = self.screen().cursor.y - 1;
            self.screen_mut().cursor_absolute(right_margin, y);
            count -= 1;
        }
    }

    /// Save cursor position and further state.
    ///
    /// The primary and alternate screen have distinct save state. One saved state
    /// is kept per screen (main / alternative). If for the current screen state
    /// was already saved it is overwritten.
    pub fn save_cursor(&mut self) {
        let origin = self.modes.get(Mode::Origin);
        let scr = self.screen_mut();
        scr.saved_cursor = Some(SavedCursor {
            x: scr.cursor.x,
            y: scr.cursor.y,
            style: scr.cursor.style,
            protected: scr.cursor.protected,
            pending_wrap: scr.cursor.pending_wrap,
            origin,
            charset: scr.charset,
        });
    }

    /// Restore cursor position and other state.
    ///
    /// The primary and alternate screen have distinct save state.
    /// If no save was done before values are reset to their initial values.
    pub fn restore_cursor(&mut self) {
        let saved = self.screen().saved_cursor.unwrap_or(SavedCursor {
            x: 0,
            y: 0,
            style: Style::default(),
            protected: false,
            pending_wrap: false,
            origin: false,
            charset: CharsetState::default(),
        });

        // Set the style first because it can fail
        let scr = self.screen_mut();
        scr.cursor.style = saved.style;
        if let Err(err) = scr.manual_style_update() {
            // Regardless of the error here, we revert back to an unstyled
            // cursor. It is more important that the restore succeeds in
            // other attributes because terminals have no way to communicate
            // failure back.
            log::warn!("restoreCursor error updating style err={err:?}");
            scr.cursor.style = Style::default();
            scr.manual_style_update().expect("default style update");
        }

        scr.charset = saved.charset;
        scr.cursor.pending_wrap = saved.pending_wrap;
        scr.cursor.protected = saved.protected;
        let (cols, rows) = (self.cols, self.rows);
        self.modes.set(Mode::Origin, saved.origin);
        self.screen_mut()
            .cursor_absolute(saved.x.min(cols - 1), saved.y.min(rows - 1));

        // Ensure our screen is consistent
        self.screen().assert_integrity();
    }

    /// Set the character protection mode for the terminal.
    pub fn set_protected_mode(&mut self, mode: ProtectedMode) {
        let scr = self.screen_mut();
        match mode {
            ProtectedMode::Off => {
                scr.cursor.protected = false;

                // screen.protected_mode is NEVER reset to ".off" because
                // logic such as eraseChars depends on knowing what the
                // _most recent_ mode was.
            }

            ProtectedMode::Iso => {
                scr.cursor.protected = true;
                scr.protected_mode = ProtectedMode::Iso;
            }

            ProtectedMode::Dec => {
                scr.cursor.protected = true;
                scr.protected_mode = ProtectedMode::Dec;
            }
        }
    }

    /// Perform a semantic prompt command.
    ///
    /// If there is an error, we do our best to get the terminal into
    /// some coherent state, since callers typically can't handle errors
    /// (since they're sending sequences via the pty).
    pub fn semantic_prompt(&mut self, cmd: &osc::SemanticPrompt) -> Result<(), Error> {
        match cmd.action {
            PromptAction::FreshLine => self.semantic_prompt_fresh_line()?,

            PromptAction::FreshLineNewPrompt => {
                // "First do a fresh-line."
                self.semantic_prompt_fresh_line()?;

                // "Subsequent text (until a OSC "133;B" or OSC "133;I" command)
                // is a prompt string (as if followed by OSC 133;P;k=i\007)."
                let kind = cmd
                    .prompt_kind()
                    .unwrap_or(osc::semantic_prompt::PromptKind::Initial);
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Prompt(kind));

                // This is a kitty-specific flag that notes that the shell
                // is NOT capable of redraw. Redraw defaults to true so this
                // usually just disables it, but either is possible.
                if let Some(v) = cmd.redraw() {
                    self.flags.shell_redraws_prompt = v;
                }

                'click: {
                    // Handle click_events as a priority over cl. click_events
                    // is another Kitty-specific extension that converts clicks
                    // within a prompt area to SGR mouse events and defers to the
                    // shell to handle them.
                    if let Some(v) = cmd.click_events()
                        && v
                    {
                        self.screen_mut().semantic_prompt.click = SemanticClick::ClickEvents;
                        break 'click;
                    }

                    // If click_events was not set or disabled, fallback to `cl`.
                    if let Some(v) = cmd.cl() {
                        self.screen_mut().semantic_prompt.click = SemanticClick::Cl(v);
                    }
                }

                // The "aid" and "cl" options are also valid for this
                // command but we don't yet handle these in any meaningful way.
            }

            PromptAction::NewCommand => {
                // Spec:
                // Same as OSC "133;A" but may first implicitly terminate a
                // previous command: if the options specify an aid and there
                // is an active (open) command with matching aid, finish the
                // innermost such command (as well as any other commands
                // nested more deeply). If no aid is specified, treat as an
                // aid whose value is the empty string.

                // Ghostty:
                // We don't currently do explicit command tracking in any way
                // so there is no need to terminate prior commands. We just
                // perform the `A` action.
                self.semantic_prompt(&osc::SemanticPrompt {
                    action: PromptAction::FreshLineNewPrompt,
                    options_unvalidated: cmd.options_unvalidated.clone(),
                })?;
            }

            PromptAction::PromptStart => {
                // Explicit start of prompt. Optional after an A or N command.
                // The k (kind) option specifies the type of prompt:
                // regular primary prompt (k=i or default),
                // right-side prompts (k=r), or prompts for continuation lines (k=c or k=s).
                let kind = cmd
                    .prompt_kind()
                    .unwrap_or(osc::semantic_prompt::PromptKind::Initial);
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Prompt(kind));
            }

            PromptAction::EndPromptStartInput => {
                // End of prompt and start of user input, terminated by a OSC
                // "133;C" or another prompt (OSC "133;P").
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Input(InputClear::ClearExplicit));
            }

            PromptAction::EndPromptStartInputTerminateEol => {
                // End of prompt and start of user input, terminated by end-of-line.
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Input(InputClear::ClearEol));
            }

            PromptAction::EndInputStartOutput => {
                // "End of input, and start of output."
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Output);

                // If our current row is marked as a prompt and we're
                // at column zero then we assume we're un-prompting. This
                // is a heuristic to deal with fish, mostly. The issue that
                // fish brings up is that it has no PS2 equivalent and its
                // builtin OSC133 marking doesn't output continuation lines
                // as k=s. So, we assume when we get a newline with a prompt
                // cursor that the new line is also a prompt. But fish changes
                // to output on the newline. So if we're at col 0 we just assume
                // we're overwriting the prompt.
                if self.screen().cursor_row().semantic_prompt() != RowPrompt::None
                    && self.screen().cursor.x == 0
                {
                    self.screen_mut()
                        .cursor_row_mut()
                        .set_semantic_prompt(RowPrompt::None);
                }
            }

            PromptAction::EndCommand => {
                // From a terminal state perspective, this doesn't really do
                // anything. Other terminals appear to do nothing here. I think
                // its reasonable at this point to reset our semantic content
                // state but the spec doesn't really say what to do.
                self.screen_mut()
                    .cursor_set_semantic_content(SetSemantic::Output);
            }
        }
        Ok(())
    }

    // OSC 133;L
    fn semantic_prompt_fresh_line(&mut self) -> Result<(), Error> {
        let left_margin = if self.screen().cursor.x < self.scrolling_region.left {
            0
        } else {
            self.scrolling_region.left
        };

        // Spec: "If the cursor is the initial column (left, assuming
        // left-to-right writing), do nothing" This specification is very under
        // specified. We are taking the liberty to assume that in a left/right
        // margin context, if the cursor is outside of the left margin, we treat
        // it as being at the left margin for the purposes of this command.
        // This is arbitrary. If someone has a better reasonable idea we can
        // apply it.
        if self.screen().cursor.x == left_margin {
            return Ok(());
        }

        self.carriage_return();
        self.index()
    }

    /// Returns true if the cursor is currently at a prompt. Another way to look
    /// at this is it returns false if the shell is currently outputting something.
    /// This requires shell integration (semantic prompt integration).
    ///
    /// If the shell integration doesn't exist, this will always return false.
    pub fn cursor_is_at_prompt(&self) -> bool {
        // If we're on the secondary screen, we're never at a prompt.
        if self.screens.active_key == ScreenKey::Alternate {
            return false;
        }

        // If our page row is a prompt then we're always at a prompt
        let scr = self.screen();
        if scr.cursor_row().semantic_prompt() != RowPrompt::None {
            return true;
        }

        // Otherwise, determine our cursor state
        match scr.cursor.semantic_content {
            SemanticContent::Input | SemanticContent::Prompt => true,
            SemanticContent::Output => false,
        }
    }

    /// Horizontal tab moves the cursor to the next tabstop, clearing
    /// the screen to the left the tabstop.
    pub fn horizontal_tab(&mut self) {
        while self.screen().cursor.x < self.scrolling_region.right {
            // Move the cursor right
            self.screen_mut().cursor_right(1);

            // If the last cursor position was a tabstop we return. We do
            // "last cursor position" because we want a space to be written
            // at the tabstop unless we're at the end (the while condition).
            if self.tabstops.get(usize::from(self.screen().cursor.x)) {
                return;
            }
        }
    }

    // Same as horizontal_tab but moves to the previous tabstop instead of the next.
    pub fn horizontal_tab_back(&mut self) {
        // With origin mode enabled, our leftmost limit is the left margin.
        let left_limit = if self.modes.get(Mode::Origin) {
            self.scrolling_region.left
        } else {
            0
        };

        loop {
            // If we're already at the edge of the screen, then we're done.
            if self.screen().cursor.x <= left_limit {
                return;
            }

            // Move the cursor left
            self.screen_mut().cursor_left(1);
            if self.tabstops.get(usize::from(self.screen().cursor.x)) {
                return;
            }
        }
    }

    /// Clear tab stops.
    pub fn tab_clear(&mut self, cmd: csi::TabClear) {
        match cmd {
            csi::TabClear::Current => self.tabstops.unset(usize::from(self.screen().cursor.x)),
            csi::TabClear::All => self.tabstops.reset(0),
            other @ csi::TabClear::Other(_) => {
                log::warn!("invalid or unknown tab clear setting: {other:?}");
            }
        }
    }

    /// Set a tab stop on the current cursor.
    pub fn tab_set(&mut self) {
        self.tabstops.set(usize::from(self.screen().cursor.x));
    }

    pub fn tab_reset(&mut self) {
        self.tabstops.reset(TABSTOP_INTERVAL);
    }

    /// Move the cursor to the next line in the scrolling region, possibly scrolling.
    ///
    /// If the cursor is outside of the scrolling region: move the cursor one line
    /// down if it is not on the bottom-most line of the screen.
    ///
    /// If the cursor is inside the scrolling region:
    ///   If the cursor is on the bottom-most line of the scrolling region:
    ///     invoke scroll up with amount=1
    ///   If the cursor is not on the bottom-most line of the scrolling region:
    ///     move the cursor one line down
    ///
    /// This unsets the pending wrap state without wrapping.
    #[allow(clippy::if_not_else)]
    pub fn index(&mut self) -> Result<(), Error> {
        // Unset pending wrap state
        self.screen_mut().cursor.pending_wrap = false;

        // We handle our cursor semantic prompt state AFTER doing the
        // scrolling, because we may need to apply to new rows.
        let result = self.index_inner();

        let screen = self.screen_mut();
        if screen.cursor.semantic_content != SemanticContent::Output {
            // Always reset any semantic content clear-eol state.
            //
            // The specification is not clear what "end-of-line" means. If we
            // discover that there are more scenarios we should be unsetting
            // this we should document and test it.
            if screen.cursor.semantic_content_clear_eol {
                screen.cursor.semantic_content = SemanticContent::Output;
                screen.cursor.semantic_content_clear_eol = false;
            } else {
                // If we aren't clearing our state at EOL and we're not output,
                // then we mark the new row as a prompt continuation. This is
                // to work around shells that don't send OSC 133 k=s sequences
                // for continuations.
                //
                // This can be a false positive if the shell changes content
                // type later and outputs something. We handle that in the
                // semanticPrompt function.
                screen
                    .cursor_row_mut()
                    .set_semantic_prompt(RowPrompt::PromptContinuation);
            }
        } else {
            // This should never be set in the output mode.
            assert!(!screen.cursor.semantic_content_clear_eol);
        }

        result
    }

    fn index_inner(&mut self) -> Result<(), Error> {
        let rows = self.rows;
        let cols = self.cols;
        let sr = self.scrolling_region;
        let screen = self.screens.active_mut();

        // Outside of the scroll region we move the cursor one line down.
        if screen.cursor.y < sr.top || screen.cursor.y > sr.bottom {
            // We only move down if we're not already at the bottom of
            // the screen.
            if screen.cursor.y < rows - 1 {
                screen.cursor_down(1);
            }

            return Ok(());
        }

        // If the cursor is inside the scrolling region and on the bottom-most
        // line, then we scroll up. If our scrolling region is the full screen
        // we create scrollback.
        if screen.cursor.y == sr.bottom && screen.cursor.x >= sr.left && screen.cursor.x <= sr.right
        {
            // Scrolling dirties the images because it updates their placements pins.
            screen.kitty_images.dirty = true;

            // If our scrolling region is at the top, we create scrollback.
            if sr.top == 0 && sr.left == 0 && sr.right == cols - 1 {
                screen.cursor_scroll_above()?;
                return Ok(());
            }

            // Slow path for left and right scrolling region margins.
            if sr.left != 0
                || sr.right != cols - 1
                // PERF(mitchellh): If we have an SGR background set then
                // we need to preserve that background in our erased rows.
                // scrollUp does that but eraseRowBounded below does not.
                // However, scrollUp is WAY slower. We should optimize this
                // case to work in the eraseRowBounded codepath and remove
                // this check.
                || !screen.blank_cell().is_zero()
            {
                return self.scroll_up(1);
            }

            // Otherwise use a fast path function from PageList to efficiently
            // scroll the contents of the scrolling region.

            // Preserve old cursor just for assertions
            let (old_x, old_y) = (screen.cursor.x, screen.cursor.y);

            screen.pages.erase_row_bounded(
                Point::Active(Coordinate {
                    x: 0,
                    y: u32::from(sr.top),
                }),
                usize::from(sr.bottom - sr.top),
            )?;

            // eraseRow and eraseRowBounded will end up moving the cursor pin
            // up by 1, so we need to move it back down. A `cursorReload`
            // would be better option but this is more efficient and this is
            // a super hot path so we do this instead.
            assert!(screen.cursor.x == old_x);
            assert!(screen.cursor.y == old_y);
            screen.cursor.y -= 1;
            screen.cursor_down(1);

            // The operations above can prune our cursor style so we need to
            // update. This should never fail because the above can only FREE
            // memory.
            if let Err(err) = screen.manual_style_update() {
                log::warn!("deleteLines manualStyleUpdate err={err:?}");
                screen.cursor.style = Style::default();
                screen.manual_style_update().expect("default style update");
            }

            return Ok(());
        }

        // Increase cursor by 1, maximum to bottom of scroll region
        if screen.cursor.y < sr.bottom {
            screen.cursor_down(1);
        }
        Ok(())
    }

    /// Move the cursor to the previous line in the scrolling region, possibly
    /// scrolling.
    ///
    /// If the cursor is outside of the scrolling region, move the cursor one
    /// line up if it is not on the top-most line of the screen.
    ///
    /// If the cursor is inside the scrolling region:
    ///
    ///   * If the cursor is on the top-most line of the scrolling region:
    ///     invoke scroll down with amount=1
    ///   * If the cursor is not on the top-most line of the scrolling region:
    ///     move the cursor one line up
    pub fn reverse_index(&mut self) {
        let scr = self.screen();
        if scr.cursor.y != self.scrolling_region.top
            || scr.cursor.x < self.scrolling_region.left
            || scr.cursor.x > self.scrolling_region.right
        {
            self.cursor_up(1);
            return;
        }

        self.scroll_down(1);
    }

    /// Set Cursor Position. Move cursor to the position indicated
    /// by row and column (1-indexed). If column is 0, it is adjusted to 1.
    /// If column is greater than the right-most column it is adjusted to
    /// the right-most column. If row is 0, it is adjusted to 1. If row is
    /// greater than the bottom-most row it is adjusted to the bottom-most
    /// row.
    pub fn set_cursor_pos(&mut self, row_req: usize, col_req: usize) {
        // If cursor origin mode is set the cursor row will be moved relative to
        // the top margin row and adjusted to be above or at bottom-most row in
        // the current scroll region.
        //
        // If origin mode is set and left and right margin mode is set the cursor
        // will be moved relative to the left margin column and adjusted to be on
        // or left of the right margin column.
        let (x_offset, y_offset, x_max, y_max): (usize, usize, usize, usize) =
            if self.modes.get(Mode::Origin) {
                (
                    usize::from(self.scrolling_region.left),
                    usize::from(self.scrolling_region.top),
                    usize::from(self.scrolling_region.right) + 1, // We need this 1-indexed
                    usize::from(self.scrolling_region.bottom) + 1, // We need this 1-indexed
                )
            } else {
                (0, 0, usize::from(self.cols), usize::from(self.rows))
            };

        // Unset pending wrap state
        self.screen_mut().cursor.pending_wrap = false;

        // Calculate our new x/y
        let row = if row_req == 0 { 1 } else { row_req };
        let col = if col_req == 0 { 1 } else { col_req };
        let x = x_max.min(col.saturating_add(x_offset)).saturating_sub(1) as size::CellCountInt;
        let y = y_max.min(row.saturating_add(y_offset)).saturating_sub(1) as size::CellCountInt;

        // If the y is unchanged then this is fast pointer math
        let scr = self.screen_mut();
        if y == scr.cursor.y {
            if x > scr.cursor.x {
                scr.cursor_right(x - scr.cursor.x);
            } else {
                scr.cursor_left(scr.cursor.x - x);
            }

            return;
        }

        // If everything changed we do an absolute change which is slightly slower
        scr.cursor_absolute(x, y);
    }

    /// Set Top and Bottom Margins If bottom is not specified, 0 or bigger than
    /// the number of the bottom-most row, it is adjusted to the number of the
    /// bottom most row.
    ///
    /// If top < bottom set the top and bottom row of the scroll region according
    /// to top and bottom and move the cursor to the top-left cell of the display
    /// (when in cursor origin mode is set to the top-left cell of the scroll region).
    ///
    /// Otherwise: Set the top and bottom row of the scroll region to the top-most
    /// and bottom-most line of the screen.
    ///
    /// Top and bottom are 1-indexed.
    pub fn set_top_and_bottom_margin(&mut self, top_req: usize, bottom_req: usize) {
        let top = top_req.max(1);
        let rows = usize::from(self.rows);
        let bottom = rows.min(if bottom_req == 0 { rows } else { bottom_req });
        if top >= bottom {
            return;
        }

        self.scrolling_region.top = (top - 1) as size::CellCountInt;
        self.scrolling_region.bottom = (bottom - 1) as size::CellCountInt;
        self.set_cursor_pos(1, 1);
    }

    /// DECSLRM
    pub fn set_left_and_right_margin(&mut self, left_req: usize, right_req: usize) {
        // We must have this mode enabled to do anything
        if !self.modes.get(Mode::EnableLeftAndRightMargin) {
            return;
        }

        let left = left_req.max(1);
        let cols = usize::from(self.cols);
        let right = cols.min(if right_req == 0 { cols } else { right_req });
        if left >= right {
            return;
        }

        self.scrolling_region.left = (left - 1) as size::CellCountInt;
        self.scrolling_region.right = (right - 1) as size::CellCountInt;
        self.set_cursor_pos(1, 1);
    }

    /// Scroll the text down by one row.
    pub fn scroll_down(&mut self, count: usize) {
        // Preserve our x/y to restore.
        let (old_x, old_y, old_wrap) = {
            let c = &self.screen().cursor;
            (c.x, c.y, c.pending_wrap)
        };

        // Move to the top of the scroll region
        let (l, t) = (self.scrolling_region.left, self.scrolling_region.top);
        self.screen_mut().cursor_absolute(l, t);
        self.insert_lines(count);

        let scr = self.screen_mut();
        scr.cursor_absolute(old_x, old_y);
        scr.cursor.pending_wrap = old_wrap;
    }

    /// Removes amount lines from the top of the scroll region. The remaining lines
    /// to the bottom margin are shifted up and space from the bottom margin up
    /// is filled with empty lines.
    ///
    /// The new lines are created according to the current SGR state.
    ///
    /// Does not change the (absolute) cursor position.
    pub fn scroll_up(&mut self, count: usize) -> Result<(), Error> {
        // Preserve our x/y to restore.
        let (old_x, old_y, old_wrap) = {
            let c = &self.screen().cursor;
            (c.x, c.y, c.pending_wrap)
        };

        let result = self.scroll_up_inner(count);

        let scr = self.screen_mut();
        scr.cursor_absolute(old_x, old_y);
        scr.cursor.pending_wrap = old_wrap;
        result
    }

    fn scroll_up_inner(&mut self, count: usize) -> Result<(), Error> {
        let sr = self.scrolling_region;

        // If our scroll region is at the top and we have no left/right
        // margins then we move the scrolled out text into the scrollback.
        if sr.top == 0 && sr.left == 0 && sr.right == self.cols - 1 {
            // Scrolling dirties the images because it updates their placements pins.
            self.screen_mut().kitty_images.dirty = true;

            // Clamp count to the scroll region height.
            let region_height = usize::from(sr.bottom) + 1;
            let adjusted_count = count.min(region_height);

            // TODO: Create an optimized version that can scroll N times
            // This isn't critical because in most cases, scrollUp is used
            // with count=1, but it's still a big optimization opportunity.

            // Move our cursor to the bottom of the scroll region so we can
            // use the cursorScrollAbove function to create scrollback
            self.screen_mut().cursor_absolute(0, sr.bottom);
            for _ in 0..adjusted_count {
                self.screen_mut().cursor_scroll_above()?;
            }
            return Ok(());
        }

        // Move to the top of the scroll region
        self.screen_mut().cursor_absolute(sr.left, sr.top);
        self.delete_lines(count);
        Ok(())
    }

    /// Scroll the viewport of the terminal grid.
    pub fn scroll_viewport(&mut self, behavior: ScrollViewport) {
        self.screen_mut().scroll(match behavior {
            ScrollViewport::Top => Scroll::Top,
            ScrollViewport::Bottom => Scroll::Active,
            ScrollViewport::Delta(delta) => Scroll::DeltaRow(delta),
        });
    }

    /// Shared implementation of insert_lines / delete_lines. When `insert` is
    /// true rows are shifted down from the cursor (iterating bottom-up),
    /// otherwise rows are shifted up (iterating top-down).
    fn shift_lines(&mut self, count: usize, insert: bool) {
        // Rare, but happens
        if count == 0 {
            return;
        }

        let sr = self.scrolling_region;

        // If the cursor is outside the scroll region we do nothing.
        {
            let c = &self.screen().cursor;
            if c.y < sr.top || c.y > sr.bottom || c.x < sr.left || c.x > sr.right {
                return;
            }
        }

        // Scrolling dirties the images because it updates their placements pins.
        self.screen_mut().kitty_images.dirty = true;

        // At the end we need to return the cursor to the row it started on.
        let start_y = self.screen().cursor.y;

        // We have a slower path if we have left or right scroll margins.
        let left_right = sr.left > 0 || sr.right < self.cols - 1;

        // Remaining rows from our cursor to the bottom of the scroll region.
        let rem = usize::from(sr.bottom - self.screen().cursor.y) + 1;

        // We can only insert lines up to our remaining lines in the scroll
        // region. So we take whichever is smaller.
        let adjusted_count = count.min(rem);

        // Create a new tracked pin which we'll use to navigate the page list
        // so that if we need to adjust capacity it will be properly tracked.
        let start_pin = if insert {
            self.screen()
                .cursor_pin()
                .down(&self.screen().pages, rem - 1)
                .expect("scroll region bottom")
        } else {
            self.screen().cursor_pin()
        };
        let cur_h = match self.screen_mut().pages.track_pin(start_pin) {
            Ok(h) => h,
            Err(err) => {
                // This error scenario means that our GPA is OOM. This is not a
                // situation we can gracefully handle. We can't just ignore
                // because it'll result in a corrupted screen.
                log::error!("shift_lines trackPin error err={err:?}");
                panic!("shift_lines trackPin OOM");
            }
        };

        // Our current y position relative to the cursor
        let mut y: usize = if insert { rem } else { 0 };

        // Traverse from the bottom up (insert) or top down (delete)
        while if insert { y > 0 } else { y < rem } {
            let cur_p = self.screen().pages.tracked_pin(cur_h).expect("tracked pin");

            // If this is one of the lines we need to shift, do so
            let shift = if insert {
                y > adjusted_count
            } else {
                y < rem - adjusted_count
            };
            if shift {
                let off_p = if insert {
                    cur_p.up(&self.screen().pages, adjusted_count)
                } else {
                    cur_p.down(&self.screen().pages, adjusted_count)
                }
                .expect("shift source row");

                let sr = self.scrolling_region;
                self.row_will_be_shifted_at(cur_p);
                self.row_will_be_shifted_at(off_p);

                // If our scrolling region is full width, then we unset wrap.
                if !left_right {
                    let scr = self.screen_mut();
                    for p in [off_p, cur_p] {
                        let row = &mut scr.page_mut(p.node).rows[usize::from(p.y)];
                        row.set_wrap(false);
                        row.set_wrap_continuation(false);
                    }
                }

                let (src_p, dst_p) = (off_p, cur_p);

                // If our page doesn't match, then we need to do a copy from
                // one page to another. This is the slow path.
                if src_p.node != dst_p.node {
                    if let Err(err) = self.screen_mut().pages.clone_partial_row_between(
                        src_p,
                        dst_p,
                        usize::from(sr.left),
                        usize::from(sr.right) + 1,
                    ) {
                        // Adjust our page capacity to make
                        // room for we didn't have space for
                        let Some(adj) = capacity_adjustment(err) else {
                            panic!("clone_partial_row_from failed: {err:?}");
                        };
                        if let Err(e) = self.screen_mut().increase_capacity(dst_p.node, adj) {
                            match e {
                                // System OOM. We have no way to recover from this
                                // currently.
                                Error::OutOfMemory => {
                                    panic!("increaseCapacity system allocator OOM")
                                }
                                // The page can't accommodate the managed memory required
                                // for this operation.
                                _ => panic!("increaseCapacity OutOfSpace"),
                            }
                        }

                        // Continue the loop to try handling this row again.
                        continue;
                    }
                } else if !left_right {
                    // Swap the src/dst cells. This ensures that our dst gets the
                    // proper shifted rows and src gets non-garbage cell data that
                    // we can clear.
                    let page = self.screen_mut().page_mut(cur_p.node);
                    page.rows.swap(usize::from(dst_p.y), usize::from(src_p.y));

                    // Ensure what we did didn't corrupt the page
                    page.assert_integrity();
                } else {
                    // Left/right scroll margins we have to
                    // copy cells, which is much slower...
                    let page = self.screen_mut().page_mut(cur_p.node);
                    page.move_cells(
                        usize::from(src_p.y),
                        usize::from(sr.left),
                        usize::from(dst_p.y),
                        usize::from(sr.left),
                        usize::from(sr.right - sr.left) + 1,
                    )
                    .expect("move_cells");
                }
            } else {
                // Clear the cells for this row, it has been shifted (insert)
                // or it's from out of bounds (delete).
                self.row_will_be_shifted_at(cur_p);
                let (l, r) = (usize::from(sr.left), usize::from(sr.right) + 1);
                self.screen_mut().clear_cells(cur_p.node, cur_p.y, l, r);
            }

            // Mark the row as dirty
            cur_p.mark_dirty(&mut self.screen_mut().pages);

            // We have successfully processed a line.
            if insert {
                y -= 1;
                // Move our pin up to the next row.
                if let Some(p) = cur_p.up(&self.screen().pages, 1) {
                    let _ = self.screen_mut().pages.set_tracked_pin(cur_h, p);
                }
            } else {
                y += 1;
                // Move our pin down to the next row.
                if let Some(p) = cur_p.down(&self.screen().pages, 1) {
                    let _ = self.screen_mut().pages.set_tracked_pin(cur_h, p);
                }
            }
        }

        let scr = self.screen_mut();
        scr.pages.untrack_pin(cur_h);
        scr.cursor_absolute(sr.left, start_y);

        // Always unset pending wrap
        scr.cursor.pending_wrap = false;
    }

    fn row_will_be_shifted_at(&mut self, p: Pin) {
        let (sr, cols) = (self.scrolling_region, self.cols);
        let page = self.screens.active_mut().page_mut(p.node);
        row_will_be_shifted(sr, cols, page, usize::from(p.y));
    }

    /// Insert amount lines at the current cursor row. The contents of the line
    /// at the current cursor row and below (to the bottom-most line in the
    /// scrolling region) are shifted down by amount lines. The contents of the
    /// amount bottom-most lines in the scroll region are lost.
    ///
    /// This unsets the pending wrap state without wrapping. If the current cursor
    /// position is outside of the current scroll region it does nothing.
    ///
    /// If amount is greater than the remaining number of lines in the scrolling
    /// region it is adjusted down (still allowing for scrolling out every remaining
    /// line in the scrolling region)
    ///
    /// In left and right margin mode the margins are respected; lines are only
    /// scrolled in the scroll region.
    ///
    /// All cleared space is colored according to the current SGR state.
    ///
    /// Moves the cursor to the left margin.
    pub fn insert_lines(&mut self, count: usize) {
        self.shift_lines(count, true);
    }

    /// Removes amount lines from the current cursor row down. The remaining lines
    /// to the bottom margin are shifted up and space from the bottom margin up is
    /// filled with empty lines.
    ///
    /// If the current cursor position is outside of the current scroll region it
    /// does nothing. If amount is greater than the remaining number of lines in the
    /// scrolling region it is adjusted down.
    ///
    /// In left and right margin mode the margins are respected; lines are only
    /// scrolled in the scroll region.
    ///
    /// If the cell movement splits a multi cell character that character cleared,
    /// by replacing it by spaces, keeping its current attributes. All other
    /// cleared space is colored according to the current SGR state.
    ///
    /// Moves the cursor to the left margin.
    pub fn delete_lines(&mut self, count: usize) {
        self.shift_lines(count, false);
    }
}

/// Clear a stale `spacer_head` in the last cell of the row above the cursor.
fn clear_prev_row_spacer_head(scr: &mut Screen, cols: size::CellCountInt) {
    let Some(mut p) = scr.cursor_pin().up(&scr.pages, 1) else {
        return;
    };
    p.x = cols - 1;
    let page = scr.page_mut(p.node);
    if let Ok(i) = page.cell_index(usize::from(p.x), usize::from(p.y))
        && page.cells[i].wide() == Wide::SpacerHead
    {
        page.cells[i].set_wide(Wide::Narrow);
    }
}

impl Terminal {
    /// Inserts spaces at current cursor position moving existing cell contents
    /// to the right. The contents of the count right-most columns in the scroll
    /// region are lost. The cursor position is not changed.
    ///
    /// This unsets the pending wrap state without wrapping.
    ///
    /// The inserted cells are colored according to the current SGR state.
    pub fn insert_blanks(&mut self, count: usize) {
        // Unset pending wrap state without wrapping. Note: this purposely
        // happens BEFORE the scroll region check below, because that's what
        // xterm does.
        self.screen_mut().cursor.pending_wrap = false;

        // If we're given a zero then we do nothing. The rest of this function
        // assumes count > 0 and will crash if zero so return early. Note that
        // this shouldn't be possible with real CSI sequences because the value
        // is clamped to 1 min.
        if count == 0 {
            return;
        }

        let sr = self.scrolling_region;
        let cx = usize::from(self.screen().cursor.x);

        // If our cursor is outside the margins then do nothing. We DO reset
        // wrap state still so this must remain below the above logic.
        if self.screen().cursor.x < sr.left || self.screen().cursor.x > sr.right {
            return;
        }

        let pin = self.screen().cursor_pin();
        let (node, y) = (pin.node, usize::from(pin.y));

        // If our X is a wide spacer tail then we need to erase the
        // previous cell too so we don't split a multi-cell character.
        if self.screen().cursor_cell().wide() == Wide::SpacerTail {
            assert!(cx > 0);
            self.screen_mut().clear_cells(node, pin.y, cx - 1, cx + 1);
        }

        // Remaining cols from our cursor to the right margin.
        let rem = usize::from(sr.right) - cx + 1;

        // If the cell at the right margin is wide, its spacer tail is
        // outside the scroll region and would be orphaned by either the
        // shift or the clear. Clean up both halves up front.
        {
            let right = cx + rem - 1;
            if self.screen().cursor_row_cells()[right].wide() == Wide::Wide {
                self.screen_mut().clear_cells(node, pin.y, right, right + 2);
            }
        }

        // We can only insert blanks up to our remaining cols
        let adjusted_count = count.min(rem);

        // This is the amount of space at the right of the scroll region
        // that will NOT be blank, so we need to shift the correct cols right.
        // "scroll_amount" is the number of such cols.
        let scroll_amount = rem - adjusted_count;
        if scroll_amount > 0 {
            self.screen_mut()
                .page_mut(node)
                .pause_integrity_checks(true);

            let end = cx + scroll_amount - 1;

            // If our last cell we're shifting is wide, then we need to clear
            // it to be empty so we don't split the multi-cell char.
            if self.screen().cursor_row_cells()[end].wide() == Wide::Wide {
                debug_assert!(self.screen().cursor_row_cells()[end + 1].wide() == Wide::SpacerTail);
                self.screen_mut().clear_cells(node, pin.y, end, end + 2);
            }

            // We work backwards so we don't overwrite data.
            let page = self.screen_mut().page_mut(node);
            for x in (cx..=end).rev() {
                page.swap_cells(x, y, x + adjusted_count, y)
                    .expect("swap_cells");
            }

            self.screen_mut()
                .page_mut(node)
                .pause_integrity_checks(false);
        }

        // Insert blanks. The blanks preserve the background color.
        self.screen_mut()
            .clear_cells(node, pin.y, cx, cx + adjusted_count);

        // Our row is always dirty
        self.screen_mut().cursor_mark_dirty();
    }

    /// Removes amount characters from the current cursor position to the right.
    /// The remaining characters are shifted to the left and space from the right
    /// margin is filled with spaces.
    ///
    /// If amount is greater than the remaining number of characters in the
    /// scrolling region, it is adjusted down.
    ///
    /// Does not change the cursor position.
    pub fn delete_chars(&mut self, count_req: usize) {
        if count_req == 0 {
            return;
        }

        let sr = self.scrolling_region;
        let cx = usize::from(self.screen().cursor.x);

        // If our cursor is outside the margins then do nothing. We DO reset
        // wrap state still so this must remain below the above logic.
        if self.screen().cursor.x < sr.left || self.screen().cursor.x > sr.right {
            return;
        }

        let pin = self.screen().cursor_pin();
        let (node, y) = (pin.node, usize::from(pin.y));

        // Remaining cols from our cursor to the right margin.
        let rem = usize::from(sr.right) - cx + 1;

        // We can only insert blanks up to our remaining cols
        let count = count_req.min(rem);

        let scr = self.screen_mut();
        scr.split_cell_boundary(cx as size::CellCountInt);
        scr.split_cell_boundary((cx + count) as size::CellCountInt);
        scr.split_cell_boundary(sr.right + 1);

        // This is the amount of space at the right of the scroll region
        // that will NOT be blank, so we need to shift the correct cols right.
        // "scroll_amount" is the number of such cols.
        let scroll_amount = rem - count;
        if scroll_amount > 0 {
            let page = scr.page_mut(node);
            page.pause_integrity_checks(true);
            for x in cx..cx + scroll_amount {
                page.swap_cells(x + count, y, x, y).expect("swap_cells");
            }
            page.pause_integrity_checks(false);
        }

        // Insert blanks. The blanks preserve the background color.
        scr.clear_cells(node, pin.y, cx + scroll_amount, cx + rem);

        // Our row's soft-wrap is always reset.
        scr.cursor_reset_wrap();

        // Our row is always dirty
        scr.cursor_mark_dirty();
    }

    pub fn erase_chars(&mut self, count_req: usize) {
        let cx = usize::from(self.screen().cursor.x);
        let count = {
            let remaining = usize::from(self.cols) - cx;
            let mut end = remaining.min(count_req.max(1));

            // If our last cell is a wide char then we need to also clear the
            // cell beyond it since we can't just split a wide char.
            if end != remaining {
                let last = self
                    .screen()
                    .cursor_cell_right((end - 1) as size::CellCountInt);
                if last.wide() == Wide::Wide {
                    end += 1;
                }
            }

            end
        };

        // Handle any boundary conditions on the edges of the erased area.
        //
        // TODO(qwerasd): This isn't actually correct if you take in to account
        // protected modes. We need to figure out how to make `clearCells` or at
        // least `clearUnprotectedCells` handle boundary conditions...
        let scr = self.screen_mut();
        scr.split_cell_boundary(cx as size::CellCountInt);
        scr.split_cell_boundary((cx + count) as size::CellCountInt);

        // Reset our row's soft-wrap.
        scr.cursor_reset_wrap();

        // Mark our cursor row as dirty
        scr.cursor_mark_dirty();

        // Clear the cells
        let pin = scr.cursor_pin();

        // If we never had a protection mode, then we can assume no cells
        // are protected and go with the fast path. If the last protection
        // mode was not ISO we also always ignore protection attributes.
        if scr.protected_mode != ProtectedMode::Iso {
            scr.clear_cells(pin.node, pin.y, cx, cx + count);
            return;
        }

        scr.clear_unprotected_cells(pin.node, pin.y, cx, cx + count);
    }

    /// Erase the line.
    pub fn erase_line(&mut self, mode: csi::EraseLine, protected_req: bool) {
        // Get our start/end positions depending on mode.
        let (start, end) = match mode {
            csi::EraseLine::Right => {
                let mut x = usize::from(self.screen().cursor.x);

                // If our X is a wide spacer tail then we need to erase the
                // previous cell too so we don't split a multi-cell character.
                if x > 0 && self.screen().cursor_cell().wide() == Wide::SpacerTail {
                    x -= 1;
                }

                // Reset our row's soft-wrap.
                self.screen_mut().cursor_reset_wrap();

                (x, usize::from(self.cols))
            }

            csi::EraseLine::Left => {
                let mut x = usize::from(self.screen().cursor.x);

                // If our x is a wide char we need to delete the tail too.
                if self.screen().cursor_cell().wide() == Wide::Wide {
                    x += 1;
                }

                (0, x + 1)
            }

            // Note that it seems like complete should reset the soft-wrap
            // state of the line but in xterm it does not.
            csi::EraseLine::Complete => (0, usize::from(self.cols)),

            other => {
                log::error!("unimplemented erase line mode: {other:?}");
                return;
            }
        };

        // All modes will clear the pending wrap state and we know we have
        // a valid mode at this point.
        let scr = self.screen_mut();
        scr.cursor.pending_wrap = false;

        // We always mark our row as dirty
        scr.cursor_mark_dirty();

        // We respect protected attributes if explicitly requested (probably
        // a DECSEL sequence) or if our last protected mode was ISO even if its
        // not currently set.
        let protected = scr.protected_mode == ProtectedMode::Iso || protected_req;

        let pin = scr.cursor_pin();

        // If we're not respecting protected attributes, we can use a fast-path
        // to fill the entire line.
        if !protected {
            scr.clear_cells(pin.node, pin.y, start, end);
            return;
        }

        scr.clear_unprotected_cells(pin.node, pin.y, start, end);
    }

    /// Erase the display.
    pub fn erase_display(&mut self, mode: csi::EraseDisplay, protected_req: bool) {
        // We respect protected attributes if explicitly requested (probably
        // a DECSEL sequence) or if our last protected mode was ISO even if its
        // not currently set.
        let protected = self.screen().protected_mode == ProtectedMode::Iso || protected_req;

        match mode {
            csi::EraseDisplay::ScrollComplete => {
                if let Err(err) = self.screen_mut().scroll_clear() {
                    log::warn!("scroll clear failed, doing a normal clear err={err:?}");
                    self.erase_display(csi::EraseDisplay::Complete, protected_req);
                    return;
                }

                // Unsets pending wrap state
                self.screen_mut().cursor.pending_wrap = false;

                // Clear all Kitty graphics state for this screen
                self.kitty_images_delete_all();
            }

            csi::EraseDisplay::Complete => {
                // If we're on the primary screen and our last non-empty row is
                // a prompt, then we do a scroll_complete instead. This is a
                // heuristic to get the generally desirable behavior that ^L
                // at a prompt scrolls the screen contents prior to clearing.
                // Most shells send `ESC [ H ESC [ 2 J` so we can't just check
                // our current cursor position. See #905
                if self.screens.active_key == ScreenKey::Primary && self.last_row_is_prompt() {
                    // If we fail, we just fall back to doing a normal clear
                    // so we don't worry about the error.
                    let _ = self.screen_mut().scroll_clear();
                }

                // All active area
                self.screen_mut()
                    .clear_rows(Point::Active(Coordinate::default()), None, protected);

                // Unsets pending wrap state
                self.screen_mut().cursor.pending_wrap = false;

                // Clear all Kitty graphics state for this screen
                self.kitty_images_delete_all();

                // Cleared screen dirty bit
                self.flags.dirty.clear = true;
            }

            csi::EraseDisplay::Below => {
                // All lines to the right (including the cursor)
                self.erase_line(csi::EraseLine::Right, protected_req);

                // All lines below
                if self.screen().cursor.y + 1 < self.rows {
                    let y = u32::from(self.screen().cursor.y) + 1;
                    self.screen_mut().clear_rows(
                        Point::Active(Coordinate { x: 0, y }),
                        None,
                        protected,
                    );
                }

                // Unsets pending wrap state. Should be done by erase_line.
                assert!(!self.screen().cursor.pending_wrap);
            }

            csi::EraseDisplay::Above => {
                // Erase to the left (including the cursor)
                self.erase_line(csi::EraseLine::Left, protected_req);

                // All lines above
                if self.screen().cursor.y > 0 {
                    let y = u32::from(self.screen().cursor.y) - 1;
                    self.screen_mut().clear_rows(
                        Point::Active(Coordinate { x: 0, y: 0 }),
                        Some(Point::Active(Coordinate { x: 0, y })),
                        protected,
                    );
                }

                // Unsets pending wrap state
                assert!(!self.screen().cursor.pending_wrap);
            }

            csi::EraseDisplay::Scrollback => self
                .screen_mut()
                .erase_rows(Point::History(Coordinate::default()), None),
        }
    }

    /// The "are we at a prompt" heuristic of `erase_display(.complete)`.
    fn last_row_is_prompt(&self) -> bool {
        let scr = self.screen();
        let Some(active_br) = scr.pages.get_bottom_right(crate::point::Tag::Active) else {
            return false;
        };
        let top = scr.pages.get_top_left(crate::point::Tag::Active);
        // Go from the bottom of the active up and see if we're
        // at a prompt.
        // (The Zig loop decides on the first row it sees.)
        let Some(p) = active_br
            .row_iterator(&scr.pages, Direction::LeftUp, top)
            .next()
        else {
            return false;
        };
        let row = p.row_and_cell(&scr.pages).expect("row").0;
        match row.semantic_prompt() {
            // If we're at a prompt or input area, then we are at a prompt.
            RowPrompt::Prompt | RowPrompt::PromptContinuation => true,

            // If we have command output, then we're most certainly not
            // at a prompt.
            RowPrompt::None => false,
        }
    }

    /// Delete all kitty graphics state for the active screen.
    fn kitty_images_delete_all(&mut self) {
        let info = crate::kitty::graphics_storage::TerminalInfo::from_terminal(self);
        let screen = self.screen_mut();
        screen.kitty_images.delete(
            &mut screen.pages,
            &info,
            kitty_graphics_command::Delete::All(true),
        );
    }

    /// Resets all margins and fills the whole screen with the character 'E'
    ///
    /// Sets the cursor to the top left corner.
    pub fn decaln(&mut self) -> Result<(), Error> {
        // Clear our stylistic attributes. This is the only thing that can
        // fail so we do it first so we can undo it.
        let old_style = self.screen().cursor.style;
        {
            let scr = self.screen_mut();
            scr.cursor.style = Style {
                bg_color: scr.cursor.style.bg_color,
                fg_color: scr.cursor.style.fg_color,
                ..Style::default()
            };
            if let Err(e) = scr.manual_style_update() {
                scr.cursor.style = old_style;
                return Err(e);
            }
        }

        // Reset margins, also sets cursor to top-left
        self.scrolling_region = ScrollingRegion {
            top: 0,
            bottom: self.rows - 1,
            left: 0,
            right: self.cols - 1,
        };

        // Origin mode is disabled
        self.modes.set(Mode::Origin, false);

        // Move our cursor to the top-left
        self.set_cursor_pos(1, 1);

        // Use clearRows instead of eraseDisplay because we must NOT respect
        // protected attributes here.
        self.screen_mut()
            .clear_rows(Point::Active(Coordinate::default()), None, false);

        // Fill with Es by moving the cursor but reset it after.
        let rows = self.rows;
        loop {
            let scr = self.screen_mut();
            let pin = scr.cursor_pin();
            let style_id = scr.cursor.style_id;
            let page = scr.page_mut(pin.node);
            let y = usize::from(pin.y);
            let base = page.rows[y].cells();
            let cols = usize::from(page.size.cols);
            let mut e = Cell::init(u32::from('E'));
            e.set_style_id(style_id);
            // DECALN does not respect protected state. Verified with xterm.
            e.set_protected(false);
            page.cells[base..base + cols].fill(e);

            // If we have a ref-counted style, increase
            if style_id != style::DEFAULT_ID {
                let _ = page.styles.use_multiple(style_id, cols as u16);
                page.rows[y].set_styled(true);
            }

            // We messed with the page so assert its integrity here.
            page.assert_integrity();

            scr.cursor_mark_dirty();
            if scr.cursor.y == rows - 1 {
                break;
            }
            scr.cursor_down(1);
        }

        // Reset the cursor to the top-left
        self.set_cursor_pos(1, 1);
        Ok(())
    }

    /// Execute a kitty graphics command. The response (if any) is a full,
    /// valid APC sequence payload to send to the pty.
    ///
    /// If an error occurs, the caller should response to the pty that a
    /// an error occurred otherwise the behavior of the graphics protocol is
    /// undefined.
    ///
    pub fn kitty_graphics(
        &mut self,
        cmd: &mut kitty_graphics_command::Command,
    ) -> Option<kitty_graphics_command::Response> {
        self.kitty_graphics_command(cmd)
    }

    /// Execute a borrowed Kitty graphics command.
    pub fn kitty_graphics_command(
        &mut self,
        cmd: &kitty_graphics_command::Command,
    ) -> Option<kitty_graphics_command::Response> {
        crate::kitty::graphics_exec::execute(self, cmd)
    }

    /// Set a style attribute.
    pub fn set_attribute(&mut self, attr: sgr::Attribute<'_>) -> Result<(), Error> {
        self.screen_mut().set_attribute(attr)
    }

    /// Print the active attributes as a string. This is used to respond to DECRQSS
    /// requests.
    ///
    /// Boolean attributes are printed first, followed by foreground color, then
    /// background color. Each attribute is separated by a semicolon.
    pub fn print_attributes(&self) -> String {
        use std::fmt::Write;
        // The SGR response always starts with a 0. See https://vt100.net/docs/vt510-rm/DECRPSS
        let mut out = String::from("0");

        let pen = self.screen().cursor.style;
        let mut attrs = Vec::with_capacity(8);

        if pen.flags.bold {
            attrs.push('1');
        }
        if pen.flags.faint {
            attrs.push('2');
        }
        if pen.flags.italic {
            attrs.push('3');
        }
        if pen.flags.underline != ghostty_vt::sgr::Underline::None {
            attrs.push('4');
        }
        if pen.flags.blink {
            attrs.push('5');
        }
        if pen.flags.inverse {
            attrs.push('7');
        }
        if pen.flags.invisible {
            attrs.push('8');
        }
        if pen.flags.strikethrough {
            attrs.push('9');
        }

        for c in attrs {
            let _ = write!(out, ";{c}");
        }

        match pen.fg_color {
            StyleColor::None => {}
            StyleColor::Palette(idx) => {
                if idx >= 16 {
                    let _ = write!(out, ";38:5:{idx}");
                } else if idx >= 8 {
                    let _ = write!(out, ";9{}", idx - 8);
                } else {
                    let _ = write!(out, ";3{idx}");
                }
            }
            StyleColor::Rgb(rgb) => {
                let _ = write!(out, ";38:2::{}:{}:{}", rgb.r, rgb.g, rgb.b);
            }
        }

        match pen.bg_color {
            StyleColor::None => {}
            StyleColor::Palette(idx) => {
                if idx >= 16 {
                    let _ = write!(out, ";48:5:{idx}");
                } else if idx >= 8 {
                    let _ = write!(out, ";10{}", idx - 8);
                } else {
                    let _ = write!(out, ";4{idx}");
                }
            }
            StyleColor::Rgb(rgb) => {
                let _ = write!(out, ";48:2::{}:{}:{}", rgb.r, rgb.g, rgb.b);
            }
        }

        out
    }

    /// DECCOLM changes the terminal width between 80 and 132 columns. This
    /// function call will do NOTHING unless `setDeccolmSupported` has been
    /// called with "true".
    ///
    /// This breaks the expectation around modern terminals that they resize
    /// with the window. This will fix the grid at either 80 or 132 columns.
    /// The rows will continue to be variable.
    pub fn deccolm(&mut self, mode: DeccolmMode) -> Result<(), Error> {
        // If DEC mode 40 isn't enabled, then this is ignored. We also make
        // sure that we don't have deccolm set because we want to fully ignore
        // set mode.
        if !self.modes.get(Mode::EnableMode3) {
            self.modes.set(Mode::M132Column, false);
            return Ok(());
        }

        // Enable it
        self.modes
            .set(Mode::M132Column, mode == DeccolmMode::Cols132);

        // Resize to the requested size
        let rows = self.rows;
        self.resize(
            match mode {
                DeccolmMode::Cols132 => 132,
                DeccolmMode::Cols80 => 80,
            },
            rows,
        )?;

        // Erase our display and move our cursor.
        self.erase_display(csi::EraseDisplay::Complete, false);
        self.set_cursor_pos(1, 1);
        Ok(())
    }

    /// Resize the underlying terminal.
    pub fn resize(
        &mut self,
        cols: size::CellCountInt,
        rows: size::CellCountInt,
    ) -> Result<(), Error> {
        // If our cols/rows didn't change then we're done
        if self.cols == cols && self.rows == rows {
            return Ok(());
        }

        // Resize our tabstops
        if self.cols != cols {
            self.tabstops = Tabstops::new(usize::from(cols), 8).map_err(map_tabstops_err)?;
        }

        // Resize primary screen, which supports reflow
        let reflow = self.modes.get(Mode::Wraparound);
        let prompt_redraw = self.flags.shell_redraws_prompt;
        let primary = self.screens.get_mut(ScreenKey::Primary).expect("primary");
        primary.resize(ScreenResize {
            cols,
            rows,
            reflow,
            prompt_redraw,
        })?;

        // Alternate screen, if it exists, doesn't reflow
        if let Some(alt) = self.screens.get_mut(ScreenKey::Alternate) {
            alt.resize(ScreenResize {
                cols,
                rows,
                reflow: false,
                prompt_redraw: Redraw::False,
            })?;
        }

        // Whenever we resize we just mark it as a screen clear
        self.flags.dirty.clear = true;

        // Set our size
        self.cols = cols;
        self.rows = rows;

        // Reset the scrolling region
        self.scrolling_region = ScrollingRegion {
            top: 0,
            bottom: rows - 1,
            left: 0,
            right: cols - 1,
        };
        Ok(())
    }

    /// Set the pwd for the terminal.
    pub fn set_pwd(&mut self, pwd: &[u8]) -> Result<(), Error> {
        self.pwd.clear();
        self.pwd.extend_from_slice(pwd);
        Ok(())
    }

    /// Returns the pwd for the terminal, if any. The memory is owned by the
    /// Terminal and is not copied. It is safe until a reset or set_pwd.
    pub fn get_pwd(&self) -> Option<&[u8]> {
        if self.pwd.is_empty() {
            return None;
        }
        Some(&self.pwd)
    }

    /// Switch to the given screen type (alternate or primary).
    ///
    /// This does NOT handle behaviors such as clearing the screen,
    /// copying the cursor, etc. This should be handled by downstream
    /// callers.
    ///
    /// After calling this function, the active screen is `key`, and the
    /// returned value will be the key of the previous screen. If the return
    /// value is `None`, then the screen was not switched because it was
    /// already the active screen.
    ///
    /// Note: This is written in a generic way so that we can support
    /// more than two screens in the future if needed. There isn't
    /// currently a spec for this, but it is something I think might
    /// be useful in the future.
    pub fn switch_screen(&mut self, key: ScreenKey) -> Result<Option<ScreenKey>, Error> {
        // If we're already on the requested screen we do nothing.
        if self.screens.active_key == key {
            return Ok(None);
        }
        let old_key = self.screens.active_key;

        // We always end hyperlink state when switching screens.
        // We need to do this on the original screen.
        self.screens.active_mut().end_hyperlink();

        // Switch the screens/
        if self.screens.get(key).is_none() {
            let primary = self.screens.get(ScreenKey::Primary).expect("primary");
            let opts = ScreenOptions {
                cols: self.cols,
                rows: self.rows,
                max_scrollback: match key {
                    ScreenKey::Primary => primary.pages.explicit_max_size,
                    ScreenKey::Alternate => 0,
                },

                // Inherit our Kitty image storage limit from the primary
                // screen if we have to initialize.
                kitty_image_storage_limit: primary.kitty_images.total_limit,
            };
            self.screens.get_init(key, opts)?;
        }
        let old_charset = self.screens.active().charset;
        let new = self.screens.get_mut(key).expect("screen exists");

        // The new screen should not have any hyperlinks set
        assert!(new.cursor.hyperlink_id == 0);

        // Bring our charset state with us
        new.charset = old_charset;

        // Clear our selection
        new.clear_selection();

        // Mark kitty images as dirty so they redraw. Without this set
        // the images will remain where they were (the dirty bit on
        // the screen only tracks the terminal grid, not the images).
        new.kitty_images.dirty = true;

        // Mark our terminal as dirty to redraw the grid.
        self.flags.dirty.clear = true;

        // Finalize the switch
        self.screens.switch_to(key);

        Ok(Some(old_key))
    }

    /// Switch screen via a mode switch (e.g. mode 47, 1047, 1049).
    /// This is a much more opinionated operation than `switch_screen`
    /// since it also handles the behaviors of the specific mode,
    /// such as clearing the screen, saving/restoring the cursor,
    /// etc.
    ///
    /// This should be used for legacy compatibility with VT protocols,
    /// but more modern usage should use `switch_screen` instead and handle
    /// details like clearing the screen, cursor saving, etc. manually.
    pub fn switch_screen_mode(
        &mut self,
        mode: SwitchScreenMode,
        enabled: bool,
    ) -> Result<(), Error> {
        // The behavior in this function is completely based on reading
        // the xterm source, specifically "charproc.c" for
        // `srm_ALTBUF`, `srm_OPT_ALTBUF`, and `srm_OPT_ALTBUF_CURSOR`.
        // We shouldn't touch anything in here without adding a unit
        // test AND verifying the behavior with xterm.

        match mode {
            SwitchScreenMode::M47 => {}

            // If we're disabling 1047 and we're on alt screen then
            // we clear the screen.
            SwitchScreenMode::M1047 => {
                if !enabled && self.screens.active_key == ScreenKey::Alternate {
                    self.erase_display(csi::EraseDisplay::Complete, false);
                }
            }

            // 1049 unconditionally saves the cursor on enabling, even
            // if we're already on the alternate screen.
            SwitchScreenMode::M1049 => {
                if enabled {
                    self.save_cursor();
                }
            }
        }

        // Switch screens first to whatever we're going to.
        let to = if enabled {
            ScreenKey::Alternate
        } else {
            ScreenKey::Primary
        };
        let old = self.switch_screen(to)?;

        match mode {
            // For these modes, we need to copy the cursor. We only copy
            // the cursor if the screen actually changed, otherwise the
            // cursor is already copied. The cursor is copied regardless
            // of destination screen.
            SwitchScreenMode::M47 | SwitchScreenMode::M1047 => {
                if let Some(old) = old {
                    self.copy_cursor_from(old);
                }
            }

            // Mode 1049 restores cursor on the primary screen when
            // we disable it.
            SwitchScreenMode::M1049 => {
                if enabled {
                    assert!(self.screens.active_key == ScreenKey::Alternate);
                    self.erase_display(csi::EraseDisplay::Complete, false);

                    // When we enter alt screen with 1049, we always copy the
                    // cursor from the primary screen (if we weren't already
                    // on it).
                    if let Some(old) = old {
                        self.copy_cursor_from(old);
                    }
                } else {
                    assert!(self.screens.active_key == ScreenKey::Primary);
                    self.restore_cursor();
                }
            }
        }
        Ok(())
    }

    fn copy_cursor_from(&mut self, old: ScreenKey) {
        let other = self.screens.get(old).expect("old screen").cursor.clone();
        if let Err(err) = self.screen_mut().cursor_copy(&other, false) {
            log::warn!("cursor copy failed entering alt screen err={err:?}");
        }
    }

    /// Return the current string value of the terminal. Newlines are
    /// encoded as "\n". This omits any formatting such as fg/bg.
    pub fn plain_string(&self) -> Result<String, Error> {
        self.screen()
            .dump_string_alloc(Point::Viewport(Coordinate::default()))
    }

    /// Same as plain_string, but respects row wrap state when building the string.
    pub fn plain_string_unwrapped(&self) -> Result<String, Error> {
        self.screen()
            .dump_string_alloc_unwrapped(Point::Viewport(Coordinate::default()))
    }

    /// Full reset.
    ///
    /// This will attempt to free the existing screen memory but if that fails
    /// this will reuse the existing memory. In the latter case, memory may
    /// be wasted (since its unused) but it isn't leaked.
    pub fn full_reset(&mut self) {
        // Ensure we're back on primary screen
        self.screens.switch_to(ScreenKey::Primary);
        self.screens.remove(ScreenKey::Alternate);

        // Reset our screens
        self.screen_mut().reset().expect("screen reset");

        // Rest our basic state
        self.modes.reset();
        self.flags = Flags::default();
        self.tabstops.reset(TABSTOP_INTERVAL);
        self.previous_char = None;
        self.pwd.clear();
        self.status_display = StatusDisplay::Main;
        self.scrolling_region = ScrollingRegion {
            top: 0,
            bottom: self.rows - 1,
            left: 0,
            right: self.cols - 1,
        };

        // Always mark dirty so we redraw everything
        self.flags.dirty.clear = true;
    }

    /// Returns true if the point is dirty, used for testing.
    pub fn is_dirty(&self, pt: Point) -> bool {
        let scr = self.screen();
        let pin = scr.pages.pin(pt).expect("valid point");
        pin.is_dirty(&scr.pages)
    }

    /// Clear all dirty bits. Testing only.
    pub fn clear_dirty(&mut self) {
        self.screen_mut().pages.clear_dirty();
    }
}

#[cfg(test)]
mod tests;

impl crate::kitty::graphics_exec::ExecTerminal for Terminal {
    fn kitty_parts(
        &mut self,
    ) -> (
        &mut crate::kitty::graphics_storage::ImageStorage,
        &mut crate::page_list::PageList,
    ) {
        let screen = self.screens.active_mut();
        (&mut screen.kitty_images, &mut screen.pages)
    }
    fn exec_info(&self) -> crate::kitty::graphics_storage::TerminalInfo {
        crate::kitty::graphics_storage::TerminalInfo::from_terminal(self)
    }
    fn exec_cursor_pin(&self) -> Pin {
        self.screen().cursor_pin()
    }
    fn exec_index(&mut self) -> Result<(), Error> {
        self.index()
    }
    fn exec_set_cursor_pos(&mut self, row: usize, col: usize) {
        self.set_cursor_pos(row, col);
    }
}
