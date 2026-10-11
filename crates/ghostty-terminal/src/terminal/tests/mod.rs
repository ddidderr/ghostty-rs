//! Tests ported from `Terminal.zig`, split into chunks (`p1`..`p8`) in source
//! order. Shared helpers live here.
#![allow(dead_code, unused_imports)]
pub(super) use super::*;
pub(super) use crate::point::{Coordinate, Point};

mod p1;
mod p2;
mod p3;
mod p4;
mod p5;
mod p6;
mod p7;
mod p8;

/// `Terminal.init(alloc, .{ .cols, .rows })` (max_scrollback keeps the default).
pub(super) fn init(cols: u16, rows: u16) -> Terminal {
    Terminal::new(Options::new(cols, rows)).unwrap()
}

/// Like `init` but with an explicit max_scrollback (bytes).
pub(super) fn init_scrollback(cols: u16, rows: u16, max_scrollback: usize) -> Terminal {
    let mut o = Options::new(cols, rows);
    o.max_scrollback = max_scrollback;
    Terminal::new(o).unwrap()
}

pub(super) fn pt_active(x: u16, y: u32) -> Point {
    Point::Active(Coordinate { x, y })
}
pub(super) fn pt_screen(x: u16, y: u32) -> Point {
    Point::Screen(Coordinate { x, y })
}
pub(super) fn pt_viewport(x: u16, y: u32) -> Point {
    Point::Viewport(Coordinate { x, y })
}
pub(super) fn pt_history(x: u16, y: u32) -> Point {
    Point::History(Coordinate { x, y })
}

/// Zig `try t.printString(str)`.
pub(super) fn print_str(t: &mut Terminal, s: &str) {
    t.print_string(s).unwrap();
}

/// Feed raw bytes through a fresh read-only VT stream (Zig
/// `var s = t.vtStream(); try s.nextSlice(...)`). Parser state does not
/// persist across calls; create a stream with `t.vt_stream()` if it must.
pub(super) fn feed(t: &mut Terminal, bytes: &[u8]) {
    let mut s = t.vt_stream();
    s.next_slice(bytes).unwrap();
}

/// Zig `t.plainString(alloc)`.
pub(super) fn plain(t: &Terminal) -> String {
    t.plain_string().unwrap()
}

/// Zig `t.plainStringUnwrapped(alloc)`.
pub(super) fn plain_unwrapped(t: &Terminal) -> String {
    t.plain_string_unwrapped().unwrap()
}

/// Zig `t.screens.active.pages.getCell(pt).?` (a copy of the cell).
pub(super) fn cell_at(t: &Terminal, pt: Point) -> crate::page::Cell {
    t.screen().pages.get_cell(pt).unwrap()
}

/// Zig `t.screens.active.dumpStringAlloc(alloc, pt)`.
pub(super) fn dump(t: &Terminal, pt: Point) -> String {
    t.screen().dump_string_alloc(pt).unwrap()
}

/// Zig `t.screens.active.pages.pin(pt).?`.
pub(super) fn pin_at(t: &Terminal, pt: Point) -> crate::page_list::Pin {
    t.screen().pages.pin(pt).unwrap()
}

/// Zig `pin.rowAndCell().row` for a point (copy of the row).
pub(super) fn row_at(t: &Terminal, pt: Point) -> crate::page::Row {
    *pin_at(t, pt).row_and_cell(&t.screen().pages).unwrap().0
}

/// Zig `pin.node.data.lookupGrapheme(cell)` for a point: extra codepoints.
pub(super) fn graphemes_at(t: &Terminal, pt: Point) -> Option<Vec<u32>> {
    let p = pin_at(t, pt);
    let page = t.screen().page(p.node);
    let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
    page.lookup_grapheme(i).map(<[u32]>::to_vec)
}
