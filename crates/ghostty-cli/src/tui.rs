//! Shared TUI helpers (ZIG/src/cli/tui.zig) plus a tiny re-implementation of
//! the part of libvaxis that Ghostty's "pretty" (non-interactive, tty)
//! listings use: composing a single row of styled cells in a window and
//! writing it with vaxis' `prettyPrint` escape sequences.
//!
//! The byte output was verified against Ghostty 1.3.1 in a pty.

use std::io::Write;

use unicode_width::UnicodeWidthChar;

/// Whether pretty printing is possible on this platform
/// (`tui.can_pretty_print`; false only on iOS/tvOS/watchOS in Zig).
pub const CAN_PRETTY_PRINT: bool = true;

/// `vaxis.ctlseqs.unicode_set`.
pub const UNICODE_SET: &str = "\x1b[?2027h";
/// `vaxis.ctlseqs.unicode_reset`.
pub const UNICODE_RESET: &str = "\x1b[?2027l";

/// A cell color (`vaxis.Color`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Index(u8),
    Rgb(u8, u8, u8),
}

/// A cell style (the subset of `vaxis.Style` that is used).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cell {
    /// Never written (`vaxis.Cell{}`): rendered by moving the cursor.
    Unwritten,
    /// The trailing half of a wide grapheme.
    Continuation(Style),
    Glyph(String, Style),
}

/// The result of printing a segment (`vaxis.Window.PrintResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrintResult {
    pub col: u16,
    pub row: u16,
    pub overflow: bool,
}

/// A vaxis window of `width` x `height` cells. The listings only ever use
/// row 0 explicitly, but text that does not fit wraps onto the following
/// rows (and is rendered there), like vaxis does.
#[derive(Debug, Clone)]
pub struct Row {
    width: u16,
    height: u16,
    rows: Vec<Vec<Cell>>,
}

/// Display width of a string (`Window.gwidth` with unicode mode on).
#[must_use]
pub fn gwidth(s: &str) -> u16 {
    let w: usize = s.chars().map(|c| c.width().unwrap_or(0)).sum();
    u16::try_from(w).unwrap_or(u16::MAX)
}

impl Row {
    #[must_use]
    pub fn new(width: u16) -> Row {
        Row::with_height(width, window_size().1)
    }

    #[must_use]
    pub fn with_height(width: u16, height: u16) -> Row {
        Row {
            width,
            height: height.max(1),
            rows: vec![vec![Cell::Unwritten; usize::from(width)]],
        }
    }

    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// `Window.clear`.
    pub fn clear(&mut self) {
        self.rows = vec![vec![Cell::Unwritten; usize::from(self.width)]];
    }

    fn cell_mut(&mut self, row: u16, col: u16) -> &mut Cell {
        while self.rows.len() <= usize::from(row) {
            self.rows
                .push(vec![Cell::Unwritten; usize::from(self.width)]);
        }
        &mut self.rows[usize::from(row)][usize::from(col)]
    }

    /// `Window.printSegment` starting on row 0 at `col_offset`; wraps onto
    /// the next rows when the text does not fit.
    pub fn print_segment(&mut self, text: &str, style: Style, col_offset: u16) -> PrintResult {
        self.print_segment_at(text, style, 0, col_offset)
    }

    /// `Window.printSegment` with an explicit `row_offset`.
    pub fn print_segment_at(
        &mut self,
        text: &str,
        style: Style,
        row_offset: u16,
        col_offset: u16,
    ) -> PrintResult {
        let mut col = col_offset;
        let mut row = row_offset;
        let mut overflow = false;
        let mut last: Option<(u16, u16)> = None;
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if w == 0 {
                // Combine with the previous glyph.
                if let Some((r, c)) = last {
                    if let Cell::Glyph(g, _) = self.cell_mut(r, c) {
                        g.push(ch);
                    }
                }
                continue;
            }
            let w16 = u16::try_from(w).unwrap_or(1);
            if col + w16 > self.width {
                row += 1;
                col = 0;
            }
            if row >= self.height || w16 > self.width {
                overflow = true;
                break;
            }
            *self.cell_mut(row, col) = Cell::Glyph(ch.to_string(), style);
            for i in 1..w16 {
                *self.cell_mut(row, col + i) = Cell::Continuation(style);
            }
            last = Some((row, col));
            col += w16;
            if col >= self.width {
                row += 1;
                col = 0;
            }
        }
        PrintResult { col, row, overflow }
    }

    /// `Window.print`: print several segments one after another.
    pub fn print(&mut self, segments: &[(&str, Style)], col_offset: u16) -> PrintResult {
        let mut res = PrintResult {
            col: col_offset,
            row: 0,
            overflow: false,
        };
        for (text, style) in segments {
            res = self.print_segment_at(text, *style, res.row, res.col);
            if res.overflow {
                break;
            }
        }
        res
    }

    /// Write this window like `vx.prettyPrint` does (the escape sequences
    /// are identical to libvaxis': synchronized update, cursor hidden, only
    /// the written part of each row, unwritten gaps skipped with CUF).
    ///
    /// `first` must be true for the first row printed by a program (vaxis
    /// then also returns to column 0 and erases below).
    pub fn pretty_print(&self, w: &mut dyn Write, first: bool) -> std::io::Result<()> {
        if first {
            w.write_all(b"\r\x1b[m\x1b[J")?;
        }
        w.write_all(b"\x1b[?25l\x1b[?2026h\x1b[m")?;

        // Rows after the last written row are not printed.
        let used_rows = self
            .rows
            .iter()
            .rposition(|r| r.iter().any(|c| !matches!(c, Cell::Unwritten)))
            .map_or(1, |i| i + 1);

        let mut cursor = Style::default();
        for (ri, cells) in self.rows[..used_rows].iter().enumerate() {
            if ri > 0 {
                w.write_all(b"\n\r")?;
            }
            // Trailing unwritten cells are never emitted.
            let end = cells
                .iter()
                .rposition(|c| !matches!(c, Cell::Unwritten))
                .map_or(0, |i| i + 1);

            let mut skip = 0usize;
            for cell in &cells[..end] {
                match cell {
                    Cell::Unwritten => skip += 1,
                    Cell::Continuation(_) => {}
                    Cell::Glyph(g, style) => {
                        if skip > 0 {
                            write!(w, "\x1b[{skip}C")?;
                            skip = 0;
                        }
                        write_style_change(w, &cursor, style)?;
                        cursor = *style;
                        w.write_all(g.as_bytes())?;
                    }
                }
            }
            if skip > 0 {
                write!(w, "\x1b[{skip}C")?;
            }
        }

        w.write_all(b"\r\n\x1b[m\x1b[?2026l")
    }
}

fn write_color(w: &mut dyn Write, c: Color, base: u8) -> std::io::Result<()> {
    // base is 3 for foreground, 4 for background.
    match c {
        Color::Default => write!(w, "\x1b[{base}9m"),
        Color::Index(i) if i < 8 => write!(w, "\x1b[{base}{i}m"),
        Color::Index(i) if i < 16 => write!(w, "\x1b[{}{}m", base + 6, i - 8),
        Color::Index(i) => write!(w, "\x1b[{base}8:5:{i}m"),
        Color::Rgb(r, g, b) => write!(w, "\x1b[{base}8:2:{r}:{g}:{b}m"),
    }
}

/// Emit the SGR changes to go from `cursor` to `next` in vaxis' order.
fn write_style_change(w: &mut dyn Write, cursor: &Style, next: &Style) -> std::io::Result<()> {
    if cursor.fg != next.fg {
        write_color(w, next.fg, 3)?;
    }
    if cursor.bg != next.bg {
        write_color(w, next.bg, 4)?;
    }
    if cursor.bold != next.bold {
        w.write_all(if next.bold { b"\x1b[1m" } else { b"\x1b[22m" })?;
        if next.dim {
            w.write_all(b"\x1b[2m")?;
        }
    }
    if cursor.dim != next.dim {
        w.write_all(if next.dim { b"\x1b[2m" } else { b"\x1b[22m" })?;
        if next.bold {
            w.write_all(b"\x1b[1m")?;
        }
    }
    if cursor.italic != next.italic {
        w.write_all(if next.italic { b"\x1b[3m" } else { b"\x1b[23m" })?;
    }
    Ok(())
}

/// The terminal size in columns and rows (`vaxis.Tty.getWinsize`).
#[must_use]
pub fn window_size() -> (u16, u16) {
    ratatui::crossterm::terminal::size().unwrap_or((80, 24))
}

/// Sequences vaxis writes when it is torn down (`vx.deinit`).
pub const VAXIS_DEINIT: &str = "\x1b[?25h\x1b[m\r\x1b[J";

/// A pretty-printing session on a tty, mirroring the
/// `vaxis.Tty.init` + `vaxis.init` + mode-2027 setup used by the listing
/// actions. Dropping it restores the terminal.
pub struct PrettySession<W: Write> {
    out: W,
    first: bool,
    raw: bool,
    pub width: u16,
    pub height: u16,
}

impl<W: Write> PrettySession<W> {
    pub fn new(mut out: W) -> std::io::Result<Self> {
        let (width, height) = window_size();
        // We know we are ghostty, so enable mode 2027.
        out.write_all(UNICODE_SET.as_bytes())?;
        Ok(PrettySession {
            out,
            first: true,
            raw: false,
            width,
            height,
        })
    }

    /// Like [`PrettySession::new`] but also puts the tty in raw mode, like
    /// `vaxis.Tty.init` does (this matters for the bytes that reach the
    /// terminal: with output processing off, `\r\n` stays `\r\n`).
    pub fn new_raw(out: W) -> std::io::Result<Self> {
        ratatui::crossterm::terminal::enable_raw_mode()?;
        let mut s = Self::new(out)?;
        s.raw = true;
        Ok(s)
    }

    /// A new empty row of the window width.
    #[must_use]
    pub fn row(&self) -> Row {
        Row::with_height(self.width, self.height)
    }

    /// Write a composed row.
    pub fn print_row(&mut self, row: &Row) -> std::io::Result<()> {
        row.pretty_print(&mut self.out, self.first)?;
        self.first = false;
        Ok(())
    }

    /// Restore the terminal state (also done on drop).
    pub fn finish(&mut self) -> std::io::Result<()> {
        let result = (|| {
            self.out.write_all(UNICODE_RESET.as_bytes())?;
            self.out.write_all(VAXIS_DEINIT.as_bytes())?;
            self.out.flush()
        })();
        if self.raw {
            self.raw = false;
            ratatui::crossterm::terminal::disable_raw_mode()?;
        }
        result
    }
}

impl<W: Write> Drop for PrettySession<W> {
    fn drop(&mut self) {
        if self.raw {
            let _ = ratatui::crossterm::terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn render(row: &Row, first: bool) -> String {
        let mut v = Vec::new();
        row.pretty_print(&mut v, first).unwrap();
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn plain_text_and_skip() {
        let mut row = Row::with_height(20, 5);
        let r = row.print_segment("abc", Style::default(), 0);
        assert_eq!(3, r.col);
        row.print_segment("x", Style::default(), 7);
        assert_eq!(
            "\x1b[?25l\x1b[?2026h\x1b[mabc\x1b[4Cx\r\n\x1b[m\x1b[?2026l",
            render(&row, false)
        );
    }

    #[test]
    fn first_row_prefix() {
        let mut row = Row::with_height(5, 5);
        row.print_segment("a", Style::default(), 0);
        assert!(render(&row, true).starts_with("\r\x1b[m\x1b[J\x1b[?25l\x1b[?2026h\x1b[ma"));
    }

    #[test]
    fn styles_match_vaxis_order() {
        let mut row = Row::with_height(40, 5);
        let red = Style {
            fg: Color::Index(1),
            ..Style::default()
        };
        let dim = Style {
            dim: true,
            ..Style::default()
        };
        let magenta = Style {
            fg: Color::Index(5),
            ..Style::default()
        };
        let rgb = Style {
            fg: Color::Rgb(1, 2, 3),
            bg: Color::Rgb(4, 5, 6),
            ..Style::default()
        };
        let res = row.print(&[("su", red), (":", dim), ("x", magenta), ("c", rgb)], 0);
        assert_eq!(5, res.col);
        assert_eq!(
            "\x1b[?25l\x1b[?2026h\x1b[m\x1b[31msu\x1b[39m\x1b[2m:\x1b[35m\x1b[22mx\x1b[38:2:1:2:3m\x1b[48:2:4:5:6mc\r\n\x1b[m\x1b[?2026l",
            render(&row, false)
        );
    }

    #[test]
    fn wrapping_onto_following_rows() {
        let mut row = Row::with_height(5, 3);
        let r = row.print_segment("abcdefg", Style::default(), 2);
        assert_eq!((4, 1), (r.col, r.row));
        assert_eq!(
            "\x1b[?25l\x1b[?2026h\x1b[m\x1b[2Cabc\n\rdefg\r\n\x1b[m\x1b[?2026l",
            render(&row, false)
        );
        let mut row = Row::with_height(5, 1);
        assert!(row.print_segment("abcdefg", Style::default(), 0).overflow);
    }

    #[test]
    fn wide_chars_and_overflow() {
        let mut row = Row::with_height(5, 5);
        let r = row.print_segment("a\u{4e2d}b", Style::default(), 0);
        assert_eq!(4, r.col);
        assert_eq!(2, gwidth("\u{4e2d}"));
        let r = row.print_segment("zz", Style::default(), 3);
        assert!(!r.overflow);
    }
}
