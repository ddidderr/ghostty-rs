//! CodepointMap is a map of codepoints to a discovery descriptor of a font
//! to use for that codepoint. If the descriptor doesn't return any matching
//! font, the codepoint is rendered using the default font.
//! Port of `font/CodepointMap.zig`.

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::discovery::Descriptor;

#[derive(Debug, Clone)]
pub struct Entry {
    /// Unicode codepoint range (inclusive). Asserts range.0 <= range.1.
    pub range: (u32, u32),

    /// The discovery descriptor of the font to use for this range.
    pub descriptor: Descriptor,
}

/// The list of entries.
///
/// Note: we just do a linear search because we expect to always have very
/// few entries, so the overhead of a binary search is not worth it. This is
/// possible to defeat with some pathological inputs, but there is no realistic
/// scenario where this will be a problem except people trying to fuck around.
#[derive(Debug, Clone, Default)]
pub struct CodepointMap {
    pub list: Vec<Entry>,
}

impl CodepointMap {
    /// Add an entry to the map.
    ///
    /// For conflicting codepoints, entries added later take priority over
    /// entries added earlier.
    pub fn add(&mut self, entry: Entry) {
        assert!(entry.range.0 <= entry.range.1);
        self.list.push(entry);
    }

    /// Get a descriptor for a codepoint.
    pub fn get(&self, cp: u32) -> Option<&Descriptor> {
        self.list
            .iter()
            .rev()
            .find(|e| e.range.0 <= cp && cp <= e.range.1)
            .map(|e| &e.descriptor)
    }

    /// Hash with the given hasher.
    pub fn hash_into<H: Hasher>(&self, hasher: &mut H) {
        self.list.len().hash(hasher);
        for entry in &self.list {
            entry.range.hash(hasher);
            hash_descriptor(&entry.descriptor, hasher);
        }
    }

    /// Returns a hash code that can be used to uniquely identify this
    /// action.
    pub fn hashcode(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash_into(&mut hasher);
        hasher.finish()
    }
}

/// Mirrors `Descriptor.hash` of the Zig discovery module (same fields, same
/// float handling) without depending on how the discovery module exposes it.
fn hash_descriptor<H: Hasher>(d: &Descriptor, hasher: &mut H) {
    d.family.hash(hasher);
    d.style.hash(hasher);
    d.codepoint.hash(hasher);
    d.size.to_bits().hash(hasher);
    d.bold.hash(hasher);
    d.italic.hash(hasher);
    d.monospace.hash(hasher);
    d.variations.len().hash(hasher);
    for v in &d.variations {
        v.id.hash(hasher);
        // This is not correct, but we don't currently depend on the
        // hash value being different based on decimal values of variations.
        (v.value as i64).hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(family: &str) -> Descriptor {
        Descriptor {
            family: Some(family.to_string()),
            ..Default::default()
        }
    }

    fn entry(a: u32, b: u32, family: &str) -> Entry {
        Entry {
            range: (a, b),
            descriptor: desc(family),
        }
    }

    #[test]
    fn codepointmap() {
        let mut m = CodepointMap::default();

        // Exact range
        assert!(m.get(1).is_none());
        m.add(entry(1, 1, "A"));
        assert_eq!("A", m.get(1).unwrap().family.as_deref().unwrap());

        // Later entry takes priority
        m.add(entry(1, 2, "B"));
        assert_eq!("B", m.get(1).unwrap().family.as_deref().unwrap());

        // Non-matching
        assert!(m.get(0).is_none());
        assert!(m.get(3).is_none());

        m.add(entry(3, 4, "C"));
        m.add(entry(5, 6, "D"));
        assert_eq!("C", m.get(3).unwrap().family.as_deref().unwrap());
        assert_eq!("B", m.get(1).unwrap().family.as_deref().unwrap());
    }

    #[test]
    fn hashcode_depends_on_contents() {
        let mut a = CodepointMap::default();
        let mut b = CodepointMap::default();
        assert_eq!(a.hashcode(), b.hashcode());
        a.add(entry(1, 2, "A"));
        assert_ne!(a.hashcode(), b.hashcode());
        b.add(entry(1, 2, "A"));
        assert_eq!(a.hashcode(), b.hashcode());
        b.add(entry(3, 3, "B"));
        assert_ne!(a.hashcode(), b.hashcode());
    }
}
