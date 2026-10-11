//! Terminal state formatters: plain text, VT (with styles) and HTML.
//!
//! Port of `ZIG/src/terminal/formatter.zig`.
//!
//! Writers are `&mut dyn std::io::Write`; use the `format_string` helpers (or
//! write into a `Vec<u8>`) for in-memory output. Where Zig used a
//! `Discarding` writer to count bytes (for pin maps), the Rust port renders
//! into a small scratch buffer and measures it.
#![allow(clippy::too_many_arguments)]

use crate::{
    highlight::pin_page_iterator,
    hyperlink::HyperlinkId,
    page::{Cell, ContentTag, Page, Wide},
    page_list::{PageList, Pin},
    point::{Coordinate, Tag},
    screen::Screen,
    selection::Selection,
    size,
    style::{Color, Style},
    terminal::Terminal,
};
use ghostty_vt::{
    charsets::{Charset, Slots},
    color::{Palette, Rgb},
    modes::ENTRIES,
};
use std::io::{self, Write};

type Result<T> = io::Result<T>;

/// Formats available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Plain text.
    Plain,

    /// Include VT sequences to preserve colors, styles, URLs, etc.
    /// This is predominantly SGR sequences but may contain others as needed.
    ///
    /// Note that for reference colors, like palette indices, this will
    /// vary based on the formatter and you should see the docs. For example,
    /// PageFormatter with VT will emit SGR sequences with palette indices,
    /// not the color itself.
    ///
    /// For VT, newlines will be emitted as `\r\n` so that the cursor properly
    /// moves back to the beginning prior emitting follow-up lines.
    Vt,

    /// HTML output.
    ///
    /// This will emit inline styles for as much styling as possible,
    /// in the interest of simplicity and ease of editing. This isn't meant
    /// to build the most beautiful or efficient HTML, but rather to be
    /// stylistically correct.
    ///
    /// For colors, RGB values are emitted as inline CSS (#RRGGBB) while palette
    /// indices use CSS variables (var(--vt-palette-N)). The palette colors are
    /// emitted by TerminalFormatter.Extra.palette as a <style> block if you
    /// want to also include that. But if you only format a screen or lower,
    /// the formatter doesn't have access to the current palette to render it.
    ///
    /// Newlines are emitted as actual '\n' characters. Consumers should use
    /// CSS white-space: pre or pre-wrap to preserve spacing and alignment.
    Html,
}

impl Format {
    pub fn styled(self) -> bool {
        match self {
            Format::Plain => false,
            Format::Html | Format::Vt => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodepointMap {
    /// Unicode codepoint range to replace.
    /// Asserts: range[0] <= range[1]
    pub range: [u32; 2],

    /// Replacement value for this range.
    pub replacement: Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// A single replacement codepoint.
    Codepoint(u32),

    /// A string to replace with.
    String(String),
}

/// Common encoding options regardless of what exact formatter is used.
#[derive(Debug, Clone)]
pub struct Options {
    /// The format to emit.
    pub emit: Format,

    /// Whether to unwrap soft-wrapped lines. If false, this will emit the
    /// screen contents as it is rendered on the page in the given size.
    pub unwrap: bool,

    /// Trim trailing whitespace on lines with other text. Trailing blank
    /// lines are always trimmed. This only affects trailing whitespace
    /// on rows that have at least one other cell with text. Whitespace
    /// is currently only space characters (0x20).
    pub trim: bool,

    /// Replace matching Unicode codepoints with some other values.
    /// This will use the last matching range found in the list.
    pub codepoint_map: Option<Vec<CodepointMap>>,

    /// Set a background and foreground color to use for the "screen".
    /// For styled formats, this will emit the proper sequences or styles.
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,

    /// If set, then styled formats in `emit` will use this palette to
    /// emit colors directly as RGB. If this is None, styled formats will
    /// still work but will use deferred palette styling (e.g. CSS variables
    /// for HTML or the actual palette indexes for VT).
    pub palette: Option<Palette>,
}

impl Options {
    pub fn new(emit: Format) -> Options {
        Options {
            emit,
            unwrap: false,
            trim: true,
            codepoint_map: Some(Vec::new()),
            background: None,
            foreground: None,
            palette: None,
        }
    }

    pub fn plain() -> Options {
        Options::new(Format::Plain)
    }

    pub fn vt() -> Options {
        Options::new(Format::Vt)
    }

    pub fn html() -> Options {
        Options::new(Format::Html)
    }
}

/// Maps byte positions in formatted output to PageList pins.
///
/// Used by formatters that operate on PageLists to track the source position
/// of each byte written. (Zig: `PinMap{alloc, map}`; here simply the vector to
/// append to.)
pub type PinMap<'a> = &'a mut Vec<Pin>;

fn hex(rgb: Rgb) -> String {
    format!("{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b)
}

fn invalid_cp() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid codepoint")
}

fn clone_pin(p: Pin) -> Pin {
    Pin {
        node: p.node,
        x: p.x,
        y: p.y,
        garbage: false,
    }
}

/// Append `count` copies of the last pin in `map` (or the top-left of the
/// screen when empty).
fn map_pad_last_or_top(map: &mut Vec<Pin>, pages: &PageList, count: usize) {
    let pin = match map.last() {
        Some(last) => clone_pin(*last),
        None => pages.get_top_left(Tag::Screen).expect("top left"),
    };
    map.extend(std::iter::repeat_n(pin, count));
}

/// Terminal formatter formats the active terminal screen.
///
/// This will always only emit data related to the currently active screen.
/// If you want to emit data for a specific screen (e.g. primary vs alt), then
/// switch to that screen in the terminal prior to using this.
///
/// If you want to emit data for all screens (a less common operation), then
/// you must create a no-content TerminalFormatter followed by multiple
/// explicit ScreenFormatter calls. This isn't a common operation so this
/// little extra work should be acceptable.
///
/// For styled formatting, this will emit the palette colors at the
/// beginning so that the output can be rendered properly according to
/// the current terminal state.
pub struct TerminalFormatter<'a> {
    /// The terminal to format.
    pub terminal: &'a Terminal,

    /// The common options
    pub opts: Options,

    /// The content to include.
    pub content: Content,

    /// Extra stuff to emit, such as terminal modes, palette, cursor, etc.
    /// This information is ONLY emitted when the format is "vt".
    pub extra: TerminalExtra,

    /// If non-null, then `map` will contain the Pin of every byte
    /// byte written to the writer offset by the byte index.
    ///
    /// Note that some emitted bytes may not correspond to any Pin, such as
    /// the extra data around terminal state (palette, modes, etc.). For these,
    /// we'll map it to the most previous pin so there is some continuity but
    /// its an arbitrary choice.
    ///
    /// Warning: there is a significant performance hit to track this
    pub pin_map: Option<PinMap<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalExtra {
    /// Emit the palette using OSC 4 sequences.
    pub palette: bool,

    /// Emit terminal modes that differ from their defaults using CSI h/l
    /// sequences. Defaults are according to the Ghostty defaults which
    /// are generally match most terminal defaults. This will include
    /// things like current screen, bracketed mode, mouse event reporting,
    /// etc.
    pub modes: bool,

    /// Emit scrolling region state using DECSTBM and DECSLRM sequences.
    pub scrolling_region: bool,

    /// Emit tabstop positions by clearing all tabs (CSI 3 g) and setting
    /// each configured tabstop with HTS.
    pub tabstops: bool,

    /// Emit the present working directory using OSC 7.
    pub pwd: bool,

    /// Emit keyboard modes such as ModifyOtherKeys using CSI > 4 m
    /// sequences.
    pub keyboard: bool,

    /// The screen extras to emit. TerminalFormatter always only
    /// emits data for the currently active screen. If you want to emit
    /// data for all screens, you should manually construct a no-content
    /// terminal formatter, followed by screen formatters.
    pub screen: ScreenExtra,
}

impl TerminalExtra {
    /// Emit nothing.
    pub const NONE: TerminalExtra = TerminalExtra {
        palette: false,
        modes: false,
        scrolling_region: false,
        tabstops: false,
        pwd: false,
        keyboard: false,
        screen: ScreenExtra::NONE,
    };

    /// Emit style-relevant information only such as palettes.
    pub const STYLES: TerminalExtra = TerminalExtra {
        palette: true,
        modes: false,
        scrolling_region: false,
        tabstops: false,
        pwd: false,
        keyboard: false,
        screen: ScreenExtra::STYLES,
    };

    /// Emit everything. This reconstructs the terminal state as closely
    /// as possible.
    pub const ALL: TerminalExtra = TerminalExtra {
        palette: true,
        modes: true,
        scrolling_region: true,
        tabstops: true,
        pwd: true,
        keyboard: true,
        screen: ScreenExtra::ALL,
    };
}

impl<'a> TerminalFormatter<'a> {
    pub fn new(terminal: &'a Terminal, opts: Options) -> Self {
        TerminalFormatter {
            terminal,
            opts,
            content: Content::Selection(None),
            extra: TerminalExtra::STYLES,
            pin_map: None,
        }
    }

    /// Format into a new `String` (convenience, no pin map is touched
    /// beyond what `pin_map` asks for).
    pub fn format_string(self) -> Result<String> {
        let mut buf = Vec::new();
        self.format(&mut buf)?;
        String::from_utf8(buf).map_err(|_| invalid_cp())
    }

    pub fn format(mut self, writer: &mut dyn Write) -> Result<()> {
        let pages = &self.terminal.screens.active().pages;

        // Emit palette before screen content if using VT format. Technically
        // we could do this after but this way if replay is slow for whatever
        // reason the colors will be right right away.
        if self.extra.palette {
            let mut buf: Vec<u8> = Vec::new();
            match self.opts.emit {
                Format::Plain => {}

                Format::Vt => {
                    for (i, rgb) in self.terminal.colors.palette.current.iter().enumerate() {
                        write!(
                            buf,
                            "\x1b]4;{};rgb:{:02x}/{:02x}/{:02x}\x1b\\",
                            i, rgb.r, rgb.g, rgb.b
                        )?;
                    }
                }

                // For HTML, we emit CSS to setup our palette variables.
                Format::Html => {
                    buf.extend_from_slice(b"<style>:root{");
                    for (i, rgb) in self.terminal.colors.palette.current.iter().enumerate() {
                        write!(
                            buf,
                            "--vt-palette-{}: #{:02x}{:02x}{:02x};",
                            i, rgb.r, rgb.g, rgb.b
                        )?;
                    }
                    buf.extend_from_slice(b"}</style>");
                }
            }
            writer.write_all(&buf)?;

            // If we have a pin_map, add the bytes we wrote to map. Map all
            // those bytes to the same pin. Use the top left to ensure the
            // node is always properly initialized.
            if self.opts.emit != Format::Plain
                && let Some(m) = self.pin_map.as_deref_mut()
            {
                let top = pages.get_top_left(Tag::Screen).expect("top left");
                m.extend(std::iter::repeat_n(top, buf.len()));
            }
        }

        // Emit terminal modes that differ from defaults. We probably have
        // some modes we want to emit before and some after, but for now for
        // simplicity we just emit them all before. If we make this more complex
        // later we should add test cases for it.
        if self.opts.emit == Format::Vt && self.extra.modes {
            let mut buf: Vec<u8> = Vec::new();
            for entry in ENTRIES {
                let current = self.terminal.modes.get(entry.mode);
                let default_val = self.terminal.modes.default.get(entry.mode);

                if current != default_val {
                    let prefix = if entry.ansi { "" } else { "?" };
                    let suffix = if current { "h" } else { "l" };
                    write!(buf, "\x1b[{}{}{}", prefix, entry.value, suffix)?;
                }
            }
            writer.write_all(&buf)?;

            if let Some(m) = self.pin_map.as_deref_mut() {
                let top = pages.get_top_left(Tag::Screen).expect("top left");
                m.extend(std::iter::repeat_n(top, buf.len()));
            }
        }

        let mut screen_formatter =
            ScreenFormatter::new(self.terminal.screens.active(), self.opts.clone());
        screen_formatter.content = self.content;
        screen_formatter.extra = self.extra.screen;
        screen_formatter.pin_map = self.pin_map.as_deref_mut();
        screen_formatter.format(writer)?;

        // Extra terminal state to emit after the screen contents so that
        // it doesn't impact the emitted contents.
        if self.opts.emit == Format::Vt {
            let mut buf: Vec<u8> = Vec::new();

            // Emit scrolling region using DECSTBM and DECSLRM
            if self.extra.scrolling_region {
                let region = &self.terminal.scrolling_region;

                // DECSTBM: top and bottom margins (1-indexed)
                // Only emit if not the full screen
                if region.top != 0 || region.bottom != self.terminal.rows - 1 {
                    write!(buf, "\x1b[{};{}r", region.top + 1, region.bottom + 1)?;
                }

                // DECSLRM: left and right margins (1-indexed)
                // Only emit if not the full width
                if region.left != 0 || region.right != self.terminal.cols - 1 {
                    write!(buf, "\x1b[{};{}s", region.left + 1, region.right + 1)?;
                }
            }

            // Emit tabstop positions
            if self.extra.tabstops {
                // Clear all tabs (CSI 3 g)
                write!(buf, "\x1b[3g")?;

                // Set each configured tabstop by moving cursor and using HTS
                for col in 0..usize::from(self.terminal.cols) {
                    if self.terminal.tabstops.get(col) {
                        // Move cursor to the column (1-indexed)
                        write!(buf, "\x1b[{}G", col + 1)?;
                        // Set tab (HTS)
                        write!(buf, "\x1bH")?;
                    }
                }
            }

            // Emit keyboard modes such as ModifyOtherKeys
            if self.extra.keyboard {
                // Only emit if modify_other_keys_2 is true
                if self.terminal.flags.modify_other_keys_2 {
                    write!(buf, "\x1b[>4;2m")?;
                }
            }

            // Emit present working directory using OSC 7
            if self.extra.pwd {
                let pwd = &self.terminal.pwd;
                if !pwd.is_empty() {
                    buf.extend_from_slice(b"\x1b]7;");
                    buf.extend_from_slice(pwd);
                    buf.extend_from_slice(b"\x1b\\");
                }
            }

            writer.write_all(&buf)?;

            // If we have a pin_map, add the bytes we wrote to map.
            if let Some(m) = self.pin_map.as_deref_mut() {
                map_pad_last_or_top(m, pages, buf.len());
            }
        }
        Ok(())
    }
}

/// Screen formatter formats a single terminal screen (e.g. primary vs alt).
pub struct ScreenFormatter<'a> {
    /// The screen to format.
    pub screen: &'a Screen,

    /// The common options
    pub opts: Options,

    /// The content to include.
    pub content: Content,

    /// Extra stuff to emit, such as cursor, style, hyperlinks, etc.
    /// This information is ONLY emitted when the format is "vt".
    pub extra: ScreenExtra,

    /// If non-null, then `map` will contain the Pin of every byte
    /// byte written to the writer offset by the byte index.
    ///
    /// Note that some emitted bytes may not correspond to any Pin, such as
    /// the extra data around screen state. For these, we'll map it to the
    /// most previous pin so there is some continuity but its an arbitrary
    /// choice.
    ///
    /// Warning: there is a significant performance hit to track this
    pub pin_map: Option<PinMap<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// Emit no content, only terminal state such as modes, palette, etc.
    /// via extra.
    None,

    /// Emit the content specified by the selection. None for all.
    /// The selection is inclusive on both ends.
    Selection(Option<Selection>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenExtra {
    /// Emit cursor position using CUP (CSI H).
    pub cursor: bool,

    /// Emit current SGR style state based on the cursor's active style_id.
    /// This reconstructs the SGR attributes (bold, italic, colors, etc.) at
    /// the cursor position.
    pub style: bool,

    /// Emit current hyperlink state using OSC 8 sequences.
    /// This sets the active hyperlink based on cursor.hyperlink_id.
    pub hyperlink: bool,

    /// Emit character protection mode using DECSCA.
    pub protection: bool,

    /// Emit Kitty keyboard protocol state using CSI > u and CSI = sequences.
    pub kitty_keyboard: bool,

    /// Emit character set designations and invocations.
    /// This includes G0-G3 designations (ESC ( ) * +) and GL/GR invocations.
    pub charsets: bool,
}

impl ScreenExtra {
    /// Emit nothing.
    pub const NONE: ScreenExtra = ScreenExtra {
        cursor: false,
        style: false,
        hyperlink: false,
        protection: false,
        kitty_keyboard: false,
        charsets: false,
    };

    /// Emit style-relevant information only.
    pub const STYLES: ScreenExtra = ScreenExtra {
        cursor: false,
        style: true,
        hyperlink: true,
        protection: false,
        kitty_keyboard: false,
        charsets: false,
    };

    /// Emit everything. This reconstructs the screen state as closely
    /// as possible.
    pub const ALL: ScreenExtra = ScreenExtra {
        cursor: true,
        style: true,
        hyperlink: true,
        protection: true,
        kitty_keyboard: true,
        charsets: true,
    };

    fn is_set(self) -> bool {
        self != Self::NONE
    }
}

impl<'a> ScreenFormatter<'a> {
    pub fn new(screen: &'a Screen, opts: Options) -> Self {
        ScreenFormatter {
            screen,
            opts,
            content: Content::Selection(None),
            extra: ScreenExtra::NONE,
            pin_map: None,
        }
    }

    /// Format into a new `String`.
    pub fn format_string(self) -> Result<String> {
        let mut buf = Vec::new();
        self.format(&mut buf)?;
        String::from_utf8(buf).map_err(|_| invalid_cp())
    }

    pub fn format(mut self, writer: &mut dyn Write) -> Result<()> {
        match self.content {
            Content::None => {}

            Content::Selection(selection) => {
                // Emit our pagelist contents according to our selection.
                let mut list_formatter =
                    PageListFormatter::new(&self.screen.pages, self.opts.clone());
                list_formatter.pin_map = self.pin_map.as_deref_mut();
                if let Some(sel) = selection {
                    list_formatter.top_left = Some(sel.top_left(&self.screen.pages));
                    list_formatter.bottom_right = Some(sel.bottom_right(&self.screen.pages));
                    list_formatter.rectangle = sel.rectangle;
                }
                list_formatter.format(writer)?;
            }
        }

        // Emit extra screen state after content if we care. The state has
        // to be emitted after since some state such as cursor position and
        // style are impacted by content rendering.
        match self.opts.emit {
            Format::Plain => return Ok(()),
            Format::Vt => {
                if !self.extra.is_set() {
                    return Ok(());
                }
            }

            // HTML doesn't preserve any screen state because it has
            // nothing to do with rendering.
            Format::Html => return Ok(()),
        }

        let mut buf: Vec<u8> = Vec::new();

        // Emit current SGR style state
        if self.extra.style {
            let cursor = &self.screen.cursor;
            write!(buf, "{}", cursor.style.formatter_vt())?;
        }

        // Emit current hyperlink state using OSC 8
        if self.extra.hyperlink
            && let Some(link) = &self.screen.cursor.hyperlink
        {
            // Start hyperlink with uri (and explicit id if present)
            match &link.id {
                HyperlinkId::Explicit(id) => {
                    buf.extend_from_slice(b"\x1b]8;id=");
                    buf.extend_from_slice(id);
                    buf.push(b';');
                    buf.extend_from_slice(&link.uri);
                    buf.extend_from_slice(b"\x1b\\");
                }
                HyperlinkId::Implicit(_) => {
                    buf.extend_from_slice(b"\x1b]8;;");
                    buf.extend_from_slice(&link.uri);
                    buf.extend_from_slice(b"\x1b\\");
                }
            }
        }

        // Emit character protection mode using DECSCA
        if self.extra.protection && self.screen.cursor.protected {
            // DEC protected mode
            write!(buf, "\x1b[1\"q")?;
        }

        // Emit Kitty keyboard protocol state using CSI = u
        if self.extra.kitty_keyboard {
            let current_flags = self.screen.kitty_keyboard.current();
            if current_flags.int() != ghostty_vt::kitty::key::Flags::DISABLED.int() {
                write!(buf, "\x1b[={};1u", current_flags.int())?;
            }
        }

        // Emit character set designations and invocations
        if self.extra.charsets {
            let charset = &self.screen.charset;

            // Emit G0-G3 designations
            for slot in [Slots::G0, Slots::G1, Slots::G2, Slots::G3] {
                let cs = charset.charsets.get(slot);
                if cs != Charset::Utf8 {
                    // Only emit non-default charsets
                    let intermediate = match slot {
                        Slots::G0 => '(',
                        Slots::G1 => ')',
                        Slots::G2 => '*',
                        Slots::G3 => '+',
                    };
                    let fin = match cs {
                        Charset::Ascii => 'B',
                        Charset::British => 'A',
                        Charset::DecSpecial => '0',
                        Charset::Utf8 => continue,
                    };
                    write!(buf, "\x1b{intermediate}{fin}")?;
                }
            }

            // Emit GL invocation if not G0
            if charset.gl != Slots::G0 {
                let seq: &[u8] = match charset.gl {
                    Slots::G0 => unreachable!(),
                    Slots::G1 => b"\x0e",  // SO - Shift Out
                    Slots::G2 => b"\x1bn", // LS2
                    Slots::G3 => b"\x1bo", // LS3
                };
                buf.extend_from_slice(seq);
            }

            // Emit GR invocation if not G2
            if charset.gr != Slots::G2 {
                let seq: &[u8] = match charset.gr {
                    Slots::G0 => unreachable!(), // GR can't be G0
                    Slots::G1 => b"\x1b~",       // LS1R
                    Slots::G2 => unreachable!(),
                    Slots::G3 => b"\x1b|", // LS3R
                };
                buf.extend_from_slice(seq);
            }
        }

        // Emit cursor position using CUP (CSI H)
        if self.extra.cursor {
            let cursor = &self.screen.cursor;
            // CUP is 1-indexed
            write!(buf, "\x1b[{};{}H", cursor.y + 1, cursor.x + 1)?;
        }

        writer.write_all(&buf)?;

        // If we have a pin_map, map all the extra bytes to the same pin.
        if let Some(m) = self.pin_map.as_deref_mut() {
            map_pad_last_or_top(m, &self.screen.pages, buf.len());
        }
        Ok(())
    }
}

/// PageList formatter formats multiple pages as represented by a PageList.
pub struct PageListFormatter<'a> {
    /// The pagelist to format.
    pub list: &'a PageList,

    /// The common options
    pub opts: Options,

    /// The bounds of the PageList to format. The top left and bottom right
    /// MUST be ordered properly.
    pub top_left: Option<Pin>,
    pub bottom_right: Option<Pin>,

    /// If true, the boundaries define a rectangle selection where start_x
    /// and end_x apply to every row, not just the first and last.
    pub rectangle: bool,

    /// If non-null, then `map` will contain the Pin of every byte
    /// byte written to the writer offset by the byte index.
    ///
    /// Warning: there is a significant performance hit to track this
    pub pin_map: Option<PinMap<'a>>,
}

impl<'a> PageListFormatter<'a> {
    pub fn new(list: &'a PageList, opts: Options) -> Self {
        PageListFormatter {
            list,
            opts,
            top_left: None,
            bottom_right: None,
            rectangle: false,
            pin_map: None,
        }
    }

    /// Format into a new `String`.
    pub fn format_string(self) -> Result<String> {
        let mut buf = Vec::new();
        self.format(&mut buf)?;
        String::from_utf8(buf).map_err(|_| invalid_cp())
    }

    pub fn format(mut self, writer: &mut dyn Write) -> Result<()> {
        let tl = self
            .top_left
            .unwrap_or_else(|| self.list.get_top_left(Tag::Screen).expect("top left"));
        let br = self.bottom_right.unwrap_or_else(|| {
            self.list
                .get_bottom_right(Tag::Screen)
                .expect("bottom right")
        });

        // If we keep track of pins, we'll need this.
        let mut point_map: Vec<Coordinate> = Vec::new();

        let mut page_state: Option<TrailingState> = None;
        for chunk in pin_page_iterator(self.list, tl, br) {
            assert!(chunk.start < chunk.end);
            assert!(chunk.end > 0);

            let node = self.list.node(chunk.node).expect("chunk node");
            let mut formatter = PageFormatter::new(&node.data, &self.opts);
            formatter.start_y = chunk.start;
            formatter.end_y = Some(chunk.end - 1);
            formatter.trailing_state = page_state;
            formatter.rectangle = self.rectangle;

            // For rectangle selection, apply start_x and end_x to all chunks
            if self.rectangle {
                formatter.start_x = tl.x;
                formatter.end_x = Some(br.x);
            } else {
                // Otherwise only on the first/last, respectively.
                if chunk.node == tl.node {
                    formatter.start_x = tl.x;
                }
                if chunk.node == br.node {
                    formatter.end_x = Some(br.x);
                }
            }

            // If we're tracking pins, then we setup a point map for the
            // page formatter (cause it can't track pins). And then we convert
            // this to pins later.
            let tracking = self.pin_map.is_some();
            if tracking {
                point_map.clear();
                formatter.point_map = Some(&mut point_map);
            }

            page_state = Some(formatter.format_with_state(writer)?);

            // If we're tracking pins then grab our points and write them
            // to our pin map.
            if let Some(m) = self.pin_map.as_deref_mut() {
                for coord in &point_map {
                    m.push(Pin {
                        node: chunk.node,
                        x: coord.x,
                        y: coord.y as size::CellCountInt,
                        garbage: false,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Trailing state. This is used to ensure that rows wrapped across
/// multiple pages are unwrapped properly, as well as other accounting
/// we may do in the future.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrailingState {
    pub rows: usize,
    pub cells: usize,
}

impl TrailingState {
    pub const EMPTY: TrailingState = TrailingState { rows: 0, cells: 0 };
}

/// Page formatter.
///
/// For styled formatting such as VT, this will emit references for palette
/// colors. If you want to capture the palette as-is at the type of formatting,
/// you'll have to emit the sequences for setting up the palette prior to
/// this formatting. (TODO: A function to do this)
pub struct PageFormatter<'a> {
    /// The page to format.
    pub page: &'a Page,

    /// The common options
    pub opts: &'a Options,

    /// Start and end points within the page to format. If end x is not given
    /// then it will be the full width. If end y is not given then it will be
    /// the full height.
    ///
    /// The start and end are both inclusive, so equal values will still
    /// return a non-empty result (i.e. a single cell or row).
    ///
    /// The start x is considered the X in the first row and end X is
    /// X in the final row. This isn't a rectangle selection by default.
    ///
    /// If start X falls on the second column of a wide character, then
    /// the entire character will be included (as if you specified the
    /// previous column).
    pub start_x: size::CellCountInt,
    pub start_y: size::CellCountInt,
    pub end_x: Option<size::CellCountInt>,
    pub end_y: Option<size::CellCountInt>,

    /// If true, the start x/y and end x/y define a rectangle selection.
    /// In this case, the boundaries will apply to every row, not just
    /// the first and last.
    pub rectangle: bool,

    /// If non-null, then `map` will contain the x/y coordinate of every
    /// byte written to the writer offset by the byte index.
    ///
    /// The x/y coordinate will be the coordinates within the page.
    ///
    /// Warning: there is a significant performance hit to track this
    pub point_map: Option<&'a mut Vec<Coordinate>>,

    /// The previous trailing state from the prior page. If you're iterating
    /// over multiple pages this helps ensure that unwrapping and other
    /// accounting works properly.
    pub trailing_state: Option<TrailingState>,
}

impl<'a> PageFormatter<'a> {
    /// Initializes a page formatter. Other options can be set directly on the
    /// struct after initialization and before calling `format()`.
    pub fn new(page: &'a Page, opts: &'a Options) -> Self {
        PageFormatter {
            page,
            opts,
            start_x: 0,
            start_y: 0,
            end_x: None,
            end_y: None,
            rectangle: false,
            point_map: None,
            trailing_state: None,
        }
    }

    /// Format into a new `String`.
    pub fn format_string(self) -> Result<String> {
        let mut buf = Vec::new();
        self.format(&mut buf)?;
        String::from_utf8(buf).map_err(|_| invalid_cp())
    }

    pub fn format(self, writer: &mut dyn Write) -> Result<()> {
        self.format_with_state(writer).map(|_| ())
    }

    fn map_push_n(&mut self, coord: Coordinate, n: usize) {
        if let Some(m) = self.point_map.as_deref_mut() {
            m.extend(std::iter::repeat_n(coord, n));
        }
    }

    pub fn format_with_state(mut self, writer: &mut dyn Write) -> Result<TrailingState> {
        let page = self.page;
        let opts = self.opts;
        let mut blank_rows: usize = 0;
        let mut blank_cells: usize = 0;

        // Continue our prior trailing state if we have it, but only if we're
        // starting from the beginning (start_y and start_x are both 0).
        // If a non-zero start position is specified, ignore trailing state.
        if let Some(state) = self.trailing_state
            && self.start_y == 0
            && self.start_x == 0
        {
            blank_rows = state.rows;
            blank_cells = state.cells;
        }
        let ret = |rows: usize, cells: usize| TrailingState { rows, cells };

        // Setup our starting column and perform some validation for overflows.
        // Note: start_x only applies to the first row, end_x only applies to the last row.
        let start_x = self.start_x;
        if start_x >= page.size.cols {
            return Ok(ret(blank_rows, blank_cells));
        }
        let end_x_unclamped = self.end_x.unwrap_or(page.size.cols - 1);
        let mut end_x = end_x_unclamped.min(page.size.cols - 1);

        // Setup our starting row and perform some validation for overflows.
        let start_y = self.start_y;
        if start_y >= page.size.rows {
            return Ok(ret(blank_rows, blank_cells));
        }
        let end_y_unclamped = self.end_y.unwrap_or(page.size.rows - 1);
        if start_y > end_y_unclamped {
            return Ok(ret(blank_rows, blank_cells));
        }
        let mut end_y = end_y_unclamped.min(page.size.rows - 1);

        // Edge case: if our end x/y falls on a spacer head AND we're unwrapping,
        // then we move the x/y to the start of the next row (if available).
        if opts.unwrap && !self.rectangle {
            let cells = page.get_cells(usize::from(end_y)).expect("row cells");
            if cells[usize::from(end_x)].wide() == Wide::SpacerHead {
                // Move to next row if available
                //
                // TODO: if unavailable, we should add to our trailing state
                //
                // so the pagelist formatter can be aware and maybe add
                // another page
                if end_y < page.size.rows - 1 {
                    end_y += 1;
                    end_x = 0;
                }
            }
        }

        // If we only have a single row, validate that start_x <= end_x
        if start_y == end_y && start_x > end_x {
            return Ok(ret(blank_rows, blank_cells));
        }

        // Wrap HTML output in monospace font styling
        match opts.emit {
            Format::Plain => {}

            Format::Html => {
                let mut header: Vec<u8> = Vec::new();

                // Monospace and whitespace preserving
                header.extend_from_slice(b"<div style=\"font-family: monospace; white-space: pre;");

                // Background/foreground colors
                if let Some(bg) = opts.background {
                    write!(header, "background-color: #{};", hex(bg))?;
                }
                if let Some(fg) = opts.foreground {
                    write!(header, "color: #{};", hex(fg))?;
                }

                header.extend_from_slice(b"\">");

                writer.write_all(&header)?;
                self.map_push_n(Coordinate { x: 0, y: 0 }, header.len());
            }

            Format::Vt => {
                // OSC 10 sets foreground color, OSC 11 sets background color
                let mut header: Vec<u8> = Vec::new();
                if let Some(fg) = opts.foreground {
                    write!(
                        header,
                        "\x1b]10;rgb:{:02x}/{:02x}/{:02x}\x1b\\",
                        fg.r, fg.g, fg.b
                    )?;
                }
                if let Some(bg) = opts.background {
                    write!(
                        header,
                        "\x1b]11;rgb:{:02x}/{:02x}/{:02x}\x1b\\",
                        bg.r, bg.g, bg.b
                    )?;
                }

                writer.write_all(&header)?;
                self.map_push_n(Coordinate { x: 0, y: 0 }, header.len());
            }
        }

        // Our style for non-plain formats
        let mut style = Style::default();

        // Track hyperlink state for HTML output. We need to close </a> tags
        // when the hyperlink changes or ends.
        let mut current_hyperlink_id: Option<u16> = None;

        let mut scratch: Vec<u8> = Vec::new();

        'rows: for y in start_y..=end_y {
            let row = *page.get_row(usize::from(y)).expect("row");
            let cells: &[Cell] = page.get_cells(usize::from(y)).expect("cells");

            // Determine the x range for this row
            // - First row: start_x to end of row (or end_x if single row)
            // - Last row: start of row to end_x
            // - Middle rows: full width
            let (cells_subset, row_start_x) = {
                // The end is always straightforward
                let row_end_x: size::CellCountInt = if self.rectangle || y == end_y {
                    end_x + 1
                } else {
                    page.size.cols
                };

                // The first we have to check if our start X falls on the
                // tail of a wide character.
                let row_start_x: size::CellCountInt =
                    if start_x > 0 && (self.rectangle || y == start_y) {
                        match cells[usize::from(start_x)].wide() {
                            // Include the prior cell to get the full wide char
                            Wide::SpacerTail => start_x - 1,

                            // If we're a spacer head on our first row then we
                            // skip this whole row.
                            Wide::SpacerHead => continue 'rows,

                            Wide::Narrow | Wide::Wide => start_x,
                        }
                    } else {
                        0
                    };

                (
                    &cells[usize::from(row_start_x)..usize::from(row_end_x)],
                    row_start_x,
                )
            };

            // If this row is blank, accumulate to avoid a bunch of extra
            // work later. If it isn't blank, make sure we dump all our
            // blanks.
            if !Cell::has_text_any(cells_subset) {
                blank_rows += 1;
                continue;
            }

            if blank_rows > 0 {
                // Reset style before emitting newlines to prevent background
                // colors from bleeding into the next line's leading cells.
                if !style.is_default() {
                    self.format_style_close(writer)?;
                    style = Style::default();
                }

                let sequence: &[u8] = match opts.emit {
                    // Plaintext just uses standard newlines because newlines
                    // on their own usually move the cursor back in anywhere
                    // you type plaintext.
                    Format::Plain => b"\n",

                    // VT uses \r\n because in a raw pty, \n alone doesn't
                    // guarantee moving the cursor back to column 0. \r
                    // makes it work for sure.
                    Format::Vt => b"\r\n",

                    // HTML uses just \n because HTML rendering will move
                    // the cursor back.
                    Format::Html => b"\n",
                };

                for _ in 0..blank_rows {
                    writer.write_all(sequence)?;
                }

                // \r and \n map to the row that ends with this newline.
                // If we're continuing (trailing state) then this will be
                // in a prior page, so we just map to the first row of this
                // page.
                if let Some(map) = self.point_map.as_deref_mut() {
                    let start = map.last().copied().unwrap_or(Coordinate { x: 0, y: 0 });

                    // The first one inherits the x value.
                    map.extend(std::iter::repeat_n(
                        Coordinate {
                            x: start.x,
                            y: start.y,
                        },
                        sequence.len(),
                    ));

                    // All others have x = 0 since they reference their prior
                    // blank line.
                    for y_offset in 1..blank_rows {
                        map.extend(std::iter::repeat_n(
                            Coordinate {
                                x: 0,
                                y: start.y + y_offset as u32,
                            },
                            sequence.len(),
                        ));
                    }
                }

                blank_rows = 0;
            }

            // If we're not wrapped, we always add a newline so after
            // the row is printed we can add a newline.
            if !row.wrap() || !opts.unwrap {
                blank_rows += 1;
            }

            // If the row doesn't continue a wrap then we need to reset
            // our blank cell count.
            if !row.wrap_continuation() || !opts.unwrap {
                blank_cells = 0;
            }

            // Go through each cell and print it
            'cells: for (i, cell) in cells_subset.iter().enumerate() {
                let x: size::CellCountInt = row_start_x + i as size::CellCountInt;

                // Skip spacers. These happen naturally when wide characters
                // are printed again on the screen (for well-behaved terminals!)
                match cell.wide() {
                    Wide::Narrow | Wide::Wide => {}
                    Wide::SpacerHead | Wide::SpacerTail => continue,
                }

                // If we have a zero value, then we accumulate a counter. We
                // only want to turn zero values into spaces if we have a non-zero
                // char sometime later.
                'blank: {
                    // If we're emitting styled output (not plaintext) and
                    // the cell has some kind of styling or is not empty
                    // then this isn't blank.
                    if opts.emit.styled() && (!cell.is_empty() || cell.has_styling()) {
                        break 'blank;
                    }

                    // Cells with no text are blank
                    if !cell.has_text() {
                        blank_cells += 1;
                        continue 'cells;
                    }

                    // Trailing spaces are blank. We know it is trailing
                    // because if we get a non-empty cell later we'll
                    // fill the blanks.
                    if cell.codepoint() == ' ' as u32 && opts.trim {
                        blank_cells += 1;
                        continue 'cells;
                    }
                }

                // This cell is not blank. If we have accumulated blank cells
                // then we want to emit them now.
                if blank_cells > 0 {
                    scratch.clear();
                    scratch.resize(blank_cells, b' ');
                    writer.write_all(&scratch)?;

                    if let Some(map) = self.point_map.as_deref_mut() {
                        // Map each blank cell to its coordinate. Blank cells can span
                        // multiple rows if they carry over from wrap continuation.
                        let mut remaining_blanks = blank_cells;
                        let mut blank_x = x;
                        let mut blank_y = y;
                        while remaining_blanks > 0 {
                            if blank_x > 0 {
                                // We have space in this row
                                blank_x -= 1;
                            } else if blank_y > 0 {
                                // Wrap to previous row
                                blank_y -= 1;
                                blank_x = page.size.cols - 1;
                            } else {
                                // Can't go back further, just use (0, 0)
                                blank_x = 0;
                                blank_y = 0;
                            }

                            map.push(Coordinate {
                                x: blank_x,
                                y: u32::from(blank_y),
                            });
                            remaining_blanks -= 1;
                        }
                    }

                    blank_cells = 0;
                }

                'style: {
                    // If we aren't emitting styled output then we don't
                    // have to worry about styles.
                    if !opts.emit.styled() {
                        break 'style;
                    }

                    // Get our cell style.
                    let cell_style = self.cell_style(*cell);

                    // If the style hasn't changed, don't bloat output.
                    if cell_style.eql(style) {
                        break 'style;
                    }

                    // If we had a previous style, we need to close it,
                    // because we've confirmed we have some new style
                    // (which is maybe default).
                    if !style.is_default() {
                        match opts.emit {
                            Format::Html => self.format_style_close(writer)?,

                            // For VT, we only close if we're switching to a default
                            // style because any non-default style will emit
                            // a \x1b[0m as the start of a VT coloring sequence.
                            Format::Vt => {
                                if cell_style.is_default() {
                                    self.format_style_close(writer)?;
                                }
                            }

                            // Unreachable because of the styled() check at the
                            // top of this block.
                            Format::Plain => unreachable!(),
                        }
                    }

                    // At this point, we can copy our style over
                    style = cell_style;

                    // If we're just the default style now, we're done.
                    if cell_style.is_default() {
                        break 'style;
                    }

                    // New style, emit it.
                    scratch.clear();
                    self.format_style_open(&mut scratch, &style)?;
                    writer.write_all(&scratch)?;

                    // If we have a point map, we map the style to
                    // this cell.
                    self.map_push_n(Coordinate { x, y: u32::from(y) }, scratch.len());
                }

                // Hyperlink state
                'hyperlink: {
                    // We currently only emit hyperlinks for HTML. In the
                    // future we can support emitting OSC 8 hyperlinks for
                    // VT output as well.
                    if opts.emit != Format::Html {
                        break 'hyperlink;
                    }

                    // Get the hyperlink ID. This ID is our internal ID,
                    // not necessarily the OSC8 ID.
                    let link_id_: Option<u16> = if cell.hyperlink() {
                        page.cell_index(usize::from(x), usize::from(y))
                            .ok()
                            .and_then(|i| page.lookup_hyperlink(i))
                    } else {
                        None
                    };

                    // If our hyperlink IDs match (even null) then we have
                    // identical hyperlink state and we do nothing.
                    if current_hyperlink_id == link_id_ {
                        break 'hyperlink;
                    }

                    // If our prior hyperlink ID was non-null, we need to
                    // close it because the ID has changed.
                    if current_hyperlink_id.is_some() {
                        self.format_hyperlink_close(writer)?;
                        current_hyperlink_id = None;
                    }

                    // Set our current hyperlink ID
                    let Some(link_id) = link_id_ else {
                        break 'hyperlink;
                    };
                    current_hyperlink_id = Some(link_id);

                    // Emit the opening hyperlink tag
                    let uri: Vec<u8> = page
                        .hyperlink_set
                        .get(link_id)
                        .map(|l| l.uri.clone())
                        .unwrap_or_default();
                    scratch.clear();
                    self.format_hyperlink_open(&mut scratch, &uri)?;
                    writer.write_all(&scratch)?;

                    // If we have a point map, we map the hyperlink to
                    // this cell.
                    self.map_push_n(Coordinate { x, y: u32::from(y) }, scratch.len());
                }

                match cell.content_tag() {
                    // We combine codepoint and graphemes because both have
                    // shared style handling.
                    ContentTag::Codepoint | ContentTag::CodepointGrapheme => {
                        scratch.clear();
                        self.write_cell(&mut scratch, *cell, x, y)?;
                        writer.write_all(&scratch)?;

                        // If we have a point map, all codepoints map to this
                        // cell.
                        self.map_push_n(Coordinate { x, y: u32::from(y) }, scratch.len());
                    }

                    // Cells with only background color (no text). Emit a space
                    // with the appropriate background color SGR sequence.
                    ContentTag::BgColorPalette | ContentTag::BgColorRgb => {
                        writer.write_all(b" ")?;
                        self.map_push_n(Coordinate { x, y: u32::from(y) }, 1);
                    }
                }
            }
        }

        // If the style is non-default, we need to close our style tag.
        if !style.is_default() {
            self.format_style_close(writer)?;
        }

        // Close any open hyperlink for HTML output
        if current_hyperlink_id.is_some() {
            self.format_hyperlink_close(writer)?;
        }

        // Close the monospace wrapper for HTML output
        if opts.emit == Format::Html {
            let closing = b"</div>";
            writer.write_all(closing)?;
            if let Some(map) = self.point_map.as_deref_mut() {
                let last = map.last().copied().unwrap_or(Coordinate { x: 0, y: 0 });
                map.extend(std::iter::repeat_n(last, closing.len()));
            }
        }

        Ok(ret(blank_rows, blank_cells))
    }

    fn write_cell(
        &self,
        out: &mut Vec<u8>,
        cell: Cell,
        x: size::CellCountInt,
        y: size::CellCountInt,
    ) -> Result<()> {
        // Blank cells get an empty space that isn't replaced by anything
        // because it isn't really a space. We do this so that formatting
        // is preserved if we're emitting styles.
        if !cell.has_text() {
            out.push(b' ');
            return Ok(());
        }

        self.write_codepoint_with_replacement(out, cell.codepoint())?;
        if cell.content_tag() == ContentTag::CodepointGrapheme {
            let idx = self
                .page
                .cell_index(usize::from(x), usize::from(y))
                .map_err(|_| invalid_cp())?;
            for &cp in self.page.lookup_grapheme(idx).expect("grapheme") {
                self.write_codepoint_with_replacement(out, cp)?;
            }
        }
        Ok(())
    }

    fn write_codepoint_with_replacement(&self, out: &mut Vec<u8>, codepoint: u32) -> Result<()> {
        // Search for our replacement
        let r: Option<&Replacement> = self.opts.codepoint_map.as_ref().and_then(|map| {
            map.iter()
                .rev()
                .find(|m| m.range[0] <= codepoint && codepoint <= m.range[1])
                .map(|m| &m.replacement)
        });

        // If no replacement, write it directly.
        let Some(r) = r else {
            return self.write_codepoint(out, codepoint);
        };

        match r {
            Replacement::Codepoint(v) => self.write_codepoint(out, *v),
            Replacement::String(s) => {
                for c in s.chars() {
                    self.write_codepoint(out, c as u32)?;
                }
                Ok(())
            }
        }
    }

    fn write_codepoint(&self, out: &mut Vec<u8>, codepoint: u32) -> Result<()> {
        let push = |out: &mut Vec<u8>, cp: u32| -> Result<()> {
            let c = char::from_u32(cp).ok_or_else(invalid_cp)?;
            let mut b = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            Ok(())
        };
        match self.opts.emit {
            Format::Plain | Format::Vt => push(out, codepoint),
            Format::Html => match codepoint {
                0x3C => {
                    out.extend_from_slice(b"&lt;");
                    Ok(())
                }
                0x3E => {
                    out.extend_from_slice(b"&gt;");
                    Ok(())
                }
                0x26 => {
                    out.extend_from_slice(b"&amp;");
                    Ok(())
                }
                0x22 => {
                    out.extend_from_slice(b"&quot;");
                    Ok(())
                }
                0x27 => {
                    out.extend_from_slice(b"&#39;");
                    Ok(())
                }
                _ => {
                    // For HTML, emit ASCII (< 0x80) directly, but encode
                    // all non-ASCII as numeric entities to avoid encoding
                    // detection issues (fixes #9426). We can't set the
                    // meta tag because we emit partial HTML so this ensures
                    // proper unicode handling.
                    if codepoint < 0x80 {
                        push(out, codepoint)
                    } else {
                        write!(out, "&#{codepoint};")
                    }
                }
            },
        }
    }

    /// Returns the style for the given cell. If there is no styling this
    /// will return the default style.
    fn cell_style(&self, cell: Cell) -> Style {
        match cell.content_tag() {
            ContentTag::Codepoint | ContentTag::CodepointGrapheme => {
                if cell.has_styling() {
                    self.page.style(cell)
                } else {
                    Style::default()
                }
            }

            ContentTag::BgColorPalette => Style {
                bg_color: Color::Palette(cell.color_palette()),
                ..Style::default()
            },

            ContentTag::BgColorRgb => Style {
                bg_color: Color::Rgb(cell.color_rgb()),
                ..Style::default()
            },
        }
    }

    fn format_style_open(&self, out: &mut Vec<u8>, style: &Style) -> Result<()> {
        match self.opts.emit {
            Format::Plain => unreachable!(),

            Format::Vt => {
                let mut formatter = style.formatter_vt();
                formatter.palette = self.opts.palette.as_ref();
                write!(out, "{formatter}")
            }

            // We use `display: inline` so that the div doesn't impact
            // layout since we're primarily using it as a CSS wrapper.
            Format::Html => {
                let mut formatter = style.formatter_html();
                formatter.palette = self.opts.palette.as_ref();
                write!(out, "<div style=\"display: inline;{formatter}\">")
            }
        }
    }

    fn write_close_str(&mut self, writer: &mut dyn Write, s: &[u8]) -> Result<()> {
        writer.write_all(s)?;
        if let Some(m) = self.point_map.as_deref_mut() {
            debug_assert!(m.capacity() > 0);
            let last = m.last().copied().unwrap_or(Coordinate { x: 0, y: 0 });
            m.extend(std::iter::repeat_n(last, s.len()));
        }
        Ok(())
    }

    fn format_style_close(&mut self, writer: &mut dyn Write) -> Result<()> {
        let s: &[u8] = match self.opts.emit {
            Format::Plain => return Ok(()),
            Format::Vt => b"\x1b[0m",
            Format::Html => b"</div>",
        };
        self.write_close_str(writer, s)
    }

    fn format_hyperlink_open(&self, out: &mut Vec<u8>, uri: &[u8]) -> Result<()> {
        match self.opts.emit {
            Format::Plain | Format::Vt => unreachable!(),

            Format::Html => {
                out.extend_from_slice(b"<a href=\"");
                for &byte in uri {
                    self.write_codepoint(out, u32::from(byte))?;
                }
                out.extend_from_slice(b"\">");
                Ok(())
            }
        }
    }

    fn format_hyperlink_close(&mut self, writer: &mut dyn Write) -> Result<()> {
        let s: &[u8] = match self.opts.emit {
            Format::Html => b"</a>",
            Format::Plain | Format::Vt => return Ok(()),
        };
        self.write_close_str(writer, s)
    }
}

#[cfg(test)]
mod tests;
