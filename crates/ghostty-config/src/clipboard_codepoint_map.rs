//! ClipboardCodepointMap is a map of codepoints to replacement values
//! for clipboard operations. When copying text to clipboard, matching
//! codepoints will be replaced with their mapped values.

/// One entry of the map and its replacement value. The canonical
/// definitions live in `ghostty_terminal::formatter` (`CodepointMap`,
/// `Replacement`), which is what the terminal formatter consumes.
pub use ghostty_terminal::formatter::{CodepointMap as Entry, Replacement};

/// The list of entries.
///
/// Note: we do a linear search because we expect to always have very
/// few entries, so the overhead of a binary search is not worth it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipboardCodepointMap {
    pub list: Vec<Entry>,
}

impl ClipboardCodepointMap {
    /// Add an entry to the map.
    ///
    /// For conflicting codepoints, entries added later take priority over
    /// entries added earlier.
    pub fn add(&mut self, entry: Entry) {
        assert!(entry.range[0] <= entry.range[1]);
        self.list.push(entry);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn add_and_clone() {
        let mut m = ClipboardCodepointMap::default();
        m.add(Entry {
            range: [1, 2],
            replacement: Replacement::String("x".into()),
        });
        let c = m.clone();
        assert_eq!(m, c);
        assert_eq!(c.list.len(), 1);
    }
}
