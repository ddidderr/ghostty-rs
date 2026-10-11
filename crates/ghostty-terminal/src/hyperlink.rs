//! Owned OSC 8 hyperlinks and per-page identifiers.
/// Nonzero page-local hyperlink identifier.
pub type Id = u16;
/// Mapping from a cell index to its hyperlink.
pub type Map = std::collections::HashMap<usize, Id>;
/// An explicit OSC 8 ID or an automatically generated screen-local ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HyperlinkId {
    /// Application-provided byte string.
    Explicit(Vec<u8>),
    /// Screen-generated identifier.
    Implicit(u32),
}
/// A decoded hyperlink owning its ID and URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    /// Hyperlink identity.
    pub id: HyperlinkId,
    /// URI bytes, which need not be UTF-8.
    pub uri: Vec<u8>,
}
impl Hyperlink {
    /// Duplicate the owned strings.
    pub fn dupe(&self) -> Self {
        self.clone()
    }
    /// Compare identity and URI.
    pub fn eql(&self, other: &Self) -> bool {
        self == other
    }
    /// Logical string allocation including 32-byte chunk rounding.
    pub fn string_bytes(&self) -> usize {
        self.uri.len().next_multiple_of(32)
            + match &self.id {
                HyperlinkId::Explicit(v) => v.len().next_multiple_of(32),
                HyperlinkId::Implicit(_) => 0,
            }
    }
}
/// Owned page entry; unlike Zig it needs no base address for decoding.
pub type PageEntry = Hyperlink;
/// Reference-counted set of page entries.
pub type Set = crate::ref_counted_set::RefCountedSet<PageEntry>;
