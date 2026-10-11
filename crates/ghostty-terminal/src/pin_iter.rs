//! Pin-based iterators.
//!
//! Zig's `PageList.Pin` has `rowIterator`, `cellIterator` and
//! `promptIterator` methods that start from a pin and use an inclusive limit
//! *pin* (the Rust `PageList` iterators are point-based). These are local
//! equivalents used by `Screen` and `Selection`.
//!
//! TODO(integration): replace with `Pin` methods in `page_list.rs` once
//! available (see docs/rewrite/notes/ghostty-terminal-pagelist-requests.md).
use crate::{
    page_list::{Direction, PageList, Pin, PromptIterator},
    point::{Coordinate, Point, Tag},
};

fn same_row(a: Pin, b: Pin) -> bool {
    a.node == b.node && a.y == b.y
}

/// Iterates the rows (x = 0) from `start` toward `limit` (inclusive).
pub struct RowIter<'a> {
    list: &'a PageList,
    current: Option<Pin>,
    limit: Option<Pin>,
    direction: Direction,
}

impl Iterator for RowIter<'_> {
    type Item = Pin;

    fn next(&mut self) -> Option<Pin> {
        let cur = self.current?;
        let result = Pin { x: 0, ..cur };
        if self.limit.is_some_and(|l| same_row(l, cur)) {
            self.current = None;
        } else {
            self.current = match self.direction {
                Direction::RightDown => cur.down(self.list, 1),
                Direction::LeftUp => cur.up(self.list, 1),
            };
        }
        Some(result)
    }
}

/// Equivalent of `Pin.rowIterator(direction, limit)`. The limit is inclusive.
pub fn row_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> RowIter<'_> {
    RowIter {
        list,
        current: Some(start),
        limit,
        direction,
    }
}

/// Iterates cells from `start` toward the end of the `limit` row.
///
/// Note that, as in Zig, the limit only applies at row granularity: iteration
/// continues to the end (or start, for `LeftUp`) of the limit row.
pub struct CellIter<'a> {
    list: &'a PageList,
    cell: Option<Pin>,
    limit: Option<Pin>,
    direction: Direction,
}

impl Iterator for CellIter<'_> {
    type Item = Pin;

    fn next(&mut self) -> Option<Pin> {
        let cell = self.cell?;
        let cols = self
            .list
            .node(cell.node)
            .map_or(self.list.cols, |n| n.data.size.cols);
        let at_limit = self.limit.is_some_and(|l| same_row(l, cell));
        self.cell = match self.direction {
            Direction::RightDown => {
                if cell.x + 1 < cols {
                    Some(Pin {
                        x: cell.x + 1,
                        ..cell
                    })
                } else if at_limit {
                    None
                } else {
                    cell.down(self.list, 1).map(|p| Pin { x: 0, ..p })
                }
            }
            Direction::LeftUp => {
                if cell.x > 0 {
                    Some(Pin {
                        x: cell.x - 1,
                        ..cell
                    })
                } else if at_limit {
                    None
                } else {
                    cell.up(self.list, 1).map(|p| {
                        let cols = self.list.node(p.node).map_or(cols, |n| n.data.size.cols);
                        Pin { x: cols - 1, ..p }
                    })
                }
            }
        };
        Some(cell)
    }
}

/// Equivalent of `Pin.cellIterator(direction, limit)`.
pub fn cell_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> CellIter<'_> {
    CellIter {
        list,
        cell: Some(start),
        limit,
        direction,
    }
}

/// Equivalent of `Pin.promptIterator(direction, limit)`.
pub fn prompt_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> PromptIterator<'_> {
    let pt = |p: Pin| {
        list.point_from_pin(Tag::Screen, p)
            .unwrap_or(Point::Screen(Coordinate::default()))
    };
    match direction {
        Direction::RightDown => list.prompt_iterator(direction, pt(start), limit.map(pt)),
        Direction::LeftUp => list.prompt_iterator(
            direction,
            limit.map_or(Point::Screen(Coordinate::default()), pt),
            Some(pt(start)),
        ),
    }
}
