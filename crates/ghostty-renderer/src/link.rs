//! Link matching for the renderer (`renderer/link.zig`).

use std::collections::HashSet;

use ghostty_input::Mods;
use ghostty_input::link::{Highlight, Link as InputLink};
use ghostty_terminal::point::Coordinate;
use ghostty_terminal::render::RenderState;
use onig::{Regex, Region, SearchOptions};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A configured link regex failed to compile.
    #[error("invalid link regex: {0}")]
    Regex(#[from] onig::Error),
    /// The render state contains an invalid codepoint so it can't be
    /// converted to a string.
    #[error("render state contains an invalid codepoint")]
    InvalidString,
}

/// The link configuration needed for renderers.
pub struct Link {
    /// The regular expression to match the link against.
    pub regex: Regex,

    /// The situations in which the link should be highlighted.
    pub highlight: Highlight,
}

/// A set of links. This provides a higher level API for renderers
/// to match against a viewport and determine if cells are part of
/// a link.
#[derive(Default)]
pub struct Set {
    pub links: Vec<Link>,
}

impl Set {
    /// Returns the set of links from the configuration.
    pub fn from_config(config: &[InputLink]) -> Result<Set, Error> {
        let mut links = Vec::with_capacity(config.len());
        for link in config {
            links.push(Link {
                regex: Regex::new(&link.regex)?,
                highlight: link.highlight,
            });
        }
        Ok(Set { links })
    }

    /// Fills `result` with the matches from regex link matches.
    pub fn render_cell_map(
        &self,
        result: &mut HashSet<Coordinate>,
        render_state: &RenderState,
        mouse_viewport: Option<Coordinate>,
        mouse_mods: Mods,
    ) -> Result<(), Error> {
        // Fast path, not very likely since we have default links.
        if self.links.is_empty() {
            return Ok(());
        }

        // Convert our render state to a string + byte map.
        let mut str = String::new();
        let mut map: Vec<Coordinate> = Vec::new();
        render_state
            .string(&mut str, Some(&mut map))
            .map_err(|_| Error::InvalidString)?;

        // Go through each link and see if we have any matches.
        for link in &self.links {
            // Determine if our highlight conditions are met. We use a
            // match here instead of an if so that we get a compile
            // error if any other conditions are added.
            match link.highlight {
                Highlight::Always => {}
                Highlight::AlwaysMods(v) => {
                    if !mouse_mods.equal(v) {
                        continue;
                    }
                }

                // We check the hover points later.
                Highlight::Hover => {
                    if mouse_viewport.is_none() {
                        continue;
                    }
                }
                Highlight::HoverMods(v) => {
                    if mouse_viewport.is_none() {
                        continue;
                    }
                    if !mouse_mods.equal(v) {
                        continue;
                    }
                }
            }

            let mut offset: usize = 0;
            while offset < str.len() {
                let haystack = &str[offset..];
                let mut region = Region::new();
                let found = link.regex.search_with_options(
                    haystack,
                    0,
                    haystack.len(),
                    SearchOptions::SEARCH_OPTION_NONE,
                    Some(&mut region),
                );
                // A mismatch (or any search failure such as hitting the
                // retry limit) ends the search for this link.
                let Some((offset_start, offset_end)) = found.and(region.pos(0)) else {
                    break;
                };

                // We have a match!
                let start = offset + offset_start;
                let end = offset + offset_end;

                // Increment our offset by the number of bytes in the match.
                // We do this after handling the match (Zig: `defer`). A
                // zero-length match advances by one character so we always
                // make progress (Zig would loop forever here).
                let next_offset = if end > offset {
                    end
                } else {
                    let step = str[offset..].chars().next().map_or(1, char::len_utf8);
                    offset + step
                };

                let hovered = match link.highlight {
                    Highlight::Always | Highlight::AlwaysMods(_) => true,
                    Highlight::Hover | Highlight::HoverMods(_) => match mouse_viewport {
                        Some(vp) => map[start..end].contains(&vp),
                        None => false,
                    },
                };

                if hovered {
                    // Record the match
                    for pt in &map[start..end] {
                        result.insert(*pt);
                    }
                }

                offset = next_offset;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_input::link::Action;
    use ghostty_terminal::terminal::{Options, Terminal};

    fn link(regex: &str, highlight: Highlight) -> InputLink {
        InputLink {
            regex: regex.to_string(),
            action: Action::Open,
            highlight,
        }
    }

    fn state() -> RenderState {
        let mut t = Terminal::new(Options::new(5, 3)).unwrap();
        {
            let mut s = t.vt_stream();
            s.next_slice(b"1ABCD2EFGH\r\n3IJKL").unwrap();
        }
        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
        state
    }

    fn c(x: u16, y: u32) -> Coordinate {
        Coordinate { x, y }
    }

    #[test]
    fn render_cell_map() {
        let state = state();
        let set = Set::from_config(&[link("AB", Highlight::Always), link("EF", Highlight::Always)])
            .unwrap();

        // Get our matches
        let mut result = HashSet::new();
        set.render_cell_map(&mut result, &state, None, Mods::default())
            .unwrap();
        assert!(!result.contains(&c(0, 0)));
        assert!(result.contains(&c(1, 0)));
        assert!(result.contains(&c(2, 0)));
        assert!(!result.contains(&c(3, 0)));
        assert!(result.contains(&c(1, 1)));
        assert!(!result.contains(&c(1, 2)));
    }

    #[test]
    fn render_cell_map_hover_links() {
        let state = state();
        let set = Set::from_config(&[link("AB", Highlight::Hover), link("EF", Highlight::Always)])
            .unwrap();

        // Not hovering over the first link
        {
            let mut result = HashSet::new();
            set.render_cell_map(&mut result, &state, None, Mods::default())
                .unwrap();
            assert!(!result.contains(&c(0, 0)));
            assert!(!result.contains(&c(1, 0)));
            assert!(!result.contains(&c(2, 0)));
            assert!(!result.contains(&c(3, 0)));
            assert!(result.contains(&c(1, 1)));
            assert!(!result.contains(&c(1, 2)));
        }

        // Hovering over the first link
        {
            let mut result = HashSet::new();
            set.render_cell_map(&mut result, &state, Some(c(1, 0)), Mods::default())
                .unwrap();
            assert!(!result.contains(&c(0, 0)));
            assert!(result.contains(&c(1, 0)));
            assert!(result.contains(&c(2, 0)));
            assert!(!result.contains(&c(3, 0)));
            assert!(result.contains(&c(1, 1)));
            assert!(!result.contains(&c(1, 2)));
        }
    }

    #[test]
    fn render_cell_map_mods_no_match() {
        let state = state();
        let set = Set::from_config(&[
            link("AB", Highlight::Always),
            link(
                "EF",
                Highlight::AlwaysMods(Mods {
                    ctrl: true,
                    ..Mods::default()
                }),
            ),
        ])
        .unwrap();

        let mut result = HashSet::new();
        set.render_cell_map(&mut result, &state, None, Mods::default())
            .unwrap();
        assert!(!result.contains(&c(0, 0)));
        assert!(result.contains(&c(1, 0)));
        assert!(result.contains(&c(2, 0)));
        assert!(!result.contains(&c(3, 0)));
        assert!(!result.contains(&c(1, 1)));
        assert!(!result.contains(&c(1, 2)));
    }

    #[test]
    fn default_url_regex_matches() {
        let mut t = Terminal::new(Options::new(30, 2)).unwrap();
        {
            let mut s = t.vt_stream();
            s.next_slice(b"see https://example.com now").unwrap();
        }
        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
        let set = Set::from_config(&[link(ghostty_config::url::REGEX, Highlight::Always)]).unwrap();
        let mut result = HashSet::new();
        set.render_cell_map(&mut result, &state, None, Mods::default())
            .unwrap();
        assert!(!result.contains(&c(0, 0)));
        assert!(result.contains(&c(4, 0)));
        assert!(result.contains(&c(20, 0)));
        assert!(!result.contains(&c(24, 0)));
    }

    #[test]
    fn invalid_regex_is_error() {
        assert!(Set::from_config(&[link("(", Highlight::Always)]).is_err());
    }
}
