//! Canonical definitions of the pieces of the Zig font package that the config
//! needs (`font/face.zig` `Variation`, `font/CodepointMap.zig`, and
//! `font/discovery.zig` `Descriptor`). `ghostty-font` depends on this crate,
//! so these live here and the font crate re-exports them.

use std::hash::{Hash, Hasher};

/// A font variation axis setting.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Variation {
    pub id: VariationId,
    pub value: f64,
}

/// A 4-byte variation axis tag (Zig: `Variation.Id`, a packed u32 where
/// `a` is the most significant byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VariationId(pub u32);

impl VariationId {
    #[must_use]
    pub fn init(v: &[u8; 4]) -> Self {
        Self(u32::from_be_bytes(*v))
    }

    /// Converts the ID to its 4 byte string form.
    #[must_use]
    pub fn str(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

/// Descriptor is used to search for fonts. The only required field
/// is "family". The rest are ignored unless they're set to a non-zero
/// value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Descriptor {
    /// Font family to search for. This can be a fully qualified font
    /// name such as "Fira Code", "monospace", "serif", etc.
    ///
    /// On systems that use fontconfig (Linux), this can be a full
    /// fontconfig pattern, such as "Fira Code-14:bold".
    pub family: Option<String>,

    /// Specific font style to search for. This will filter the style
    /// string the font advertises. The "bold/italic" booleans later in this
    /// struct filter by the style trait the font has, not the string, so
    /// these can be used in conjunction or not.
    pub style: Option<String>,

    /// A codepoint that this font must be able to render.
    pub codepoint: u32,

    /// Font size in points that the font should support. For conversion
    /// to pixels, we will use 72 DPI for Mac and 96 DPI for everything else.
    /// (If pixel conversion is necessary, i.e. emoji fonts)
    pub size: f32,

    /// True if we want to search specifically for a font that supports
    /// specific styles.
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,

    /// Variation axes to apply to the font. This also impacts searching
    /// for fonts since fonts with the ability to set these variations
    /// will be preferred, but not guaranteed.
    pub variations: Vec<Variation>,
}

impl Hash for Descriptor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.family.hash(state);
        self.style.hash(state);
        self.codepoint.hash(state);
        self.size.to_bits().hash(state);
        self.bold.hash(state);
        self.italic.hash(state);
        self.monospace.hash(state);
        self.variations.len().hash(state);
        for v in &self.variations {
            v.id.hash(state);
            // Not exact, but we don't depend on the hash being different
            // based on decimal values of variations (same as Zig).
            (v.value as i64).hash(state);
        }
    }
}

impl Descriptor {
    /// A hash code that can be used to identify this descriptor.
    #[must_use]
    pub fn hashcode(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

/// One entry of a [`CodepointMap`].
#[derive(Debug, Clone, PartialEq)]
pub struct CodepointMapEntry {
    /// Unicode codepoint range (inclusive). Invariant: range[0] <= range[1].
    pub range: [u32; 2],
    /// The discovery descriptor of the font to use for this range.
    pub descriptor: Descriptor,
}

/// CodepointMap is a map of codepoints to a discovery descriptor of a font
/// to use for that codepoint. If the descriptor doesn't return any matching
/// font, the codepoint is rendered using the default font.
///
/// Note: we just do a linear search because we expect to always have very
/// few entries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CodepointMap {
    pub list: Vec<CodepointMapEntry>,
}

impl CodepointMap {
    /// Add an entry to the map.
    ///
    /// For conflicting codepoints, entries added later take priority over
    /// entries added earlier.
    pub fn add(&mut self, entry: CodepointMapEntry) {
        assert!(entry.range[0] <= entry.range[1]);
        self.list.push(entry);
    }

    /// Get a descriptor for a codepoint.
    #[must_use]
    pub fn get(&self, cp: u32) -> Option<&Descriptor> {
        self.list
            .iter()
            .rev()
            .find(|e| e.range[0] <= cp && cp <= e.range[1])
            .map(|e| &e.descriptor)
    }

    /// Returns a hash code that can be used to uniquely identify this map.
    #[must_use]
    pub fn hashcode(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.list.len().hash(&mut h);
        for e in &self.list {
            e.range.hash(&mut h);
            e.descriptor.hash(&mut h);
        }
        h.finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn variation_id_wght_should_be_2003265652() {
        let id = VariationId::init(b"wght");
        assert_eq!(id.0, 2003265652);
        assert_eq!(&id.str(), b"wght");
    }

    #[test]
    fn variation_id_slnt_should_be_1936486004() {
        let id = VariationId::init(b"slnt");
        assert_eq!(id.0, 1936486004);
        assert_eq!(&id.str(), b"slnt");
    }

    fn e(a: u32, b: u32, fam: &str) -> CodepointMapEntry {
        CodepointMapEntry {
            range: [a, b],
            descriptor: Descriptor {
                family: Some(fam.into()),
                ..Descriptor::default()
            },
        }
    }

    #[test]
    fn codepointmap() {
        let mut m = CodepointMap::default();
        assert!(m.get(1).is_none());
        m.add(e(1, 1, "A"));
        assert_eq!(m.get(1).unwrap().family.as_deref(), Some("A"));

        // Later entry takes priority
        m.add(e(1, 2, "B"));
        assert_eq!(m.get(1).unwrap().family.as_deref(), Some("B"));

        // Non-matching
        assert!(m.get(0).is_none());
        assert!(m.get(3).is_none());

        m.add(e(3, 4, "C"));
        m.add(e(5, 6, "D"));
        assert_eq!(m.get(3).unwrap().family.as_deref(), Some("C"));
        assert_eq!(m.get(6).unwrap().family.as_deref(), Some("D"));
        assert_eq!(m.get(2).unwrap().family.as_deref(), Some("B"));
    }
}
