//! Searches for a substring within the viewport of a PageList.
//!
//! This contains logic to efficiently detect when the viewport changes
//! and only re-search when necessary.
//!
//! The specialization for "viewport" is because the viewport is the
//! only part of the search where the user can actively see the results,
//! usually. In that case, it is more efficient to re-search only the
//! viewport rather than store all the results for the entire screen.
//!
//! Note that this searches all the pages that viewport covers, so
//! this can include extra matches outside the viewport if the data
//! lives in the same page.

use super::sliding_window::{Direction, SlidingWindow};
use crate::highlight::Flattened;
use crate::page_list::{Direction as ListDirection, NodeId, PageList};
use crate::point::{Coordinate, Point, Tag};

#[derive(Debug)]
pub struct ViewportSearch {
    window: SlidingWindow,
    fingerprint: Option<Fingerprint>,

    /// If this is `None`, then active dirty tracking is disabled and if the
    /// viewport overlaps the active area we always re-search. If this is
    /// `Some`, then we only re-search if the active area is dirty. Dirty
    /// marking is up to the caller.
    pub active_dirty: Option<bool>,
}

/// Viewport fingerprint so we can detect when the viewport moves.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    /// The nodes that make up the viewport. We flatten this to a single
    /// list because all that is safe is comparing the node ids.
    nodes: Vec<NodeId>,
}

impl Fingerprint {
    fn new(pages: &PageList) -> Self {
        let mut nodes = Vec::new();
        // Get our viewport area. Bottom right of a viewport can never
        // fail.
        let it = pages.page_iterator(
            ListDirection::RightDown,
            Point::Viewport(Coordinate { x: 0, y: 0 }),
            None,
        );
        for chunk in it {
            nodes.push(chunk.node);
        }
        Self { nodes }
    }
}

impl ViewportSearch {
    pub fn new(needle_unowned: &[u8]) -> Self {
        // We just do a forward search since the viewport is usually
        // pretty small so search results are instant anyways. This avoids
        // a small amount of work to reverse things.
        Self {
            window: SlidingWindow::new(Direction::Forward, needle_unowned),
            fingerprint: None,
            active_dirty: None,
        }
    }

    /// Reset our fingerprint and results so that the next update will
    /// always re-search.
    pub fn reset(&mut self) {
        self.fingerprint = None;
        self.window.clear_and_retain_capacity();
    }

    /// The needle that this search is using.
    pub fn needle(&self) -> &[u8] {
        assert_eq!(self.window.direction(), Direction::Forward);
        self.window.needle()
    }

    /// Update the sliding window to reflect the current viewport. This
    /// will do nothing if the viewport hasn't changed since the last
    /// search.
    ///
    /// The PageList must be safe to read throughout the lifetime of this
    /// function.
    ///
    /// Returns true if the viewport changed and a re-search is needed.
    /// Returns false if the viewport is unchanged.
    pub fn update(&mut self, list: &PageList) -> bool {
        // See if our viewport changed
        let fingerprint = Fingerprint::new(list);
        if let Some(old) = &self.fingerprint {
            if *old == fingerprint {
                // Determine if we need to check if we overlap the active
                // area. If we have dirty tracking on we also set it to
                // false here.
                let check_active = match self.active_dirty {
                    None => true,
                    Some(false) => false,
                    Some(true) => {
                        self.active_dirty = Some(false);
                        true
                    }
                };

                let mut overlaps = false;
                if check_active {
                    // If our fingerprint contains the active area, then we always
                    // re-search since the active area is mutable.
                    let active_tl = list.get_top_left(Tag::Active).expect("active tl");
                    let active_br = list.get_bottom_right(Tag::Active).expect("active br");

                    // If our viewport contains the start or end of the active area,
                    // we are in the active area. We purposely do this first
                    // because our viewport is always larger than the active area.
                    for node in &old.nodes {
                        if *node == active_tl.node || *node == active_br.node {
                            overlaps = true;
                            break;
                        }
                    }
                }

                if !overlaps {
                    // No change
                    return false;
                }
            }
            self.fingerprint = None;
        }
        debug_assert!(self.fingerprint.is_none());

        // If our active area was set as dirty, we always unset it here
        // because we're re-searching now.
        if let Some(v) = &mut self.active_dirty {
            *v = false;
        }

        // Clear our previous sliding window
        self.window.clear_and_retain_capacity();

        // Add enough overlap to cover needle.len - 1 bytes (if it
        // exists) so we can cover the overlap. We only do this for the
        // soft-wrapped prior pages.
        let first = fingerprint.nodes[0];
        let mut node_ = list.node(first).and_then(|n| n.prev);
        let mut added: usize = 0;
        while let Some(id) = node_ {
            let Some(node) = list.node(id) else { break };

            // If the last row of this node isn't wrapped we can't overlap.
            let row = node
                .data
                .get_row(usize::from(node.data.size.rows) - 1)
                .expect("last row");
            if !row.wrap() {
                break;
            }

            // We could be more accurate here and count bytes since the
            // last wrap but its complicated and unlikely multiple pages
            // wrap so this should be fine.
            added += self.window.append(list, id);
            if added >= self.window.needle().len() - 1 {
                break;
            }
            node_ = node.prev;
        }

        // We can use our fingerprint nodes to initialize our sliding
        // window, since we already traversed the viewport once.
        for node in &fingerprint.nodes {
            self.window.append(list, *node);
        }

        // Add any trailing overlap as well.
        'trailing: {
            let end = fingerprint.nodes[fingerprint.nodes.len() - 1];
            let Some(end_node) = list.node(end) else {
                break 'trailing;
            };
            let wraps = end_node
                .data
                .get_row(usize::from(end_node.data.size.rows) - 1)
                .is_some_and(|r| r.wrap());
            if !wraps {
                break 'trailing;
            }

            node_ = end_node.next;
            added = 0;
            while let Some(id) = node_ {
                let Some(node) = list.node(id) else { break };
                added += self.window.append(list, id);
                if added >= self.window.needle().len() - 1 {
                    break;
                }

                // If this row doesn't wrap, then we can quit
                let row = node
                    .data
                    .get_row(usize::from(node.data.size.rows) - 1)
                    .expect("last row");
                if !row.wrap() {
                    break;
                }
                node_ = node.next;
            }
        }

        self.fingerprint = Some(fingerprint);
        true
    }

    /// Find the next match for the needle in the viewport. This returns
    /// `None` when there are no more matches.
    pub fn next(&mut self) -> Option<Flattened> {
        self.window.next()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use crate::terminal::ScrollViewport;

    fn check(list: &PageList, h: &Flattened, tag: Tag, s: (u16, u32), e: (u16, u32)) {
        let sel = h.untracked();
        assert_eq!(list.point_from_pin(tag, sel.start), Some(pt(tag, s.0, s.1)));
        assert_eq!(list.point_from_pin(tag, sel.end), Some(pt(tag, e.0, e.1)));
    }

    #[test]
    fn simple_search() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ViewportSearch::new(b"Fizz");
        assert!(search.update(&t.screen().pages));

        // Viewport contains active so update should always re-search.
        assert!(search.update(&t.screen().pages));

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, Tag::Active, (0, 0), (3, 0));
        let h = search.next().unwrap();
        check(&t.screen().pages, &h, Tag::Active, (0, 2), (3, 2));
        assert!(search.next().is_none());
    }

    #[test]
    fn clear_screen_and_search() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ViewportSearch::new(b"Fizz");
        assert!(search.update(&t.screen().pages));

        feed(&mut t, b"\x1b[2J"); // Clear screen
        feed(&mut t, b"\x1b[H"); // Move cursor home
        feed(&mut t, b"Buzz\r\nFizz\r\nBuzz");
        assert!(search.update(&t.screen().pages));

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, Tag::Active, (0, 1), (3, 1));
        assert!(search.next().is_none());
    }

    #[test]
    fn clear_screen_and_search_dirty_tracking() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ViewportSearch::new(b"Fizz");

        // Turn on dirty tracking
        search.active_dirty = Some(false);

        // Should update since we've never searched before
        assert!(search.update(&t.screen().pages));

        // Should not update since nothing changed
        assert!(!search.update(&t.screen().pages));

        feed(&mut t, b"\x1b[2J"); // Clear screen
        feed(&mut t, b"\x1b[H"); // Move cursor home
        feed(&mut t, b"Buzz\r\nFizz\r\nBuzz");

        // Should still not update since active area isn't dirty
        assert!(!search.update(&t.screen().pages));

        // Mark
        search.active_dirty = Some(true);
        assert!(search.update(&t.screen().pages));

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, Tag::Active, (0, 1), (3, 1));
        assert!(search.next().is_none());
    }

    #[test]
    fn history_search_no_active_area() {
        let mut t = terminal(10, 2);

        // Fill up first page
        let first_page_rows = {
            let l = &t.screen().pages;
            l.node(first(l)).unwrap().data.capacity.rows
        };
        feed(&mut t, b"Fizz\r\n");
        for _ in 1..first_page_rows - 1 {
            feed(&mut t, b"\r\n");
        }
        assert_eq!(t.screen().pages.first, t.screen().pages.last);

        // Create second page
        feed(&mut t, b"\r\n");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);
        feed(&mut t, b"Buzz\r\nFizz");

        t.scroll_viewport(ScrollViewport::Top);

        let mut search = ViewportSearch::new(b"Fizz");
        assert!(search.update(&t.screen().pages));

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, Tag::Screen, (0, 0), (3, 0));
        assert!(search.next().is_none());

        // Viewport doesn't contain active
        assert!(!search.update(&t.screen().pages));
        assert!(search.next().is_none());
    }
}
