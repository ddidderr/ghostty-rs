//! Shared helpers for search tests.

use crate::{
    page_list::{NodeId, PageList},
    point::{Coordinate, Point, Tag},
    screen::{Options, Screen},
    terminal::{self, Terminal},
};

pub fn screen(cols: u16, rows: u16, max_scrollback: usize) -> Screen {
    Screen::new(Options::new(cols, rows, max_scrollback)).unwrap()
}

pub fn terminal(cols: u16, rows: u16) -> Terminal {
    Terminal::new(terminal::Options::new(cols, rows)).unwrap()
}

pub fn terminal_scrollback(cols: u16, rows: u16, max_scrollback: usize) -> Terminal {
    let mut o = terminal::Options::new(cols, rows);
    o.max_scrollback = max_scrollback;
    Terminal::new(o).unwrap()
}

pub fn feed(t: &mut Terminal, bytes: &[u8]) {
    let mut s = t.vt_stream();
    s.next_slice(bytes).unwrap();
}

/// Point constructor with a tag.
pub fn pt(tag: Tag, x: u16, y: u32) -> Point {
    Point::new(tag, Coordinate { x, y })
}

pub fn active(x: u16, y: u32) -> Point {
    pt(Tag::Active, x, y)
}

pub fn first(list: &PageList) -> NodeId {
    list.first.unwrap()
}

pub fn last(list: &PageList) -> NodeId {
    list.last.unwrap()
}

pub fn next(list: &PageList, id: NodeId) -> NodeId {
    list.node(id).unwrap().next.unwrap()
}

pub fn screen_pt(x: u16, y: u32) -> Point {
    pt(Tag::Screen, x, y)
}
