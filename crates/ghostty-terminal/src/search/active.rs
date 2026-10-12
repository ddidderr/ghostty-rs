//! Searches for a substring within the active area of a PageList.
//!
//! The distinction for "active area" is important because it is the
//! only part of a PageList that is mutable. Therefore, its the only part
//! of the terminal that needs to be repeatedly searched as the contents
//! change.
//!
//! This struct specializes in searching only within that active area,
//! and handling the active area moving as new lines are added to the bottom.

use super::sliding_window::{Direction, SlidingWindow};
use crate::{
    highlight::Flattened,
    page_list::{NodeId, PageList},
};

#[derive(Debug)]
pub struct ActiveSearch {
    pub window: SlidingWindow,
}

impl ActiveSearch {
    pub fn new(needle: &[u8]) -> Self {
        // We just do a forward search since the active area is usually
        // pretty small so search results are instant anyways. This avoids
        // a small amount of work to reverse things.
        Self {
            window: SlidingWindow::new(Direction::Forward, needle),
        }
    }

    /// Update the active area to reflect the current state of the PageList.
    ///
    /// This doesn't do the search, it only copies the necessary data
    /// to perform the search later. This lets the caller hold the lock
    /// on the PageList for a minimal amount of time.
    ///
    /// This returns the first page (in reverse order) covered by this
    /// search. This allows the history search to overlap and search history.
    /// There CAN BE duplicates, and this page CAN BE mutable, so the history
    /// search results should prune anything that's in the active area.
    ///
    /// If the return value is `None` it means the active area covers the entire
    /// PageList, currently.
    pub fn update(&mut self, list: &PageList) -> Option<NodeId> {
        // Clear our previous sliding window
        self.window.clear_and_retain_capacity();

        // First up, add enough pages to cover the active area.
        let mut rem: usize = usize::from(list.rows);
        let mut node_ = list.last;
        let mut last_node: Option<NodeId> = None;
        while let Some(id) = node_ {
            let Some(node) = list.node(id) else { break };
            self.window.append(list, id);
            last_node = Some(id);

            // If we reached our target amount, then this is the last
            // page that contains the active area. We go to the previous
            // page once more since its the first page of our required
            // overlap.
            if rem <= usize::from(node.data.size.rows) {
                node_ = node.prev;
                break;
            }

            rem -= usize::from(node.data.size.rows);
            node_ = node.prev;
        }

        // Next, add enough overlap to cover needle.len - 1 bytes (if it
        // exists) so we can cover the overlap.
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
            let added = self.window.append(list, id);
            if added >= self.window.needle().len() - 1 {
                break;
            }
            node_ = node.prev;
        }

        // Return the last node we added to our window.
        last_node
    }

    /// Find the next match for the needle in the active area. This returns
    /// `None` when there are no more matches.
    pub fn next(&mut self) -> Option<Flattened> {
        self.window.next()
    }
}

#[cfg(test)]
mod tests {
    use super::{super::test_util::*, *};
    use crate::point::Tag;

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

    #[test]
    fn simple_search() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ActiveSearch::new(b"Fizz");
        search.update(&t.screen().pages);

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, (0, 0), (3, 0));
        let h = search.next().unwrap();
        check(&t.screen().pages, &h, (0, 2), (3, 2));
        assert!(search.next().is_none());
    }

    #[test]
    fn clear_screen_and_search() {
        let mut t = terminal(10, 10);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ActiveSearch::new(b"Fizz");
        search.update(&t.screen().pages);

        feed(&mut t, b"\x1b[2J"); // Clear screen
        feed(&mut t, b"\x1b[H"); // Move cursor home
        feed(&mut t, b"Buzz\r\nFizz\r\nBuzz");
        search.update(&t.screen().pages);

        let h = search.next().unwrap();
        check(&t.screen().pages, &h, (0, 1), (3, 1));
        assert!(search.next().is_none());
    }
}
