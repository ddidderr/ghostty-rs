//! SFNT container and basic OpenType data types. Port of
//! `font/opentype/sfnt.zig`.

use super::{EndOfStream, Reader};

pub type Tag = [u8; 4];

/// 32-bit signed fixed-point number (16.16). Stored as the raw `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fixed(pub i32);

impl Fixed {
    pub fn from_f64(float: f64) -> Self {
        Fixed((float * 65536.0).round() as i32)
    }
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 65536.0
    }
    /// Fractional bits (low 16).
    pub fn frac(self) -> u16 {
        (self.0 as u32 & 0xFFFF) as u16
    }
    /// Integer part (high 16, signed).
    pub fn int(self) -> i16 {
        (self.0 >> 16) as i16
    }
}

/// 16-bit signed fixed number with the low 14 bits of fraction (2.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct F2Dot14(pub i16);

impl F2Dot14 {
    pub fn from_f64(float: f64) -> Self {
        F2Dot14((float * 16384.0).round() as i16)
    }
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 16384.0
    }
}

/// Packed 32-bit value with major and minor version numbers.
/// (Zig `packed struct(u32) { minor: u16, major: u16 }`: `minor` is the low
/// half, so reading a big-endian u32 yields `major` in the high half.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version16Dot16 {
    pub minor: u16,
    pub major: u16,
}

impl Version16Dot16 {
    pub fn from_u32(v: u32) -> Self {
        Self {
            minor: (v & 0xFFFF) as u16,
            major: (v >> 16) as u16,
        }
    }
}

/// 32-bit signed 26.6 fixed point numbers. Stored as the raw `i32`
/// (the 26.6 bit pattern, i.e. what FreeType calls `FT_F26Dot6`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct F26Dot6(pub i32);

impl F26Dot6 {
    const FRAC_FACTOR: f64 = 64.0;
    const HALF: i32 = 1 << 5;

    pub fn from_f64(float: f64) -> Self {
        F26Dot6((float * Self::FRAC_FACTOR).round() as i32)
    }

    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / Self::FRAC_FACTOR
    }

    pub fn to_f32(self) -> f32 {
        self.0 as f32 / 64.0
    }

    /// The fractional bits (low 6, unsigned).
    pub fn frac(self) -> u8 {
        (self.0 & 0x3F) as u8
    }

    /// The integer part (floor).
    pub fn int(self) -> i32 {
        self.0 >> 6
    }

    /// Round to the nearest integer, .5 rounds away from 0.
    pub fn round(self) -> i32 {
        if self.0 & Self::HALF != 0 {
            self.int() + 1
        } else {
            self.int()
        }
    }
}

impl std::fmt::Display for F26Dot6 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SfntError {
    #[error("end of stream")]
    EndOfStream,
}

impl From<EndOfStream> for SfntError {
    fn from(_: EndOfStream) -> Self {
        SfntError::EndOfStream
    }
}

/// The static portion of the table directory (the "offset subtable").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffsetSubtable {
    /// Indicates the type of font file we're reading.
    ///   - 0x00_01_00_00   ----   TrueType
    ///   - 0x74_72_75_65  'true'  TrueType
    ///   - 0x4F_54_54_4F  'OTTO'  OpenType
    ///   - 0x74_79_70_31  'typ1'  PostScript
    pub sfnt_version: u32,
    pub num_tables: u16,
    pub search_range: u16,
    pub entry_selector: u16,
    pub range_shift: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableRecord {
    pub tag: Tag,
    pub checksum: u32,
    /// Offset from beginning of font file.
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct Directory {
    pub offset: OffsetSubtable,
    pub records: Vec<TableRecord>,
}

/// Wrapper for parsing a SFNT font and accessing its tables.
#[derive(Debug, Clone)]
pub struct Sfnt<'a> {
    pub directory: Directory,
    data: &'a [u8],
}

impl<'a> Sfnt<'a> {
    /// Parse a font from raw data. The struct keeps a reference to `data`.
    pub fn init(data: &'a [u8]) -> Result<Sfnt<'a>, SfntError> {
        let mut r = Reader::new(data);
        let offset = OffsetSubtable {
            sfnt_version: r.u32()?,
            num_tables: r.u16()?,
            search_range: r.u16()?,
            entry_selector: r.u16()?,
            range_shift: r.u16()?,
        };
        let mut records = Vec::with_capacity(usize::from(offset.num_tables));
        for _ in 0..offset.num_tables {
            records.push(TableRecord {
                tag: r.bytes()?,
                checksum: r.u32()?,
                offset: r.u32()?,
                length: r.u32()?,
            });
        }
        Ok(Sfnt {
            directory: Directory { offset, records },
            data,
        })
    }

    /// Returns the bytes of the table with the provided tag if present.
    /// (None as well if the record points outside of the data.)
    pub fn get_table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        let record = self.directory.records.iter().find(|r| &r.tag == tag)?;
        let start = record.offset as usize;
        let end = start.checked_add(record.length as usize)?;
        self.data.get(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;

    #[test]
    fn fixed_point() {
        let p = F26Dot6::from_f64(26.6);
        assert_eq!(p.int(), 26);
        assert_eq!(p.frac(), 38);
        assert_eq!(p.to_f64(), 26.59375);
        assert_eq!(p.round(), 27);

        let n = F26Dot6::from_f64(-26.6);
        assert_eq!(n.int(), -27);
        assert_eq!(n.frac(), 26);
        assert_eq!(n.to_f64(), -26.59375);
        assert_eq!(n.round(), -27);
    }

    #[test]
    fn parse_font() {
        let sfnt = Sfnt::init(embedded::JULIA_MONO).unwrap();
        assert_eq!(19, sfnt.directory.offset.num_tables);
        assert_eq!(b"prep", &sfnt.directory.records[18].tag);
    }

    #[test]
    fn get_table() {
        let sfnt = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let svg = sfnt.get_table(b"SVG ").unwrap();
        assert_eq!(430, svg.len());
    }

    #[test]
    fn truncated_data_is_an_error() {
        assert!(Sfnt::init(&[0, 1, 0, 0, 0, 5]).is_err());
    }
}
