//! Represents a single selection within the terminal (i.e. a highlight region).
//!
//! Unlike Zig, tracked bounds hold [`TrackedPin`] handles into the owning
//! [`PageList`], so every method that must read (or move) the pins takes the
//! screen's page list explicitly (`&screen.pages`).
#![allow(clippy::manual_assert_eq)]
use crate::{
    page::{Cell, Error},
    page_list::{Direction, PageList, Pin, TrackedPin},
    pin_iter,
    point::{Coordinate, Point, Tag},
};

// NOTE(mitchellh): I'm not very happy with how this is implemented, because
// the ordering operations which are used frequently require using
// pointFromPin which -- at the time of writing this -- is slow. The overall
// style of this struct is due to porting it from the previous implementation
// which had an efficient ordering operation.
//
// While reimplementing this, there were too many callers that already
// depended on this behavior so I kept it despite the inefficiency. In the
// future, we should take a look at this again!

/// The bounds of the selection. A selection bounds can be either tracked
/// or untracked. Untracked bounds are unsafe beyond the point the terminal
/// screen may be modified, since they may point to invalid memory. Tracked
/// bounds are always valid and will be updated as the screen changes, but
/// are more expensive to exist.
///
/// In all cases, start and end can be in any order. There is no guarantee that
/// start is before end or vice versa. If a user selects backwards,
/// start will be after end, and vice versa. Use the struct functions
/// to not have to worry about this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bounds {
    Untracked { start: Pin, end: Pin },
    Tracked { start: TrackedPin, end: TrackedPin },
}

/// Represents a single selection within the terminal (i.e. a highlight region).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// The bounds of the selection.
    pub bounds: Bounds,

    /// Whether or not this selection refers to a rectangle, rather than whole
    /// lines of a buffer. In this mode, start and end refer to the top left and
    /// bottom right of the rectangle, or vice versa if the selection is backwards.
    pub rectangle: bool,
}

/// The order of the selection:
///
///  * forward: start(x, y) is before end(x, y) (top-left to bottom-right).
///  * reverse: end(x, y) is before start(x, y) (bottom-right to top-left).
///  * mirrored_[forward|reverse]: special, rectangle selections only (see below).
///
///  For regular selections, the above also holds for top-right to bottom-left
///  (forward) and bottom-left to top-right (reverse). However, for rectangle
///  selections, both of these selections are *mirrored* as orientation
///  operations only flip the x or y axis, not both. Depending on the y axis
///  direction, this is either mirrored_forward or mirrored_reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Forward,
    Reverse,
    MirroredForward,
    MirroredReverse,
}

/// Possible adjustments to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjustment {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    BeginningOfLine,
    EndOfLine,
}

fn screen_coord(list: &PageList, p: Pin) -> Coordinate {
    list.point_from_pin(Tag::Screen, p)
        .expect("selection pin must be in the screen")
        .coord()
}

impl Selection {
    /// Initialize a new (untracked) selection with the given start and end
    /// pins on the screen.
    pub fn new(start_pin: Pin, end_pin: Pin, rect: bool) -> Self {
        Self {
            bounds: Bounds::Untracked {
                start: start_pin,
                end: end_pin,
            },
            rectangle: rect,
        }
    }

    /// Release the tracked pins (if any).
    pub fn deinit(self, list: &mut PageList) {
        if let Bounds::Tracked { start, end } = self.bounds {
            list.untrack_pin(start);
            list.untrack_pin(end);
        }
    }

    /// Returns true if this selection is equal to another selection.
    pub fn eql(&self, list: &PageList, other: &Selection) -> bool {
        self.start(list).eql(other.start(list))
            && self.end(list).eql(other.end(list))
            && self.rectangle == other.rectangle
    }

    /// The starting pin of the selection. This is NOT ordered.
    pub fn start(&self, list: &PageList) -> Pin {
        match self.bounds {
            Bounds::Untracked { start, .. } => start,
            Bounds::Tracked { start, .. } => list
                .tracked_pin(start)
                .expect("tracked selection pin must be live"),
        }
    }

    /// The ending pin of the selection. This is NOT ordered.
    pub fn end(&self, list: &PageList) -> Pin {
        match self.bounds {
            Bounds::Untracked { end, .. } => end,
            Bounds::Tracked { end, .. } => list
                .tracked_pin(end)
                .expect("tracked selection pin must be live"),
        }
    }

    /// Set the starting pin (Zig: writing through `startPtr`).
    pub fn set_start(&mut self, list: &mut PageList, pin: Pin) {
        match &mut self.bounds {
            Bounds::Untracked { start, .. } => *start = pin,
            Bounds::Tracked { start, .. } => {
                list.set_tracked_pin(*start, pin)
                    .expect("selection pin must be valid");
            }
        }
    }

    /// Set the ending pin (Zig: writing through `endPtr`).
    pub fn set_end(&mut self, list: &mut PageList, pin: Pin) {
        match &mut self.bounds {
            Bounds::Untracked { end, .. } => *end = pin,
            Bounds::Tracked { end, .. } => {
                list.set_tracked_pin(*end, pin)
                    .expect("selection pin must be valid");
            }
        }
    }

    /// Returns true if this is a tracked selection.
    pub fn tracked(&self) -> bool {
        matches!(self.bounds, Bounds::Tracked { .. })
    }

    /// Convert this selection a tracked selection. It is asserted this is
    /// an untracked selection. The tracked selection is returned.
    pub fn track(&self, list: &mut PageList) -> Result<Selection, Error> {
        let Bounds::Untracked { start, end } = self.bounds else {
            panic!("selection is already tracked");
        };

        // Track our pins
        let tracked_start = list.track_pin(start)?;
        let tracked_end = match list.track_pin(end) {
            Ok(v) => v,
            Err(e) => {
                list.untrack_pin(tracked_start);
                return Err(e);
            }
        };

        Ok(Selection {
            bounds: Bounds::Tracked {
                start: tracked_start,
                end: tracked_end,
            },
            rectangle: self.rectangle,
        })
    }

    /// Returns the top left point of the selection.
    pub fn top_left(&self, list: &PageList) -> Pin {
        match self.order(list) {
            Order::Forward => self.start(list),
            Order::Reverse => self.end(list),
            Order::MirroredForward => {
                let mut p = self.start(list);
                p.x = self.end(list).x;
                p
            }
            Order::MirroredReverse => {
                let mut p = self.end(list);
                p.x = self.start(list).x;
                p
            }
        }
    }

    /// Returns the bottom right point of the selection.
    pub fn bottom_right(&self, list: &PageList) -> Pin {
        match self.order(list) {
            Order::Forward => self.end(list),
            Order::Reverse => self.start(list),
            Order::MirroredForward => {
                let mut p = self.end(list);
                p.x = self.start(list).x;
                p
            }
            Order::MirroredReverse => {
                let mut p = self.start(list);
                p.x = self.end(list).x;
                p
            }
        }
    }

    pub fn order(&self, list: &PageList) -> Order {
        let start_pt = screen_coord(list, self.start(list));
        let end_pt = screen_coord(list, self.end(list));

        if self.rectangle {
            // Reverse (also handles single-column)
            if start_pt.y > end_pt.y && start_pt.x >= end_pt.x {
                return Order::Reverse;
            }
            if start_pt.y >= end_pt.y && start_pt.x > end_pt.x {
                return Order::Reverse;
            }

            // Mirror, bottom-left to top-right
            if start_pt.y > end_pt.y && start_pt.x < end_pt.x {
                return Order::MirroredReverse;
            }

            // Mirror, top-right to bottom-left
            if start_pt.y < end_pt.y && start_pt.x > end_pt.x {
                return Order::MirroredForward;
            }

            // Forward
            return Order::Forward;
        }

        if start_pt.y < end_pt.y {
            return Order::Forward;
        }
        if start_pt.y > end_pt.y {
            return Order::Reverse;
        }
        if start_pt.x <= end_pt.x {
            return Order::Forward;
        }
        Order::Reverse
    }

    /// Returns the selection in the given order.
    ///
    /// The returned selection is always a new untracked selection.
    ///
    /// Note that only forward and reverse are useful desired orders for this
    /// function. All other orders act as if forward order was desired.
    pub fn ordered(&self, list: &PageList, desired: Order) -> Selection {
        if self.order(list) == desired {
            return Selection::new(self.start(list), self.end(list), self.rectangle);
        }

        let tl = self.top_left(list);
        let br = self.bottom_right(list);
        match desired {
            Order::Reverse => Selection::new(br, tl, self.rectangle),
            _ => Selection::new(tl, br, self.rectangle),
        }
    }

    /// Returns true if the selection contains the given point.
    ///
    /// This recalculates top left and bottom right each call. If you have
    /// many points to check, it is cheaper to do the containment logic
    /// yourself and cache the topleft/bottomright.
    pub fn contains(&self, list: &PageList, pin: Pin) -> bool {
        let tl_pin = self.top_left(list);
        let br_pin = self.bottom_right(list);

        // This is definitely not very efficient. Low-hanging fruit to
        // improve this.
        let tl = screen_coord(list, tl_pin);
        let br = screen_coord(list, br_pin);
        let p = screen_coord(list, pin);

        // If we're in rectangle select, we can short-circuit with an easy check
        // here
        if self.rectangle {
            return p.y >= tl.y && p.y <= br.y && p.x >= tl.x && p.x <= br.x;
        }

        // If tl/br are same line
        if tl.y == br.y {
            return p.y == tl.y && p.x >= tl.x && p.x <= br.x;
        }

        // If on top line, just has to be left of X
        if p.y == tl.y {
            return p.x >= tl.x;
        }

        // If on bottom line, just has to be right of X
        if p.y == br.y {
            return p.x <= br.x;
        }

        // If between the top/bottom, always good.
        p.y > tl.y && p.y < br.y
    }

    /// Get a selection for a single row in the screen. This will return None
    /// if the row is not included in the selection.
    ///
    /// This is a very expensive operation. It has to traverse the linked list
    /// of pages for the top-left, bottom-right, and the given pin to find
    /// the coordinates. If you are calling this repeatedly, prefer
    /// `contained_row_cached`.
    pub fn contained_row(&self, list: &PageList, pin: Pin) -> Option<Selection> {
        let tl_pin = self.top_left(list);
        let br_pin = self.bottom_right(list);

        // This is definitely not very efficient. Low-hanging fruit to
        // improve this. Callers should prefer contained_row_cached if they
        // can swing it.
        let tl = screen_coord(list, tl_pin);
        let br = screen_coord(list, br_pin);
        let p = screen_coord(list, pin);

        self.contained_row_cached(list, tl_pin, br_pin, pin, tl, br, p)
    }

    /// Same as `contained_row` but useful if you're calling it repeatedly
    /// so that the pins can be cached across calls. Advanced.
    #[allow(clippy::too_many_arguments)]
    pub fn contained_row_cached(
        &self,
        list: &PageList,
        tl_pin: Pin,
        br_pin: Pin,
        pin: Pin,
        tl: Coordinate,
        br: Coordinate,
        p: Coordinate,
    ) -> Option<Selection> {
        if p.y < tl.y || p.y > br.y {
            return None;
        }

        // Rectangle case: we can return early as the x range will always be the
        // same. We've already validated that the row is in the selection.
        if self.rectangle {
            return Some(Selection::new(
                Pin { x: tl.x, ..pin },
                Pin { x: br.x, ..pin },
                true,
            ));
        }

        if p.y == tl.y {
            // If the selection is JUST this line, return it as-is.
            if p.y == br.y {
                return Some(Selection::new(tl_pin, br_pin, false));
            }

            // Selection top-left line matches only.
            return Some(Selection::new(
                tl_pin,
                Pin {
                    x: list.cols - 1,
                    ..pin
                },
                false,
            ));
        }

        // Row is our bottom selection, so we return the selection from the
        // beginning of the line to the br. We know our selection is more than
        // one line (due to conditionals above)
        if p.y == br.y {
            assert!(p.y != tl.y);
            return Some(Selection::new(Pin { x: 0, ..pin }, br_pin, false));
        }

        // Row is somewhere between our selection lines so we return the full line.
        Some(Selection::new(
            Pin { x: 0, ..pin },
            Pin {
                x: list.cols - 1,
                ..pin
            },
            false,
        ))
    }

    /// Adjust the selection by some given adjustment. An adjustment allows
    /// a selection to be expanded slightly left, right, up, down, etc.
    pub fn adjust(&mut self, list: &mut PageList, adjustment: Adjustment) {
        // Note that we always adjust "end" because end always represents
        // the last point of the selection by mouse, not necessarily the
        // top/bottom visually. So this results in the correct behavior
        // whether the user drags up or down.
        let end_pin = self.end(list);
        match adjustment {
            Adjustment::Up => {
                if let Some(new_end) = end_pin.up(list, 1) {
                    self.set_end(list, new_end);
                } else {
                    self.adjust(list, Adjustment::BeginningOfLine);
                }
            }

            Adjustment::Down => {
                // Find the next non-blank row
                let mut current = end_pin;
                loop {
                    let Some(next) = current.down(list, 1) else {
                        // If we're at the bottom, just go to the end of the line
                        self.adjust(list, Adjustment::EndOfLine);
                        break;
                    };
                    let has_text = list
                        .node(next.node)
                        .and_then(|n| n.data.get_cells(usize::from(next.y)))
                        .is_some_and(Cell::has_text_any);
                    if has_text {
                        self.set_end(list, next);
                        break;
                    }
                    current = next;
                }
            }

            Adjustment::Left => {
                let found = pin_iter::cell_iterator(list, end_pin, Direction::LeftUp, None)
                    .skip(1)
                    .find(|p| cell_has_text(list, *p));
                if let Some(next) = found {
                    self.set_end(list, next);
                }
            }

            Adjustment::Right => {
                // Step right, wrapping to the next row down at the start of each new line,
                // until we find a non-empty cell.
                let found = pin_iter::cell_iterator(list, end_pin, Direction::RightDown, None)
                    .skip(1)
                    .find(|p| cell_has_text(list, *p));
                if let Some(next) = found {
                    self.set_end(list, next);
                }
            }

            Adjustment::PageUp => {
                if let Some(new_end) = end_pin.up(list, usize::from(list.rows)) {
                    self.set_end(list, new_end);
                } else {
                    self.adjust(list, Adjustment::Home);
                }
            }

            Adjustment::PageDown => {
                if let Some(new_end) = end_pin.down(list, usize::from(list.rows)) {
                    self.set_end(list, new_end);
                } else {
                    self.adjust(list, Adjustment::End);
                }
            }

            Adjustment::Home => {
                let p = list
                    .pin(Point::Screen(Coordinate { x: 0, y: 0 }))
                    .expect("screen has a top-left");
                self.set_end(list, p);
            }

            Adjustment::End => {
                let found = list
                    .row_iterator(
                        Direction::LeftUp,
                        Point::Screen(Coordinate::default()),
                        None,
                    )
                    .find_map(|next| {
                        let cells = list.node(next.node)?.data.get_cells(usize::from(next.y))?;
                        if Cell::has_text_any(cells) {
                            Some(Pin {
                                x: (cells.len() - 1) as u16,
                                ..next
                            })
                        } else {
                            None
                        }
                    });
                if let Some(p) = found {
                    self.set_end(list, p);
                }
            }

            Adjustment::BeginningOfLine => {
                self.set_end(list, Pin { x: 0, ..end_pin });
            }

            Adjustment::EndOfLine => {
                let cols = list
                    .node(end_pin.node)
                    .map_or(list.cols, |n| n.data.size.cols);
                self.set_end(
                    list,
                    Pin {
                        x: cols - 1,
                        ..end_pin
                    },
                );
            }
        }
    }
}

fn cell_has_text(list: &PageList, p: Pin) -> bool {
    p.row_and_cell(list).is_some_and(|(_, c)| c.has_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{Options, Screen};

    fn init(cols: u16, rows: u16, max_scrollback: usize) -> Screen {
        Screen::new(Options::new(cols, rows, max_scrollback)).unwrap()
    }

    /// `s.pages.pin(.{ .screen = .{ .x, .y } }).?`
    fn sp(s: &Screen, x: u16, y: u32) -> Pin {
        s.pages.pin(Point::Screen(Coordinate { x, y })).unwrap()
    }

    /// `s.pages.pointFromPin(.screen, pin).?`
    fn spt(s: &Screen, p: Pin) -> Point {
        s.pages.point_from_pin(Tag::Screen, p).unwrap()
    }

    fn pt(x: u16, y: u32) -> Point {
        Point::Screen(Coordinate { x, y })
    }

    fn sel_of(s: &Screen, a: (u16, u32), b: (u16, u32), rect: bool) -> Selection {
        Selection::new(sp(s, a.0, a.1), sp(s, b.0, b.1), rect)
    }

    /// Adjust a fresh selection and check the resulting start/end points.
    fn check_adjust(
        s: &mut Screen,
        start: (u16, u32),
        end: (u16, u32),
        adj: Adjustment,
        want_start: (u16, u32),
        want_end: (u16, u32),
    ) {
        let mut sel = sel_of(s, start, end, false);
        sel.adjust(&mut s.pages, adj);
        assert_eq!(pt(want_start.0, want_start.1), spt(s, sel.start(&s.pages)));
        assert_eq!(pt(want_end.0, want_end.1), spt(s, sel.end(&s.pages)));
        sel.deinit(&mut s.pages);
    }

    #[test]
    fn selection_adjust_right() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A1234\nB5678\nC1234\nD5678").unwrap();

        // Simple movement right
        check_adjust(&mut s, (5, 1), (3, 3), Adjustment::Right, (5, 1), (4, 3));

        // Already at end of the line.
        check_adjust(&mut s, (4, 1), (4, 2), Adjustment::Right, (4, 1), (0, 3));

        // Already at end of the screen
        check_adjust(&mut s, (5, 1), (4, 3), Adjustment::Right, (5, 1), (4, 3));
    }

    #[test]
    fn selection_adjust_left() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A1234\nB5678\nC1234\nD5678").unwrap();

        // Simple movement left
        check_adjust(&mut s, (5, 1), (3, 3), Adjustment::Left, (5, 1), (2, 3));

        // Already at beginning of the line.
        check_adjust(&mut s, (5, 1), (0, 3), Adjustment::Left, (5, 1), (4, 2));
    }

    #[test]
    fn selection_adjust_left_skips_blanks() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A1234\nB5678\nC12\nD56").unwrap();

        // Same line
        check_adjust(&mut s, (5, 1), (4, 3), Adjustment::Left, (5, 1), (2, 3));

        // Edge
        check_adjust(&mut s, (5, 1), (0, 3), Adjustment::Left, (5, 1), (2, 2));
    }

    #[test]
    fn selection_adjust_up() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A\nB\nC\nD\nE").unwrap();

        // Not on the first line
        check_adjust(&mut s, (5, 1), (3, 3), Adjustment::Up, (5, 1), (3, 2));

        // On the first line
        check_adjust(&mut s, (5, 1), (3, 0), Adjustment::Up, (5, 1), (0, 0));
    }

    #[test]
    fn selection_adjust_down() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A\nB\nC\nD\nE").unwrap();

        // Not on the first line
        check_adjust(&mut s, (5, 1), (3, 3), Adjustment::Down, (5, 1), (3, 4));

        // On the last line
        check_adjust(&mut s, (4, 1), (3, 4), Adjustment::Down, (4, 1), (9, 4));
    }

    #[test]
    fn selection_adjust_down_with_not_full_screen() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A\nB\nC").unwrap();

        // On the last line
        check_adjust(&mut s, (4, 1), (3, 2), Adjustment::Down, (4, 1), (9, 2));
    }

    #[test]
    fn selection_adjust_home() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A\nB\nC").unwrap();

        // On the last line
        check_adjust(&mut s, (4, 1), (1, 2), Adjustment::Home, (4, 1), (0, 0));
    }

    #[test]
    fn selection_adjust_end_with_not_full_screen() {
        let mut s = init(10, 10, 0);
        s.test_write_string("A\nB\nC").unwrap();

        // On the last line
        check_adjust(&mut s, (4, 0), (1, 1), Adjustment::End, (4, 0), (9, 2));
    }

    #[test]
    fn selection_adjust_beginning_of_line() {
        let mut s = init(8, 10, 0);
        s.test_write_string("A12 B34\nC12 D34").unwrap();

        // Not at beginning of the line
        check_adjust(
            &mut s,
            (5, 1),
            (5, 1),
            Adjustment::BeginningOfLine,
            (5, 1),
            (0, 1),
        );

        // Already at beginning of the line
        check_adjust(
            &mut s,
            (5, 1),
            (0, 1),
            Adjustment::BeginningOfLine,
            (5, 1),
            (0, 1),
        );

        // End pin moves to start pin
        check_adjust(
            &mut s,
            (0, 1),
            (5, 1),
            Adjustment::BeginningOfLine,
            (0, 1),
            (0, 1),
        );
    }

    #[test]
    fn selection_adjust_end_of_line() {
        let mut s = init(8, 10, 0);
        s.test_write_string("A12 B34\nC12 D34").unwrap();

        // Not at end of the line
        check_adjust(
            &mut s,
            (1, 0),
            (1, 0),
            Adjustment::EndOfLine,
            (1, 0),
            (7, 0),
        );

        // Already at end of the line
        check_adjust(
            &mut s,
            (1, 0),
            (7, 0),
            Adjustment::EndOfLine,
            (1, 0),
            (7, 0),
        );

        // End pin moves to start pin
        check_adjust(
            &mut s,
            (7, 0),
            (1, 0),
            Adjustment::EndOfLine,
            (7, 0),
            (7, 0),
        );
    }

    #[test]
    fn selection_order_standard() {
        let s = init(100, 100, 1);

        // forward, multi-line
        assert_eq!(
            Order::Forward,
            sel_of(&s, (2, 1), (2, 2), false).order(&s.pages)
        );
        // reverse, multi-line
        assert_eq!(
            Order::Reverse,
            sel_of(&s, (2, 2), (2, 1), false).order(&s.pages)
        );
        // forward, same-line
        assert_eq!(
            Order::Forward,
            sel_of(&s, (2, 1), (3, 1), false).order(&s.pages)
        );
        // forward, single char
        assert_eq!(
            Order::Forward,
            sel_of(&s, (2, 1), (2, 1), false).order(&s.pages)
        );
        // reverse, single line
        assert_eq!(
            Order::Reverse,
            sel_of(&s, (2, 1), (1, 1), false).order(&s.pages)
        );
    }

    #[test]
    fn selection_order_rectangle() {
        let s = init(100, 100, 1);

        // Conventions:
        // TL - top left
        // BL - bottom left
        // TR - top right
        // BR - bottom right

        // forward (TL -> BR)
        assert_eq!(
            Order::Forward,
            sel_of(&s, (1, 1), (2, 2), true).order(&s.pages)
        );
        // reverse (BR -> TL)
        assert_eq!(
            Order::Reverse,
            sel_of(&s, (2, 2), (1, 1), true).order(&s.pages)
        );
        // mirrored_forward (TR -> BL)
        assert_eq!(
            Order::MirroredForward,
            sel_of(&s, (3, 1), (1, 3), true).order(&s.pages)
        );
        // mirrored_reverse (BL -> TR)
        assert_eq!(
            Order::MirroredReverse,
            sel_of(&s, (1, 3), (3, 1), true).order(&s.pages)
        );
        // forward, single line (left -> right )
        assert_eq!(
            Order::Forward,
            sel_of(&s, (1, 1), (3, 1), true).order(&s.pages)
        );
        // reverse, single line (right -> left)
        assert_eq!(
            Order::Reverse,
            sel_of(&s, (3, 1), (1, 1), true).order(&s.pages)
        );
        // forward, single column (top -> bottom)
        assert_eq!(
            Order::Forward,
            sel_of(&s, (2, 1), (2, 3), true).order(&s.pages)
        );
        // reverse, single column (bottom -> top)
        assert_eq!(
            Order::Reverse,
            sel_of(&s, (2, 3), (2, 1), true).order(&s.pages)
        );
        // forward, single cell
        assert_eq!(
            Order::Forward,
            sel_of(&s, (1, 1), (1, 1), true).order(&s.pages)
        );
    }

    #[test]
    fn top_left() {
        let s = init(10, 10, 0);
        // forward
        let sel = sel_of(&s, (1, 1), (3, 1), true);
        assert_eq!(pt(1, 1), spt(&s, sel.top_left(&s.pages)));
        // reverse
        let sel = sel_of(&s, (3, 1), (1, 1), true);
        assert_eq!(pt(1, 1), spt(&s, sel.top_left(&s.pages)));
        // mirrored_forward
        let sel = sel_of(&s, (3, 1), (1, 3), true);
        assert_eq!(pt(1, 1), spt(&s, sel.top_left(&s.pages)));
        // mirrored_reverse
        let sel = sel_of(&s, (1, 3), (3, 1), true);
        assert_eq!(pt(1, 1), spt(&s, sel.top_left(&s.pages)));
    }

    #[test]
    fn bottom_right() {
        let s = init(10, 10, 0);
        // forward
        let sel = sel_of(&s, (1, 1), (3, 1), false);
        assert_eq!(pt(3, 1), spt(&s, sel.bottom_right(&s.pages)));
        // reverse
        let sel = sel_of(&s, (3, 1), (1, 1), false);
        assert_eq!(pt(3, 1), spt(&s, sel.bottom_right(&s.pages)));
        // mirrored_forward
        let sel = sel_of(&s, (3, 1), (1, 3), true);
        assert_eq!(pt(3, 3), spt(&s, sel.bottom_right(&s.pages)));
        // mirrored_reverse
        let sel = sel_of(&s, (1, 3), (3, 1), true);
        assert_eq!(pt(3, 3), spt(&s, sel.bottom_right(&s.pages)));
    }

    #[test]
    fn ordered() {
        let s = init(10, 10, 0);
        let l = &s.pages;
        {
            // forward
            let sel = sel_of(&s, (1, 1), (3, 1), false);
            let sel_reverse = sel_of(&s, (3, 1), (1, 1), false);
            assert!(sel.ordered(l, Order::Forward).eql(l, &sel));
            assert!(sel.ordered(l, Order::Reverse).eql(l, &sel_reverse));
            assert!(sel.ordered(l, Order::MirroredForward).eql(l, &sel));
        }
        {
            // reverse
            let sel = sel_of(&s, (3, 1), (1, 1), false);
            let sel_forward = sel_of(&s, (1, 1), (3, 1), false);
            assert!(sel.ordered(l, Order::Forward).eql(l, &sel_forward));
            assert!(sel.ordered(l, Order::Reverse).eql(l, &sel));
            assert!(sel.ordered(l, Order::MirroredForward).eql(l, &sel_forward));
        }
        {
            // mirrored_forward
            let sel = sel_of(&s, (3, 1), (1, 3), true);
            let sel_forward = sel_of(&s, (1, 1), (3, 3), true);
            let sel_reverse = sel_of(&s, (3, 3), (1, 1), true);
            assert!(sel.ordered(l, Order::Forward).eql(l, &sel_forward));
            assert!(sel.ordered(l, Order::Reverse).eql(l, &sel_reverse));
            assert!(sel.ordered(l, Order::MirroredReverse).eql(l, &sel_forward));
        }
        {
            // mirrored_reverse
            let sel = sel_of(&s, (1, 3), (3, 1), true);
            let sel_forward = sel_of(&s, (1, 1), (3, 3), true);
            let sel_reverse = sel_of(&s, (3, 3), (1, 1), true);
            assert!(sel.ordered(l, Order::Forward).eql(l, &sel_forward));
            assert!(sel.ordered(l, Order::Reverse).eql(l, &sel_reverse));
            assert!(sel.ordered(l, Order::MirroredForward).eql(l, &sel_forward));
        }
    }

    #[test]
    fn selection_contains() {
        let s = init(10, 10, 0);
        let l = &s.pages;
        {
            let sel = sel_of(&s, (5, 1), (3, 2), false);

            assert!(sel.contains(l, sp(&s, 6, 1)));
            assert!(sel.contains(l, sp(&s, 1, 2)));
            assert!(!sel.contains(l, sp(&s, 1, 1)));
            assert!(!sel.contains(l, sp(&s, 5, 2)));
        }

        // Reverse
        {
            let sel = sel_of(&s, (3, 2), (5, 1), false);

            assert!(sel.contains(l, sp(&s, 6, 1)));
            assert!(sel.contains(l, sp(&s, 1, 2)));
            assert!(!sel.contains(l, sp(&s, 1, 1)));
            assert!(!sel.contains(l, sp(&s, 5, 2)));
        }

        // Single line
        {
            let sel = sel_of(&s, (5, 1), (8, 1), false);

            assert!(sel.contains(l, sp(&s, 6, 1)));
            assert!(!sel.contains(l, sp(&s, 2, 1)));
            assert!(!sel.contains(l, sp(&s, 9, 1)));
        }
    }

    #[test]
    fn selection_contains_rectangle() {
        let s = init(15, 15, 0);
        let l = &s.pages;
        for (a, b) in [((3, 3), (7, 9)), ((7, 9), (3, 3))] {
            // The second iteration is the "Reverse" case.
            let sel = sel_of(&s, a, b, true);

            assert!(sel.contains(l, sp(&s, 5, 6))); // Center
            assert!(sel.contains(l, sp(&s, 3, 6))); // Left border
            assert!(sel.contains(l, sp(&s, 7, 6))); // Right border
            assert!(sel.contains(l, sp(&s, 5, 3))); // Top border
            assert!(sel.contains(l, sp(&s, 5, 9))); // Bottom border

            assert!(!sel.contains(l, sp(&s, 5, 2))); // Above center
            assert!(!sel.contains(l, sp(&s, 5, 10))); // Below center
            assert!(!sel.contains(l, sp(&s, 2, 6))); // Left center
            assert!(!sel.contains(l, sp(&s, 8, 6))); // Right center
            assert!(!sel.contains(l, sp(&s, 8, 3))); // Just right of top right
            assert!(!sel.contains(l, sp(&s, 2, 9))); // Just left of bottom left
        }

        // Single line
        // NOTE: This is the same as normal selection but we just do it for brevity
        {
            let sel = sel_of(&s, (5, 1), (10, 1), true);

            assert!(sel.contains(l, sp(&s, 6, 1)));
            assert!(!sel.contains(l, sp(&s, 2, 1)));
            assert!(!sel.contains(l, sp(&s, 12, 1)));
        }
    }

    #[test]
    fn selection_contained_row() {
        let s = init(10, 5, 0);
        let l = &s.pages;
        let last = s.pages.cols - 1;

        {
            let sel = sel_of(&s, (5, 1), (3, 3), false);

            // Not contained
            assert!(sel.contained_row(l, sp(&s, 1, 4)).is_none());

            // Start line
            assert_eq!(
                Selection::new(sel.start(l), sp(&s, last, 1), false),
                sel.contained_row(l, sp(&s, 1, 1)).unwrap()
            );

            // End line
            assert_eq!(
                Selection::new(sp(&s, 0, 3), sel.end(l), false),
                sel.contained_row(l, sp(&s, 2, 3)).unwrap()
            );

            // Middle line
            assert_eq!(
                Selection::new(sp(&s, 0, 2), sp(&s, last, 2), false),
                sel.contained_row(l, sp(&s, 2, 2)).unwrap()
            );
        }

        // Rectangle
        {
            let sel = sel_of(&s, (3, 1), (6, 3), true);

            // Not contained
            assert!(sel.contained_row(l, sp(&s, 1, 4)).is_none());

            // Start line
            assert_eq!(
                Selection::new(sp(&s, 3, 1), sp(&s, 6, 1), true),
                sel.contained_row(l, sp(&s, 1, 1)).unwrap()
            );

            // End line
            assert_eq!(
                Selection::new(sp(&s, 3, 3), sp(&s, 6, 3), true),
                sel.contained_row(l, sp(&s, 2, 3)).unwrap()
            );

            // Middle line
            assert_eq!(
                Selection::new(sp(&s, 3, 2), sp(&s, 6, 2), true),
                sel.contained_row(l, sp(&s, 2, 2)).unwrap()
            );
        }

        // Single-line selection
        {
            let sel = sel_of(&s, (2, 1), (6, 1), false);

            // Not contained
            assert!(sel.contained_row(l, sp(&s, 1, 0)).is_none());
            assert!(sel.contained_row(l, sp(&s, 1, 2)).is_none());

            // Contained
            assert_eq!(sel, sel.contained_row(l, sp(&s, 1, 1)).unwrap());
        }
    }
}
