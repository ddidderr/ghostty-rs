//! Integer widths and safe logical offsets used by terminal pages.
use std::marker::PhantomData;
/// Maximum addressable page size.
pub const MAX_PAGE_SIZE: u32 = u32::MAX;
/// Byte offset within a page.
pub type OffsetInt = u32;
/// Number of cells in a grid dimension.
pub type CellCountInt = u16;
/// Style capacity and identifiers.
pub type StyleCountInt = u16;
/// Hyperlink capacity and identifiers.
pub type HyperlinkCountInt = u16;
/// Bytes of grapheme storage.
pub type GraphemeBytesInt = u32;
/// Bytes of string storage.
pub type StringBytesInt = u32;
/// A typed logical byte offset; never converted to a raw pointer.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Offset<T> {
    offset: u32,
    marker: PhantomData<fn() -> T>,
}
impl<T> Offset<T> {
    /// Construct a logical byte offset.
    pub const fn new(offset: u32) -> Self {
        Self {
            offset,
            marker: PhantomData,
        }
    }
    /// Return the byte displacement.
    pub const fn offset(self) -> u32 {
        self.offset
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offset() {
        assert_eq!(std::mem::size_of::<Offset<u8>>(), 4);
    }
}
