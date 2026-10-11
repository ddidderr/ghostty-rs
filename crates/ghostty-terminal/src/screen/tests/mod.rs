//! Ported Screen.zig tests. Shared helpers live here; the tests themselves
//! are split over `p*.rs` files in the order they appear in Screen.zig.
#![allow(
    clippy::manual_assert_eq,
    clippy::unwrap_used,
    clippy::too_many_lines,
    dead_code
)]

pub(super) use super::*;
pub(super) use crate::point::Coordinate;

mod p1;
mod p2;
mod p3;
mod p4;
mod p5;

/// `Screen.init(alloc, .{ .cols, .rows, .max_scrollback })`
pub(super) fn init(cols: u16, rows: u16, max_scrollback: usize) -> Screen {
    Screen::new(Options::new(cols, rows, max_scrollback)).unwrap()
}

/// Point constructors with the Zig `.{ .tag = .{ .x, .y } }` shape.
pub(super) fn active(x: u16, y: u32) -> Point {
    Point::Active(Coordinate { x, y })
}
pub(super) fn screen_pt(x: u16, y: u32) -> Point {
    Point::Screen(Coordinate { x, y })
}
pub(super) fn history(x: u16, y: u32) -> Point {
    Point::History(Coordinate { x, y })
}
pub(super) fn viewport(x: u16, y: u32) -> Point {
    Point::Viewport(Coordinate { x, y })
}

/// `s.dumpStringAlloc(alloc, tl)`
pub(super) fn dump(s: &Screen, tl: Point) -> String {
    s.dump_string_alloc(tl).unwrap()
}

/// `s.dumpStringAllocUnwrapped(alloc, tl)`
pub(super) fn dump_unwrapped(s: &Screen, tl: Point) -> String {
    s.dump_string_alloc_unwrapped(tl).unwrap()
}

/// `s.pages.pin(pt).?`
pub(super) fn pin(s: &Screen, pt: Point) -> Pin {
    s.pages.pin(pt).unwrap()
}

/// `s.pages.pointFromPin(tag, pin).?`
pub(super) fn pt_from_pin(s: &Screen, tag: Tag, p: Pin) -> Point {
    s.pages.point_from_pin(tag, p).unwrap()
}

/// The page at the cursor.
pub(super) fn cursor_page(s: &Screen) -> &Page {
    s.cursor_page()
}
