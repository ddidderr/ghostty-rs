//! ClipboardCodepointMap is a map of codepoints to replacement values
//! for clipboard operations. When copying text to clipboard, matching
//! codepoints will be replaced with their mapped values.

/// Replacement value for a codepoint range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// A single replacement codepoint.
    Codepoint(u32),
    /// A UTF-8 encoded string to replace with.
    String(String),
}

/// One entry of the map.
///
/// TODO(integration): this mirrors `terminal/formatter.zig` `CodepointMap`;
/// replace with `ghostty_terminal::formatter::CodepointMap` once available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Unicode codepoint range to replace.
    /// Asserts: range[0] <= range[1]
    pub range: [u32; 2],
    /// Replacement value for this range.
    pub replacement: Replacement,
}

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
