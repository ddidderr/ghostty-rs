//! Searches for a term in a PageList structure.
//!
//! This searches in reverse order starting from the given node.
//!
//! This assumes that nodes do not change contents. For nodes that change
//! contents, look at `ActiveSearch`, which is designed to re-search the active
//! area since it assumed to change. When integrating `ActiveSearch` with
//! `PageListSearch`, the caller should start the `PageListSearch` from the
//! returned node from `ActiveSearch::update()`.
//!
//! Concurrent access to a PageList or nodes in a PageList are not allowed,
//! so the caller should ensure that necessary locks are held. Each function
//! documents whether it accesses the PageList or not. For example, you can
//! safely call `next()` without holding a lock, but you must hold a lock
//! while calling `feed()`.
//!
//! Deviation: Zig stores a `*PageList`; here every method that touches the
//! list takes it as a parameter.

use super::sliding_window::{Direction, SlidingWindow};
use crate::highlight::Flattened;
use crate::page::Error;
use crate::page_list::{NodeId, PageList, Pin, TrackedPin};

#[derive(Debug)]
pub struct PageListSearch {
    /// The sliding window of page contents and nodes to search.
    pub window: SlidingWindow,

    /// The tracked pin for our current position in the pagelist. This
    /// will always point to the CURRENT node we're searching from so that
    /// we can track if we move.
    pub pin: TrackedPin,
}

impl PageListSearch {
    /// Initialize the page list search. The needle is copied so it can
    /// be freed immediately.
    ///
    /// Accesses the PageList/Node so the caller must ensure it is safe
    /// to do so if there is any concurrent access.
    pub fn new(needle: &[u8], list: &mut PageList, start: NodeId) -> Result<Self, Error> {
        // We put a tracked pin into the node that we're starting from.
        // By using a tracked pin, we can keep our pagelist references safe
        // because if the pagelist prunes pages, the tracked pin will
        // be moved somewhere safe.
        let (rows, cols) = {
            let n = list.node(start).ok_or(Error::InvalidId)?;
            (n.data.size.rows, n.data.size.cols)
        };
        let pin = list.track_pin(Pin {
            node: start,
            y: rows - 1,
            x: cols - 1,
            garbage: false,
        })?;

        // Create our sliding window we'll use for searching.
        let mut window = SlidingWindow::new(Direction::Reverse, needle);

        // We always feed our initial page data into the window, because
        // we have the lock anyways and this lets our `pin` point to our
        // current node and feed to work properly.
        window.append(list, start);

        Ok(Self { window, pin })
    }

    /// Modifies the PageList (to untrack a pin) so the caller must ensure
    /// that it is safe to do so.
    pub fn deinit(&mut self, list: &mut PageList) {
        list.untrack_pin(self.pin);
    }

    /// Return the next match in the loaded page nodes. If this returns
    /// `None` then the PageList search needs to be fed the next node(s).
    /// Call `feed` to do this.
    ///
    /// Beware that the selection returned may point to a node that
    /// is freed if the caller does not hold necessary locks on the
    /// PageList while searching. The pins should be validated prior to
    /// final use.
    ///
    /// This does NOT access the PageList, so it can be called without
    /// a lock held.
    pub fn next(&mut self) -> Option<Flattened> {
        self.window.next()
    }

    /// Feed more data to the sliding window from the pagelist. This will
    /// feed enough data to cover at least one match (needle length) if it
    /// exists; this doesn't perform the search, it only feeds data.
    ///
    /// This accesses nodes in the PageList, so the caller must ensure
    /// it is safe to do so (i.e. hold necessary locks).
    ///
    /// This returns false if there is no more data to feed. This essentially
    /// means we've searched the entire pagelist.
    pub fn feed(&mut self, list: &mut PageList) -> Result<bool, Error> {
        // If our pin becomes garbage it means wherever we were next
        // was reused and we can't make sense of our progress anymore.
        // It is effectively equivalent to reaching the end of the PageList.
        let Some(pin) = list.tracked_pin(self.pin) else {
            return Ok(false);
        };
        if pin.garbage {
            return Ok(false);
        }

        // Add at least enough data to find a single match.
        let mut rem = self.window.needle().len();

        // Start at our previous node and then continue adding until we
        // get our desired amount of data.
        let mut node_ = list.node(pin.node).and_then(|n| n.prev);
        while let Some(id) = node_ {
            rem = rem.saturating_sub(self.window.append(list, id));

            // Move our tracked pin to the new node.
            let (rows, cols, prev) = {
                let n = list.node(id).ok_or(Error::InvalidId)?;
                (n.data.size.rows, n.data.size.cols, n.prev)
            };
            list.set_tracked_pin(
                self.pin,
                Pin {
                    node: id,
                    y: rows - 1,
                    x: cols - 1,
                    garbage: false,
                },
            )?;

            if rem == 0 {
                break;
            }
            node_ = prev;
        }

        // True if we fed any data.
        Ok(rem < self.window.needle().len())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use crate::point::Tag;
    use crate::screen::SelectionString;
    use crate::selection::Selection;

    fn check(list: &PageList, h: &Flattened, s: (u16, u32), e: (u16, u32)) {
        let sel = h.untracked();
        assert_eq!(
            list.point_from_pin(Tag::Active, sel.start),
            Some(active(s.0, s.1))
        );
        assert_eq!(
            list.point_from_pin(Tag::Active, sel.end),
            Some(active(e.0, e.1))
        );
    }

    fn first_page_rows(t: &crate::terminal::Terminal) -> u16 {
        let l = &t.screen().pages;
        l.node(first(l)).unwrap().data.capacity.rows
    }

    #[test]
    fn simple_search() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Fizz", &mut t.screen_mut().pages, last_id).unwrap();

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, (0, 2), (3, 2));
        let h = search.next().unwrap();
        check(&t.screen().pages, &h, (0, 0), (3, 0));
        assert!(search.next().is_none());

        // We should not be able to feed since we have one page
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_multiple_pages_with_matches() {
        let mut t = terminal(10, 10);
        let rows = first_page_rows(&t);
        for _ in 0..rows - 1 {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"Fizz");
        assert_eq!(t.screen().pages.first, t.screen().pages.last);

        // Create second page
        feed(&mut t, b"\r\n");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);
        feed(&mut t, b"Buzz\r\nFizz");

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Fizz", &mut t.screen_mut().pages, last_id).unwrap();

        // First match on the last page
        assert!(search.next().is_some());
        assert!(search.next().is_none());

        // Feed should succeed and load the first page
        assert!(search.feed(&mut t.screen_mut().pages).unwrap());

        // Now we should find the match on the first page
        assert!(search.next().is_some());
        assert!(search.next().is_none());

        // No more pages to feed
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_multiple_pages_no_matches() {
        let mut t = terminal(10, 10);
        let rows = first_page_rows(&t);
        for _ in 0..rows - 1 {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"Hello");

        // Create second page
        feed(&mut t, b"\r\n");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);
        feed(&mut t, b"World");

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Nope", &mut t.screen_mut().pages, last_id).unwrap();

        // No matches on last page
        assert!(search.next().is_none());

        // Feed first page
        assert!(search.feed(&mut t.screen_mut().pages).unwrap());

        // Still no matches
        assert!(search.next().is_none());

        // No more pages
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_iteratively_through_multiple_matches() {
        let mut t = terminal(80, 24);
        let rows = first_page_rows(&t);

        // Fill first page with a match at the end
        for _ in 0..rows - 1 {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"Page1Test");
        assert_eq!(t.screen().pages.first, t.screen().pages.last);

        // Create second page with a match
        feed(&mut t, b"\r\n");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);
        feed(&mut t, b"Page2Test");

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Test", &mut t.screen_mut().pages, last_id).unwrap();

        // Match on page 2
        assert!(search.next().is_some());
        assert!(search.next().is_none());

        // Feed page 1
        assert!(search.feed(&mut t.screen_mut().pages).unwrap());
        assert!(search.next().is_some());
        assert!(search.next().is_none());

        // No more pages
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_with_match_spanning_page_boundary() {
        let mut t = terminal(80, 24);
        let rows = first_page_rows(&t);

        // Fill first page ending with "Te"
        for _ in 0..rows - 1 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.cols - 2 {
            feed(&mut t, b"x");
        }
        feed(&mut t, b"Te");
        assert_eq!(t.screen().pages.first, t.screen().pages.last);

        // Second page starts with "st"
        feed(&mut t, b"st");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Test", &mut t.screen_mut().pages, last_id).unwrap();

        // No complete match on last page alone (only has "st")
        assert!(search.next().is_none());

        // Feed first page - this should give us enough data to find "Test"
        assert!(search.feed(&mut t.screen_mut().pages).unwrap());

        // Should find the spanning match
        let h = search.next().unwrap();
        let sel = h.untracked();
        assert_ne!(sel.start.node, sel.end.node);
        {
            let str = t
                .screen()
                .selection_string(SelectionString::new(Selection::new(
                    sel.start, sel.end, false,
                )));
            assert_eq!(str, "Test");
        }

        // No more matches
        assert!(search.next().is_none());

        // No more pages
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_with_match_spanning_page_boundary_with_newline() {
        let mut t = terminal(80, 24);
        let rows = first_page_rows(&t);

        for _ in 0..rows - 1 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.cols - 2 {
            feed(&mut t, b"x");
        }
        feed(&mut t, b"Te");
        assert_eq!(t.screen().pages.first, t.screen().pages.last);

        // Second page starts with "st"
        feed(&mut t, b"\r\n");
        assert_ne!(t.screen().pages.first, t.screen().pages.last);
        feed(&mut t, b"st");

        let last_id = last(&t.screen().pages);
        let mut search = PageListSearch::new(b"Test", &mut t.screen_mut().pages, last_id).unwrap();

        // Should not find any matches since we broke with an explicit newline.
        assert!(search.next().is_none());
        assert!(search.feed(&mut t.screen_mut().pages).unwrap());
        assert!(search.next().is_none());
        assert!(!search.feed(&mut t.screen_mut().pages).unwrap());
        search.deinit(&mut t.screen_mut().pages);
    }

    #[test]
    fn feed_with_pruned_page() {
        // Zero here forces minimum max size to effectively two pages.
        let mut p = PageList::init(80, 24, Some(0)).unwrap();

        // Grow to capacity
        let page1_node = p.last.unwrap();
        let (cap1, size1) = {
            let d = &p.node(page1_node).unwrap().data;
            (d.capacity.rows, d.size.rows)
        };
        for _ in 0..cap1 - size1 {
            assert!(p.grow().unwrap().is_none());
        }

        // Grow and allocate one more page. Then fill that page up.
        let page2_node = p.grow().unwrap().unwrap();
        let (cap2, size2) = {
            let d = &p.node(page2_node).unwrap().data;
            (d.capacity.rows, d.size.rows)
        };
        for _ in 0..cap2 - size2 {
            assert!(p.grow().unwrap().is_none());
        }

        // Setup search and feed until we can't
        let last_id = p.last.unwrap();
        let mut search = PageListSearch::new(b"Test", &mut p, last_id).unwrap();
        assert!(search.feed(&mut p).unwrap());
        assert!(!search.feed(&mut p).unwrap());

        // Next should create a new page, but it should reuse our first
        // page since we're at max size.
        let new = p.grow().unwrap().unwrap();
        assert_eq!(p.last.unwrap(), new);

        // Our first should now be page2. (Zig also asserts the last node
        // pointer equals the old first; node ids carry a generation here so
        // the reused node has a new id.)
        assert_eq!(p.first.unwrap(), page2_node);

        // Feed should still do nothing
        assert!(!search.feed(&mut p).unwrap());
        search.deinit(&mut p);
    }
}
