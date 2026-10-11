//! Font Header Table (`head`). Port of `font/opentype/head.zig`.
//!
//! References:
//! - https://learn.microsoft.com/en-us/typography/opentype/spec/head
//!
//! Field names are in camelCase to match names in the spec.
#![allow(non_snake_case)]

use super::sfnt::Fixed;
use super::{EndOfStream, Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    /// Major version number of the font header table — set to 1.
    pub majorVersion: u16,
    /// Minor version number of the font header table — set to 0.
    pub minorVersion: u16,
    /// Set by font manufacturer.
    pub fontRevision: Fixed,
    /// To compute: set it to 0, sum the entire font as uint32, then store
    /// 0xB1B0AFBA - sum.
    pub checksumAdjustment: u32,
    /// Set to 0x5F0F3CF5.
    pub magicNumber: u32,
    /// Bit flags (see the OpenType spec for bit meanings).
    pub flags: u16,
    /// Set to a value from 16 to 16384.
    pub unitsPerEm: u16,
    /// Seconds since 12:00 midnight, January 1st, 1904, UTC.
    pub created: i64,
    /// Seconds since 12:00 midnight, January 1st, 1904, UTC.
    pub modified: i64,
    /// Minimum x coordinate across all glyph bounding boxes.
    pub xMin: i16,
    /// Minimum y coordinate across all glyph bounding boxes.
    pub yMin: i16,
    /// Maximum x coordinate across all glyph bounding boxes.
    pub xMax: i16,
    /// Maximum y coordinate across all glyph bounding boxes.
    pub yMax: i16,
    /// Bit 0: Bold; 1: Italic; 2: Underline; 3: Outline; 4: Shadow;
    /// 5: Condensed; 6: Extended.
    pub macStyle: u16,
    /// Smallest readable size in pixels.
    pub lowestRecPPEM: u16,
    /// Deprecated (Set to 2).
    pub fontDirectionHint: i16,
    /// 0 for short offsets (Offset16), 1 for long (Offset32).
    pub indexToLocFormat: i16,
    /// 0 for current format.
    pub glyphDataFormat: i16,
}

impl Head {
    /// Parse the table from raw data.
    pub fn init(data: &[u8]) -> Result<Head, EndOfStream> {
        let mut r = Reader::new(data);
        Ok(Head {
            majorVersion: r.u16()?,
            minorVersion: r.u16()?,
            fontRevision: Fixed(r.i32()?),
            checksumAdjustment: r.u32()?,
            magicNumber: r.u32()?,
            flags: r.u16()?,
            unitsPerEm: r.u16()?,
            created: r.i64()?,
            modified: r.i64()?,
            xMin: r.i16()?,
            yMin: r.i16()?,
            xMax: r.i16()?,
            yMax: r.i16()?,
            macStyle: r.u16()?,
            lowestRecPPEM: r.u16()?,
            fontDirectionHint: r.i16()?,
            indexToLocFormat: r.i16()?,
            glyphDataFormat: r.i16()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;
    use crate::opentype::sfnt::Sfnt;

    #[test]
    fn head() {
        let font = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let table = font.get_table(b"head").unwrap();
        let head = Head::init(table).unwrap();
        assert_eq!(
            Head {
                majorVersion: 1,
                minorVersion: 0,
                fontRevision: Fixed::from_f64(0.05499267578125),
                checksumAdjustment: 1007668681,
                magicNumber: 1594834165,
                flags: 7,
                unitsPerEm: 2000,
                created: 3797757830,
                modified: 3797760444,
                xMin: -1000,
                yMin: -1058,
                xMax: 3089,
                yMax: 2400,
                macStyle: 0,
                lowestRecPPEM: 7,
                fontDirectionHint: 2,
                indexToLocFormat: 1,
                glyphDataFormat: 0,
            },
            head
        );
    }

    #[test]
    fn head_too_short() {
        assert_eq!(Head::init(&[0; 10]), Err(EndOfStream));
    }
}
