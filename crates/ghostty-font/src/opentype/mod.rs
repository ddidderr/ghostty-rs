//! OpenType table parsing. Port of `font/opentype.zig` and `font/opentype/*`.

pub mod head;
pub mod hhea;
pub mod os2;
pub mod post;
pub mod sfnt;
pub mod svg;

pub use head::Head;
pub use hhea::Hhea;
pub use os2::OS2;
pub use post::Post;
pub use svg::SVG;

/// Error returned when a table is too short (Zig: `error.EndOfStream`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("end of stream")]
pub struct EndOfStream;

/// Minimal big-endian cursor used by the table parsers.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn bytes<const N: usize>(&mut self) -> Result<[u8; N], EndOfStream> {
        let end = self.pos.checked_add(N).ok_or(EndOfStream)?;
        let s = self.data.get(self.pos..end).ok_or(EndOfStream)?;
        self.pos = end;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, EndOfStream> {
        Ok(u16::from_be_bytes(self.bytes()?))
    }
    pub(crate) fn i16(&mut self) -> Result<i16, EndOfStream> {
        Ok(i16::from_be_bytes(self.bytes()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, EndOfStream> {
        Ok(u32::from_be_bytes(self.bytes()?))
    }
    pub(crate) fn i32(&mut self) -> Result<i32, EndOfStream> {
        Ok(i32::from_be_bytes(self.bytes()?))
    }
    pub(crate) fn i64(&mut self) -> Result<i64, EndOfStream> {
        Ok(i64::from_be_bytes(self.bytes()?))
    }
}
