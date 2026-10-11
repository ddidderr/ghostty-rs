//! SVG glyphs description table. Port of `font/opentype/svg.zig`.
//!
//! This struct is focused purely on the operations we need for Ghostty,
//! namely to be able to look up whether an glyph ID is present in the SVG
//! table or not. It isn't meant to be a general purpose SVG table reader.
//!
//! References:
//! - https://www.w3.org/2013/10/SVG_in_OpenType/#thesvg
//! - https://learn.microsoft.com/en-us/typography/opentype/spec/svg

use super::EndOfStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SvgError {
    #[error("end of stream")]
    EndOfStream,
    #[error("SVG table version not supported")]
    SVGVersionNotSupported,
}

impl From<EndOfStream> for SvgError {
    fn from(_: EndOfStream) -> Self {
        SvgError::EndOfStream
    }
}

/// Size of one document record in bytes.
const RECORD_SIZE: usize = 12;

#[derive(Debug, Clone)]
pub struct SVG<'a> {
    /// The start and end glyph IDs (inclusive) that are present in the
    /// table. This is used to very quickly include/exclude a glyph from
    /// the table.
    pub start_glyph_id: u16,
    pub end_glyph_id: u16,

    /// All records in the table (`len * 12` bytes).
    records: &'a [u8],
}

fn glyph_range(record: &[u8]) -> Result<(u16, u16), SvgError> {
    if record.len() < 4 {
        return Err(SvgError::EndOfStream);
    }
    Ok((
        u16::from_be_bytes([record[0], record[1]]),
        u16::from_be_bytes([record[2], record[3]]),
    ))
}

impl<'a> SVG<'a> {
    pub fn init(data: &'a [u8]) -> Result<SVG<'a>, SvgError> {
        let get = |pos: usize, n: usize| data.get(pos..pos.checked_add(n)?);

        // Version
        let version = get(0, 2).ok_or(SvgError::EndOfStream)?;
        if u16::from_be_bytes([version[0], version[1]]) != 0 {
            return Err(SvgError::SVGVersionNotSupported);
        }

        // Offset
        let off = get(2, 4).ok_or(SvgError::EndOfStream)?;
        let offset = u32::from_be_bytes([off[0], off[1], off[2], off[3]]) as usize;

        // Document list: count followed by records.
        let len_b = get(offset, 2).ok_or(SvgError::EndOfStream)?;
        let len = usize::from(u16::from_be_bytes([len_b[0], len_b[1]]));
        let start = offset + 2;
        // Zig reads records[0] (and the last) unconditionally.
        let records = get(start, len * RECORD_SIZE).ok_or(SvgError::EndOfStream)?;
        if len == 0 {
            return Err(SvgError::EndOfStream);
        }

        let start_range = glyph_range(&records[..RECORD_SIZE])?;
        let end_range = if len == 1 {
            start_range
        } else {
            glyph_range(&records[(len - 1) * RECORD_SIZE..])?
        };

        Ok(SVG {
            start_glyph_id: start_range.0,
            end_glyph_id: end_range.1,
            records,
        })
    }

    pub fn has_glyph(&self, glyph_id: u16) -> bool {
        // Fast path: outside the table range
        if glyph_id < self.start_glyph_id || glyph_id > self.end_glyph_id {
            return false;
        }

        // Fast path, matches the start/end glyph IDs
        if glyph_id == self.start_glyph_id || glyph_id == self.end_glyph_id {
            return true;
        }

        // Slow path: binary search our records
        let (mut lo, mut hi) = (0usize, self.records.len() / RECORD_SIZE);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let rec = &self.records[mid * RECORD_SIZE..(mid + 1) * RECORD_SIZE];
            let (start, end) = glyph_range(rec).unwrap_or((u16::MAX, 0));
            if glyph_id < start {
                hi = mid;
            } else if glyph_id > end {
                lo = mid + 1;
            } else {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;
    use crate::opentype::sfnt::Sfnt;

    // NOTE: the Zig test obtains the table through `Face.copyTable`; we read
    // the same bytes through the SFNT directory (Face-based variant lives
    // with the freetype face tests).
    #[test]
    fn svg() {
        let font = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let table = font.get_table(b"SVG ").unwrap();
        let svg = SVG::init(table).unwrap();
        assert_eq!(11482, svg.start_glyph_id);
        assert_eq!(11482, svg.end_glyph_id);
        assert!(svg.has_glyph(11482));
        assert!(!svg.has_glyph(11483));
    }

    #[test]
    fn svg_bad_version() {
        assert_eq!(
            SVG::init(&[0, 1, 0, 0, 0, 0]).unwrap_err(),
            SvgError::SVGVersionNotSupported
        );
    }
}
