//! A string along with the mapping of each individual byte in the string
//! to the point in the screen.
//!
//! Port of `ZIG/src/terminal/StringMap.zig`. Regex matching uses Oniguruma
//! (the `onig` crate), like Ghostty.
use crate::{page_list::Pin, selection::Selection};
use onig::{MatchParam, Regex, Region, SearchOptions};

/// Retry budget for StringMap regex searches.
///
/// Units are Oniguruma retry steps (internal backtracking/retry counter),
/// not bytes/characters/time.
///
/// This is Zig's `setRetryLimitInSearch`: a whole-search budget (set via
/// the `ghostty-onig` wrapper since the `onig` crate does not expose it).
/// Exceeding it aborts matching with an error which is treated as "no
/// further matches".
const ONI_SEARCH_RETRY_LIMIT: u32 = 100_000;

/// A string and a map from each byte of the string to the pin it came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringMap {
    pub string: String,
    pub map: Vec<Pin>,
}

impl StringMap {
    /// Returns an iterator that yields the next match of the given regex.
    pub fn search_iterator<'a>(&'a self, regex: &'a Regex) -> SearchIterator<'a> {
        SearchIterator {
            map: self,
            regex,
            offset: 0,
        }
    }
}

/// Iterates over the regular expression matches of the string.
pub struct SearchIterator<'a> {
    map: &'a StringMap,
    regex: &'a Regex,
    offset: usize,
}

impl<'a> SearchIterator<'a> {
    /// Returns the next regular expression match or None if there are
    /// no more matches.
    pub fn next_match(&mut self) -> Option<Match<'a>> {
        if self.offset >= self.map.string.len() {
            return None;
        }

        // Use per-search match params so we can bound regex retry steps
        // (Oniguruma's internal backtracking work counter).
        let mut match_param = MatchParam::default();
        ghostty_onig::set_retry_limit_in_search(&mut match_param, ONI_SEARCH_RETRY_LIMIT);

        let haystack = &self.map.string[self.offset..];
        let mut region = Region::new();
        let result = self.regex.search_with_param(
            haystack,
            0,
            haystack.len(),
            SearchOptions::SEARCH_OPTION_NONE,
            Some(&mut region),
            match_param,
        );

        // Mismatch and retry/stack-limit errors mean no further matches.
        let Ok(Some(_)) = result else {
            self.offset = self.map.string.len();
            return None;
        };
        let (start, end) = region.pos(0)?;

        // Increment our offset by the number of bytes in the match.
        // We do this after building the match so that it is returned
        // with the old offset.
        let m = Match {
            map: self.map,
            offset: self.offset,
            start,
            end,
        };
        self.offset += end;
        Some(m)
    }
}

impl<'a> Iterator for SearchIterator<'a> {
    type Item = Match<'a>;

    fn next(&mut self) -> Option<Match<'a>> {
        self.next_match()
    }
}

/// A single regular expression match.
#[derive(Debug, Clone, Copy)]
pub struct Match<'a> {
    map: &'a StringMap,
    offset: usize,
    /// Byte range of the match relative to `offset`.
    start: usize,
    end: usize,
}

impl Match<'_> {
    /// Returns the selection containing the full match.
    pub fn selection(&self) -> Selection {
        let start_pt = self.map.map[self.offset + self.start];
        let end_pt = self.map.map[self.offset + self.end - 1];
        Selection::new(start_pt, end_pt, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        point::{Coordinate, Point, Tag},
        screen::{Options, Screen, SelectLine, SelectionString},
    };

    fn active(x: u16, y: u32) -> Point {
        Point::Active(Coordinate { x, y })
    }

    fn screen_pt(s: &Screen, pin: Pin) -> Point {
        s.pages.point_from_pin(Tag::Screen, pin).unwrap()
    }

    fn line_map(s: &Screen, x: u16, y: u32) -> StringMap {
        let pin = s.pages.pin(active(x, y)).unwrap();
        let line = s.select_line(SelectLine::new(pin)).unwrap();
        s.selection_string_map(SelectionString {
            sel: line,
            trim: false,
        })
    }

    #[test]
    fn string_map_search_iterator() {
        // Initialize our regex
        let re = Regex::new("[A-B]{2}").unwrap();

        // Initialize our screen
        let mut s = Screen::new(Options::new(5, 5, 0)).unwrap();
        s.test_write_string("1ABCD2EFGH\n3IJKL").unwrap();
        let map = line_map(&s, 2, 1);

        // Get our iterator
        let mut it = map.search_iterator(&re);
        {
            let m = it.next_match().unwrap();
            let sel = m.selection();
            assert_eq!(
                Point::Screen(Coordinate { x: 1, y: 0 }),
                screen_pt(&s, sel.start(&s.pages))
            );
            assert_eq!(
                Point::Screen(Coordinate { x: 2, y: 0 }),
                screen_pt(&s, sel.end(&s.pages))
            );
        }

        assert!(it.next_match().is_none());
    }

    #[test]
    fn string_map_search_iterator_url_detection() {
        let re = Regex::new(ghostty_config::url::REGEX).unwrap();

        let mut s = Screen::new(Options::new(40, 5, 0)).unwrap();
        s.test_write_string("hello https://example.com/path world")
            .unwrap();

        let map = line_map(&s, 10, 0);

        let mut it = map.search_iterator(&re);
        {
            let m = it.next_match().unwrap();
            let sel = m.selection();
            // URL should start at x=6 ("https://example.com/path" starts after "hello ")
            assert_eq!(
                Point::Screen(Coordinate { x: 6, y: 0 }),
                screen_pt(&s, sel.start(&s.pages))
            );
            // URL should end at x=29 (end of "/path")
            assert_eq!(
                Point::Screen(Coordinate { x: 29, y: 0 }),
                screen_pt(&s, sel.end(&s.pages))
            );
        }

        assert!(it.next_match().is_none());
    }

    #[test]
    fn string_map_search_iterator_retry_limit_bounds_catastrophic_regex() {
        // Exponential backtracking: without the whole-search retry limit
        // this would effectively never finish.
        let re = Regex::new("(a+)+$").unwrap();
        let mut s = Screen::new(Options::new(60, 5, 0)).unwrap();
        s.test_write_string(&format!("{}b", "a".repeat(50)))
            .unwrap();
        let map = line_map(&s, 10, 0);
        let mut it = map.search_iterator(&re);
        assert!(it.next_match().is_none());
    }

    #[test]
    fn string_map_search_iterator_url_with_click_position() {
        let re = Regex::new(ghostty_config::url::REGEX).unwrap();

        let mut s = Screen::new(Options::new(40, 5, 0)).unwrap();
        s.test_write_string("hello https://example.com world")
            .unwrap();

        // Simulate clicking on "example" (x=14)
        let click_pin = s.pages.pin(active(14, 0)).unwrap();

        let map = line_map(&s, 14, 0);

        let mut it = map.search_iterator(&re);
        let mut found_url = false;
        while let Some(m) = it.next_match() {
            let sel = m.selection();
            if sel.contains(&s.pages, click_pin) {
                found_url = true;
                // Verify URL bounds
                assert_eq!(
                    Point::Screen(Coordinate { x: 6, y: 0 }),
                    screen_pt(&s, sel.start(&s.pages))
                );
                assert_eq!(
                    Point::Screen(Coordinate { x: 24, y: 0 }),
                    screen_pt(&s, sel.end(&s.pages))
                );
                break;
            }
        }
        assert!(found_url);
    }
}
