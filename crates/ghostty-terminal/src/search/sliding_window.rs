//! Searches page nodes via a sliding window. The sliding window maintains
//! the invariant that data isn't pruned until (1) we've searched it and
//! (2) we've accounted for overlaps across pages to fit the needle.
//!
//! The sliding window is first initialized empty. Pages are then appended
//! in the order to search them. The sliding window supports both a forward
//! and reverse order specified via `new`. The pages should be appended
//! in the correct order matching the search direction.
//!
//! All appends grow the window. The window is only pruned when a search
//! is done (positive or negative match) via `next()`.
//!
//! To avoid unnecessary memory growth, the recommended usage is to
//! call `next()` until it returns `None` and then `append` the next page
//! and repeat the process. This will always maintain the minimum
//! required memory to search for the needle.
//!
//! The caller is responsible for providing the pages and ensuring they're
//! in the proper order. The SlidingWindow itself doesn't own the pages, but
//! it will contain node ids to them in order to return selections. If any
//! pages become invalid, the caller should clear the sliding window and
//! start over.

use std::collections::VecDeque;

use crate::formatter::{Options, PageFormatter};
use crate::highlight::{Chunk, Flattened};
use crate::page::Page;
use crate::page_list::{Node, NodeId, PageList};
use crate::point::Coordinate;

/// The search direction. If the direction is forward then pages should
/// be appended in forward linked list order from the PageList. If the
/// direction is reverse then pages should be appended in reverse order.
///
/// This is important because in most cases, a reverse search is going
/// to be more desirable to search from the end of the active area
/// backwards so more recent data is found first. Supporting both is
/// trivial though and will let us do more complex optimizations in the
/// future (e.g. starting from the viewport and doing a forward/reverse
/// concurrently from that point).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

#[derive(Debug)]
struct Meta {
    node: NodeId,
    serial: u64,
    /// Row count of the node at the time it was appended.
    rows: u16,
    cell_map: Vec<Coordinate>,
}

/// Where a match start was found: (end search start meta index and chars
/// consumed so far, if the end is in a later meta), start meta index, data
/// consumed before the start meta.
type StartFound = (Option<(usize, usize)>, usize, usize);

/// See the module docs.
#[derive(Debug)]
pub struct SlidingWindow {
    /// Circular buffer of the encoded page text we search for the needle.
    ///
    /// Deviation: Zig uses a `CircBuf` and searches its two slices plus an
    /// overlap buffer; here we make the deque contiguous which is
    /// equivalent for match results.
    data: VecDeque<u8>,

    /// Metadata about the pages we're searching, oldest first.
    meta: VecDeque<Meta>,

    /// Offset into data for our current state. This handles the
    /// situation where our search moved through meta[0] but didn't
    /// do enough to prune it.
    data_offset: usize,

    /// The needle we're searching for (reversed for reverse searches).
    needle: Vec<u8>,

    direction: Direction,
}

/// Encode a page with `PageFormatter` (`.emit = .plain, .unwrap = true`),
/// returning the text and the page coordinate of every byte.
pub fn encode_page(page: &Page) -> (Vec<u8>, Vec<Coordinate>) {
    let mut opts = Options::plain();
    opts.unwrap = true;
    let mut map: Vec<Coordinate> = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    let mut f = PageFormatter::new(page, &opts);
    f.point_map = Some(&mut map);
    f.format(&mut out).expect("writing to a Vec cannot fail");
    (out, map)
}

/// ASCII case-insensitive substring search (`std.ascii.indexOfIgnoreCase`).
fn index_of_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

impl SlidingWindow {
    /// The needle must be non-empty.
    pub fn new(direction: Direction, needle_unowned: &[u8]) -> Self {
        let mut needle = needle_unowned.to_vec();
        if direction == Direction::Reverse {
            needle.reverse();
        }
        Self {
            data: VecDeque::new(),
            meta: VecDeque::new(),
            data_offset: 0,
            needle,
            direction,
        }
    }

    /// The needle (reversed if this is a reverse search).
    pub fn needle(&self) -> &[u8] {
        &self.needle
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// Number of data bytes in the window.
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// Number of page metadata entries in the window.
    pub fn meta_len(&self) -> usize {
        self.meta.len()
    }

    /// Clear all data but retain allocated capacity.
    pub fn clear_and_retain_capacity(&mut self) {
        self.meta.clear();
        self.data.clear();
        self.data_offset = 0;
    }

    /// Search the window for the next occurrence of the needle. As
    /// the window moves, the window will prune itself while maintaining
    /// the invariant that the window is always big enough to contain
    /// the needle.
    ///
    /// This returns a flattened highlight on a match. The flattened
    /// highlight can be used to render without a terminal lock because it
    /// doesn't need to traverse the page nodes.
    pub fn next(&mut self) -> Option<Flattened> {
        // If we have less data then the needle then we can't possibly match
        let data_len = self.data.len();
        if data_len < self.needle.len() {
            return None;
        }

        let found = {
            let offset = self.data_offset;
            let hay = &self.data.make_contiguous()[offset..];
            index_of_ignore_case(hay, &self.needle)
        };
        if let Some(idx) = found {
            return Some(self.highlight(idx, self.needle.len()));
        }

        // Special case 1-lengthed needles to delete the entire buffer.
        if self.needle.len() == 1 {
            self.clear_and_retain_capacity();
            self.assert_integrity();
            return None;
        }

        // No match. We keep `needle.len - 1` bytes available to
        // handle the future overlap case.
        'prune: {
            let mut saved: usize = 0;
            let mut found_idx: Option<usize> = None;
            for i in (0..self.meta.len()).rev() {
                let len = self.meta[i].cell_map.len();
                let needed = self.needle.len() - 1 - saved;
                if len >= needed {
                    // We save up to this meta. We set our data offset
                    // to exactly where it needs to be to continue searching.
                    self.data_offset = len - needed;
                    found_idx = Some(i);
                    break;
                }
                saved += len;
            }
            let Some(prune_count) = found_idx else {
                // Never got the amount we needed so there is nothing to prune.
                debug_assert!(saved < self.needle.len() - 1);
                break 'prune;
            };

            if prune_count == 0 {
                // This can happen if we need to save up to the first
                // meta value to retain our window.
                break 'prune;
            }

            // Delete all the metas up to but NOT including the meta we found.
            let mut prune_data_len = 0;
            for m in self.meta.drain(..prune_count) {
                prune_data_len += m.cell_map.len();
            }
            self.data.drain(..prune_data_len);
        }

        // Our data offset now moves to needle.len - 1 from the end so
        // that we can handle the overlap case.
        self.data_offset = self.data.len() - self.needle.len() + 1;

        self.assert_integrity();
        None
    }

    /// Return a flattened highlight for the given start and length.
    ///
    /// The start index is assumed to be relative to the offset. i.e.
    /// index zero is actually at `self.data[self.data_offset]`. The
    /// selection will account for the offset.
    fn highlight(&mut self, start_offset: usize, len: usize) -> Flattened {
        let start = start_offset + self.data_offset;
        let end = start + len - 1;
        debug_assert!(start < self.data.len());
        debug_assert!(start + len <= self.data.len());

        let mut chunks: Vec<Chunk> = Vec::new();
        // Row counts of each chunk's node (for the reverse fixup).
        let mut node_rows: Vec<u16> = Vec::new();
        let mut result = Flattened::default();

        // Go through the meta nodes to find our start.
        let mut meta_consumed = 0usize;
        let mut found: Option<StartFound> = None;
        for (mi, meta) in self.meta.iter().enumerate() {
            // Always increment our consumed count so that our index
            // is right for the end search if we do it.
            let prior_meta_consumed = meta_consumed;
            meta_consumed += meta.cell_map.len();

            // meta_i is the index we expect to find the match in the
            // cell map within this meta if it contains it.
            let meta_i = start - prior_meta_consumed;

            // This meta doesn't contain the match.
            if meta_i >= meta.cell_map.len() {
                continue;
            }

            // Now we look for the end. In MOST cases it is the same as
            // our starting chunk.
            let end_i = end - prior_meta_consumed;
            if end_i < meta.cell_map.len() {
                let start_map = meta.cell_map[meta_i];
                let end_map = meta.cell_map[end_i];
                result.top_x = start_map.x;
                result.bot_x = end_map.x;
                chunks.push(Chunk {
                    node: meta.node,
                    serial: meta.serial,
                    start: start_map.y as u16,
                    end: (end_map.y + 1) as u16,
                });
                node_rows.push(meta.rows);
                found = Some((None, mi, prior_meta_consumed));
            } else {
                // We found the meta that contains the start of the match
                // only. Consume this entire node from our start offset.
                let map = meta.cell_map[meta_i];
                result.top_x = map.x;
                chunks.push(Chunk {
                    node: meta.node,
                    serial: meta.serial,
                    start: map.y as u16,
                    end: meta.rows,
                });
                node_rows.push(meta.rows);
                found = Some((Some((mi + 1, meta_consumed)), mi, prior_meta_consumed));
            }
            break;
        }
        // Precondition that the start index is within the data buffer.
        let (br, prune_meta, prune_data) = found.expect("start index within data buffer");

        // Search for our end.
        if let Some((from, mut consumed)) = br {
            let mut done = false;
            for meta in self.meta.iter().skip(from) {
                let meta_i = end - consumed;
                if meta_i >= meta.cell_map.len() {
                    // This meta doesn't contain the match. We still add it
                    // to our results because we want the full flattened list.
                    chunks.push(Chunk {
                        node: meta.node,
                        serial: meta.serial,
                        start: 0,
                        end: meta.rows,
                    });
                    node_rows.push(meta.rows);
                    consumed += meta.cell_map.len();
                    continue;
                }

                // We found it
                let map = meta.cell_map[meta_i];
                result.bot_x = map.x;
                chunks.push(Chunk {
                    node: meta.node,
                    serial: meta.serial,
                    start: 0,
                    end: (map.y + 1) as u16,
                });
                node_rows.push(meta.rows);
                done = true;
                break;
            }
            // Precondition that the end index is within the data buffer.
            assert!(done, "end index within data buffer");
        }

        // Our offset into the current meta block is the start index
        // minus the amount of data fully consumed. We then add one
        // to move one past the match so we don't repeat it.
        self.data_offset = start - prune_data + 1;

        // If we went beyond our initial meta node we can prune.
        if prune_meta > 0 {
            self.meta.drain(..prune_meta);
            // Delete all the data up to our current index.
            debug_assert!(prune_data > 0);
            self.data.drain(..prune_data);
        }

        if self.direction == Direction::Reverse {
            if chunks.len() > 1 {
                // Reverse all our chunks. This should be pretty obvious why.
                chunks.reverse();
                node_rows.reverse();

                // Normally with forward traversal with multiple pages, the
                // suffix of the first page and the prefix of the last page
                // are used. For a reverse traversal this is inverted, so we
                // fix up start/end to match the forward logic. Middle pages
                // always use the full page.
                let n = chunks.len();
                chunks[0].start = chunks[0].end - 1;
                chunks[0].end = node_rows[0];
                chunks[n - 1].end = chunks[n - 1].start + 1;
                chunks[n - 1].start = 0;
            } else {
                // For a single chunk, the y values are in reverse order
                // (start is the screen-end, end is the screen-start).
                // Swap them to get proper top-to-bottom order.
                let start_y = chunks[0].start;
                chunks[0].start = chunks[0].end - 1;
                chunks[0].end = start_y + 1;
            }

            // X values also need to be reversed since the top/bottom
            // are swapped for the nodes.
            std::mem::swap(&mut result.top_x, &mut result.bot_x);
        }

        result.chunks = chunks;
        result
    }

    /// Add a new node to the sliding window. This will always grow
    /// the sliding window; data isn't pruned until it is consumed
    /// via a search (via `next()`).
    ///
    /// Returns the number of bytes of content added to the sliding window.
    /// The total bytes will be larger since this omits metadata, but it is
    /// an accurate measure of the text content size added.
    pub fn append(&mut self, list: &PageList, id: NodeId) -> usize {
        match list.node(id) {
            Some(node) => self.append_node(id, node),
            None => 0,
        }
    }

    fn append_node(&mut self, id: NodeId, node: &Node) -> usize {
        let (mut written, mut cell_map) = encode_page(&node.data);
        debug_assert_eq!(cell_map.len(), written.len());

        // If the node we're adding isn't soft-wrapped, we add the
        // trailing newline.
        let rows = node.data.size.rows;
        let last_wrap = rows > 0
            && node
                .data
                .get_row(usize::from(rows) - 1)
                .is_some_and(|r| r.wrap());
        if !last_wrap {
            written.push(b'\n');
            let last = cell_map
                .last()
                .copied()
                .unwrap_or(Coordinate { x: 0, y: 0 });
            cell_map.push(last);
        }

        // If our written data is empty, then there is nothing to
        // add to our data set.
        if written.is_empty() {
            self.assert_integrity();
            return 0;
        }

        // If we're doing a reverse search then we need to reverse all
        // our encodings.
        if self.direction == Direction::Reverse {
            written.reverse();
            cell_map.reverse();
        }

        let n = written.len();
        self.data.extend(written);
        self.meta.push_back(Meta {
            node: id,
            serial: node.serial,
            rows,
            cell_map,
        });

        self.assert_integrity();
        n
    }

    /// Only for tests!
    #[cfg(test)]
    fn test_change_needle(&mut self, new: &[u8]) {
        assert_eq!(new.len(), self.needle.len());
        self.needle = new.to_vec();
    }

    fn assert_integrity(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        // Integrity check: verify our data matches our metadata exactly.
        let data_len: usize = self.meta.iter().map(|m| m.cell_map.len()).sum();
        assert_eq!(data_len, self.data.len());

        // Integrity check: verify our data offset is within bounds.
        assert!(self.data.is_empty() || self.data_offset < self.data.len());
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use crate::highlight::Flattened;
    use crate::point::{Point, Tag};
    use crate::screen::Screen;

    fn check(list: &PageList, h: &Flattened, tag: Tag, s: (u16, u32), e: (u16, u32)) {
        let sel = h.untracked();
        assert_eq!(list.point_from_pin(tag, sel.start), Some(pt(tag, s.0, s.1)));
        assert_eq!(list.point_from_pin(tag, sel.end), Some(pt(tag, e.0, e.1)));
    }

    fn check_active(list: &PageList, h: &Flattened, s: (u16, u32), e: (u16, u32)) {
        check(list, h, Tag::Active, s, e);
    }

    /// Fill the first page, ending with `tail` ("boo!" etc.) in the last
    /// row of the first page (preceded by cols-4 x's).
    fn fill_first_page(s: &mut Screen, tail: &str) -> u16 {
        let first_page_rows = s.pages.node(first(&s.pages)).unwrap().data.capacity.rows;
        for _ in 0..first_page_rows - 1 {
            s.test_write_string("\n").unwrap();
        }
        for _ in 0..s.pages.cols - 4 {
            s.test_write_string("x").unwrap();
        }
        s.test_write_string(tail).unwrap();
        first_page_rows
    }

    #[test]
    fn sliding_window_empty_on_init() {
        let w = SlidingWindow::new(Direction::Forward, b"boo!");
        assert_eq!(0, w.data_len());
        assert_eq!(0, w.meta_len());
    }

    #[test]
    fn sliding_window_single_append() {
        let mut w = SlidingWindow::new(Direction::Forward, b"boo!");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 0), (10, 0));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (22, 0));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_case_insensitive_ascii() {
        let mut w = SlidingWindow::new(Direction::Forward, b"Boo!");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 0), (10, 0));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (22, 0));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_single_char() {
        let mut w = SlidingWindow::new(Direction::Forward, b"b");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 0), (7, 0));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (19, 0));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_no_match() {
        let mut w = SlidingWindow::new(Direction::Forward, b"nope!");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // Should still keep the page
        assert_eq!(1, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages() {
        let mut w = SlidingWindow::new(Direction::Forward, b"boo!");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (76, 22), (79, 22));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 23), (10, 23));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_two_pages_single_char() {
        let mut w = SlidingWindow::new(Direction::Forward, b"b");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (76, 22), (76, 22));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 23), (7, 23));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_two_pages_match_across_boundary() {
        let mut w = SlidingWindow::new(Direction::Forward, b"hello, world");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "hell");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("o, world!").unwrap();
        assert_ne!(s.pages.first, s.pages.last);

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (76, 22), (7, 23));
        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // We shouldn't prune because we don't have enough space
        assert_eq!(2, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_no_match_across_boundary_with_newline() {
        let mut w = SlidingWindow::new(Direction::Forward, b"hello, world");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "hell");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\no, world!").unwrap();
        assert_ne!(s.pages.first, s.pages.last);

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // We shouldn't prune because we don't have enough space
        assert_eq!(2, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_no_match_across_boundary_with_newline_reverse() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"hello, world");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "hell");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\no, world!").unwrap();
        assert_ne!(s.pages.first, s.pages.last);

        let node = first(&s.pages);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_two_pages_no_match_prunes_first_page() {
        let mut w = SlidingWindow::new(Direction::Forward, b"nope!");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // We should've pruned our page because the second page
        // has enough text to contain our needle.
        assert_eq!(1, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_no_match_keeps_both_pages() {
        let mut s = screen(80, 24, 1000);
        let first_page_rows = fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        // Imaginary needle for search. Doesn't match!
        let needle = vec![b'x'; usize::from(first_page_rows) * usize::from(s.pages.cols)];
        let mut w = SlidingWindow::new(Direction::Forward, &needle);

        let node = first(&s.pages);
        w.append(&s.pages, node);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);

        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // No pruning because both pages are needed to fit needle.
        assert_eq!(2, w.meta_len());
    }

    // The Zig test asserts the circular buffer wraps; the VecDeque-backed
    // window has no equivalent slice layout so only the behaviour is checked.
    #[test]
    fn sliding_window_single_append_across_circular_buffer_boundary() {
        let mut w = SlidingWindow::new(Direction::Forward, b"abc");
        let mut s = screen(80, 24, 0);
        s.test_write_string("XXXXXXXXXXXXXXXXXXXboo!XXXXX").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);
        w.append(&s.pages, node);

        // Search non-match, prunes page
        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());

        // Change the needle, just needs to be the same length (not a real API)
        w.test_change_needle(b"boo");

        w.append(&s.pages, node);
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (21, 0));
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_match_on_boundary() {
        let mut w = SlidingWindow::new(Direction::Forward, b"abcd");
        let mut s = screen(80, 24, 0);
        s.test_write_string("o!XXXXXXXXXXXXXXXXXXXbo").unwrap();

        // We need to surgically modify the last row to be soft-wrapped
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        {
            let n = s.pages.node_mut(node).unwrap();
            let y = usize::from(n.data.size.rows) - 1;
            n.data.get_row_mut(y).unwrap().set_wrap(true);
        }

        w.append(&s.pages, node);
        w.append(&s.pages, node);

        // Search non-match, prunes page
        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());

        w.test_change_needle(b"boo!");

        w.append(&s.pages, node);
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (21, 0), (1, 0));
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"boo!");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (22, 0));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 0), (10, 0));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_no_match_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"nope!");
        let mut s = screen(80, 24, 0);
        s.test_write_string("hello. boo! hello. boo!").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"boo!");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let node = first(&s.pages);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (7, 23), (10, 23));
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (76, 22), (79, 22));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_two_pages_match_across_boundary_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"hello, world");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "hell");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("o, world!").unwrap();
        assert_ne!(s.pages.first, s.pages.last);

        let node = first(&s.pages);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);
        w.append(&s.pages, node);

        let h = w.next().unwrap();
        check_active(&s.pages, &h, (76, 22), (7, 23));
        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // In reverse mode, the last appended meta (first original page) is large
        // enough to contain needle.len - 1 bytes, so pruning occurs
        assert_eq!(1, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_no_match_prunes_first_page_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"nope!");
        let mut s = screen(80, 24, 1000);
        fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let node = first(&s.pages);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());
    }

    #[test]
    fn sliding_window_two_pages_no_match_keeps_both_pages_reversed() {
        let mut s = screen(80, 24, 1000);
        let first_page_rows = fill_first_page(&mut s, "boo!");
        assert_eq!(s.pages.first, s.pages.last);
        s.test_write_string("\n").unwrap();
        assert_ne!(s.pages.first, s.pages.last);
        s.test_write_string("hello. boo!").unwrap();

        let needle = vec![b'x'; usize::from(first_page_rows) * usize::from(s.pages.cols)];
        let mut w = SlidingWindow::new(Direction::Reverse, &needle);

        let node = first(&s.pages);
        let n2 = next(&s.pages, node);
        w.append(&s.pages, n2);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert!(w.next().is_none());

        // No pruning because both pages are needed to fit needle.
        assert_eq!(2, w.meta_len());
    }

    #[test]
    fn sliding_window_single_append_across_circular_buffer_boundary_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"abc");
        let mut s = screen(80, 24, 0);
        s.test_write_string("XXXXXXXXXXXXXXXXXXXboo!XXXXX").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        w.append(&s.pages, node);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());

        // test_change_needle doesn't reverse, so pass reversed needle
        w.test_change_needle(b"oob");

        w.append(&s.pages, node);
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (19, 0), (21, 0));
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_match_on_boundary_reversed() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"abcd");
        let mut s = screen(80, 24, 0);
        s.test_write_string("o!XXXXXXXXXXXXXXXXXXXbo").unwrap();
        assert_eq!(s.pages.first, s.pages.last);
        let node = first(&s.pages);
        {
            let n = s.pages.node_mut(node).unwrap();
            let y = usize::from(n.data.size.rows) - 1;
            n.data.get_row_mut(y).unwrap().set_wrap(true);
        }

        w.append(&s.pages, node);
        w.append(&s.pages, node);

        assert!(w.next().is_none());
        assert_eq!(1, w.meta_len());

        w.test_change_needle(b"!oob");

        w.append(&s.pages, node);
        let h = w.next().unwrap();
        check_active(&s.pages, &h, (21, 0), (1, 0));
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_soft_wrapped() {
        let mut w = SlidingWindow::new(Direction::Forward, b"boo!");
        let mut t = terminal(4, 5);
        feed(&mut t, b"A\r\nxxboo!\r\nC");
        let screen = t.screen();
        assert_eq!(screen.pages.first, screen.pages.last);
        let node = first(&screen.pages);
        w.append(&screen.pages, node);

        let h = w.next().unwrap();
        check_active(&screen.pages, &h, (2, 1), (1, 2));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    #[test]
    fn sliding_window_single_append_reversed_soft_wrapped() {
        let mut w = SlidingWindow::new(Direction::Reverse, b"boo!");
        let mut t = terminal(4, 5);
        feed(&mut t, b"A\r\nxxboo!\r\nC");
        let screen = t.screen();
        assert_eq!(screen.pages.first, screen.pages.last);
        let node = first(&screen.pages);
        w.append(&screen.pages, node);

        let h = w.next().unwrap();
        check_active(&screen.pages, &h, (2, 1), (1, 2));
        assert!(w.next().is_none());
        assert!(w.next().is_none());
    }

    // This tests a real bug that occurred where a whitespace-only page
    // that encodes to zero bytes would crash.
    #[test]
    fn sliding_window_append_whitespace_only_node() {
        let mut w = SlidingWindow::new(Direction::Forward, b"x");
        let mut s = screen(80, 24, 0);

        // By setting the empty page to wrap we get a zero-byte page.
        let node = first(&s.pages);
        {
            let n = s.pages.node_mut(node).unwrap();
            let y = usize::from(n.data.size.rows) - 1;
            n.data.get_row_mut(y).unwrap().set_wrap(true);
        }
        assert_eq!(s.pages.first, s.pages.last);
        w.append(&s.pages, node);

        // No matches expected
        assert!(w.next().is_none());
    }

    #[allow(dead_code)]
    fn _unused(_: Point) {}
}
