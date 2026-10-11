//! CodepointMap is a map of codepoints to a discovery descriptor of a font
//! to use for that codepoint. If the descriptor doesn't return any matching
//! font, the codepoint is rendered using the default font.
//! Port of `font/CodepointMap.zig`.

//!
//! The canonical definition lives in `ghostty-config` (the `font-codepoint-map`
//! option parses straight into it); it is re-exported here.

pub use ghostty_config::font_types::CodepointMap;
pub use ghostty_config::font_types::CodepointMapEntry as Entry;
#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_config::font_types::Descriptor;

    fn desc(family: &str) -> Descriptor {
        Descriptor {
            family: Some(family.to_string()),
            ..Default::default()
        }
    }

    fn entry(a: u32, b: u32, family: &str) -> Entry {
        Entry {
            range: [a, b],
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
