//! Ported formatter.zig tests, split by Zig source line range:
//! p1 = "Page plain ..." (Zig lines 1580-3287), p2 = "Page VT ...", "PageList ..."
//! and "TerminalFormatter ..." (3288-4458), p3 = "Screen ...", "Terminal vt ...",
//! "Page html ..." up to "Page VT style reset properly closes styles"
//! (4459-5617), p4 = "Page codepoint_map ..." onwards (5618-end).
#![allow(dead_code, clippy::needless_range_loop)]

use super::*;
use crate::{
    page::Page,
    page_list::Pin,
    point::{Coordinate, Point},
    terminal::{Options as TermOptions, Terminal},
};

mod p1;
mod p2;
mod p3;
mod p4;

/// Zig `Terminal.init(alloc, .{ .cols, .rows })`.
pub(super) fn term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(TermOptions::new(cols, rows)).unwrap()
}

/// Zig `var s = t.vtStream(); try s.nextSlice(bytes)` (fresh stream per call;
/// use `t.vt_stream()` directly if parser state must persist).
pub(super) fn feed(t: &mut Terminal, bytes: &[u8]) {
    let mut s = t.vt_stream();
    s.next_slice(bytes).unwrap();
}

/// Zig `&pages.pages.last.?.data`.
pub(super) fn last_page(t: &Terminal) -> &Page {
    let pages = &t.screen().pages;
    &pages.node(pages.last.unwrap()).unwrap().data
}

/// Zig `&pages.pages.first.?.data`.
pub(super) fn first_page(t: &Terminal) -> &Page {
    let pages = &t.screen().pages;
    &pages.node(pages.first.unwrap()).unwrap().data
}

/// Zig `pages.pages.first.?.data.capacity.rows`.
pub(super) fn first_page_rows(t: &Terminal) -> u16 {
    first_page(t).capacity.rows
}

/// Format a whole `PageFormatter` into a string, returning the output.
pub(super) fn fmt_str(f: PageFormatter<'_>) -> String {
    f.format_string().unwrap()
}
