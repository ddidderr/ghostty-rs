//! OS/2 and Windows Metrics Table. Port of `font/opentype/os2.zig`.
//!
//! The Zig version has one extern struct per table version and a merged
//! `OS2` struct with optional trailing fields; here the per-version structs
//! are replaced by a single sequential parser that stops at the end of the
//! version-specific layout.
//!
//! References:
//! - https://learn.microsoft.com/en-us/typography/opentype/spec/os2
#![allow(non_snake_case)]

use super::{EndOfStream, Reader};

/// Font selection flags (`fsSelection`), bit 0 is the LSB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FSSelection {
    /// Font contains italic or oblique glyphs.
    pub italic: bool,
    /// Glyphs are underscored.
    pub underscore: bool,
    /// Glyphs have their foreground and background reversed.
    pub negative: bool,
    /// Outline (hollow) glyphs.
    pub outlined: bool,
    /// Glyphs are overstruck.
    pub strikeout: bool,
    /// Glyphs are emboldened.
    pub bold: bool,
    /// Glyphs are in the standard weight/style for the font.
    pub regular: bool,
    /// If set, it is strongly recommended that applications use
    /// OS/2.sTypoAscender - OS/2.sTypoDescender + OS/2.sTypoLineGap as the
    /// default line spacing for this font.
    pub use_typo_metrics: bool,
    /// The font has 'name' table strings consistent with a weight/width/slope
    /// family without requiring use of name IDs 21 and 22.
    pub wws: bool,
    /// Font contains oblique glyphs.
    pub oblique: bool,
}

impl FSSelection {
    pub fn from_u16(v: u16) -> Self {
        let b = |n: u32| v & (1 << n) != 0;
        Self {
            italic: b(0),
            underscore: b(1),
            negative: b(2),
            outlined: b(3),
            strikeout: b(4),
            bold: b(5),
            regular: b(6),
            use_typo_metrics: b(7),
            wws: b(8),
            oblique: b(9),
        }
    }

    pub fn to_u16(self) -> u16 {
        let bits = [
            self.italic,
            self.underscore,
            self.negative,
            self.outlined,
            self.strikeout,
            self.bold,
            self.regular,
            self.use_typo_metrics,
            self.wws,
            self.oblique,
        ];
        bits.iter()
            .enumerate()
            .fold(0u16, |acc, (i, &set)| acc | (u16::from(set) << i))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Os2Error {
    #[error("end of stream")]
    EndOfStream,
    #[error("OS/2 version not supported")]
    OS2VersionNotSupported,
}

impl From<EndOfStream> for Os2Error {
    fn from(_: EndOfStream) -> Self {
        Os2Error::EndOfStream
    }
}

/// Merged view over all OS/2 table versions; fields that only exist in
/// later versions are `Option`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OS2 {
    pub version: u16,
    pub xAvgCharWidth: i16,
    pub usWeightClass: u16,
    pub usWidthClass: u16,
    pub fsType: u16,
    pub ySubscriptXSize: i16,
    pub ySubscriptYSize: i16,
    pub ySubscriptXOffset: i16,
    pub ySubscriptYOffset: i16,
    pub ySuperscriptXSize: i16,
    pub ySuperscriptYSize: i16,
    pub ySuperscriptXOffset: i16,
    pub ySuperscriptYOffset: i16,
    pub yStrikeoutSize: i16,
    pub yStrikeoutPosition: i16,
    pub sFamilyClass: i16,
    pub panose: [u8; 10],
    pub ulUnicodeRange1: u32,
    pub ulUnicodeRange2: u32,
    pub ulUnicodeRange3: u32,
    pub ulUnicodeRange4: u32,
    pub achVendID: [u8; 4],
    pub fsSelection: FSSelection,
    pub usFirstCharIndex: u16,
    pub usLastCharIndex: u16,
    pub sTypoAscender: i16,
    pub sTypoDescender: i16,
    pub sTypoLineGap: i16,
    pub usWinAscent: u16,
    pub usWinDescent: u16,

    // Version 1+
    pub ulCodePageRange1: Option<u32>,
    pub ulCodePageRange2: Option<u32>,

    // Version 2+
    pub sxHeight: Option<i16>,
    pub sCapHeight: Option<i16>,
    pub usDefaultChar: Option<u16>,
    pub usBreakChar: Option<u16>,
    pub usMaxContext: Option<u16>,

    // Version 5
    pub usLowerOpticalPointSize: Option<u16>,
    pub usUpperOpticalPointSize: Option<u16>,
}

impl OS2 {
    pub fn init(data: &[u8]) -> Result<OS2, Os2Error> {
        let mut r = Reader::new(data);
        let version = r.u16()?;
        if version > 5 {
            return Err(Os2Error::OS2VersionNotSupported);
        }

        let mut t = OS2 {
            version,
            xAvgCharWidth: r.i16()?,
            usWeightClass: r.u16()?,
            usWidthClass: r.u16()?,
            fsType: r.u16()?,
            ySubscriptXSize: r.i16()?,
            ySubscriptYSize: r.i16()?,
            ySubscriptXOffset: r.i16()?,
            ySubscriptYOffset: r.i16()?,
            ySuperscriptXSize: r.i16()?,
            ySuperscriptYSize: r.i16()?,
            ySuperscriptXOffset: r.i16()?,
            ySuperscriptYOffset: r.i16()?,
            yStrikeoutSize: r.i16()?,
            yStrikeoutPosition: r.i16()?,
            sFamilyClass: r.i16()?,
            panose: r.bytes()?,
            ulUnicodeRange1: r.u32()?,
            ulUnicodeRange2: r.u32()?,
            ulUnicodeRange3: r.u32()?,
            ulUnicodeRange4: r.u32()?,
            achVendID: r.bytes()?,
            fsSelection: FSSelection::from_u16(r.u16()?),
            usFirstCharIndex: r.u16()?,
            usLastCharIndex: r.u16()?,
            sTypoAscender: r.i16()?,
            sTypoDescender: r.i16()?,
            sTypoLineGap: r.i16()?,
            usWinAscent: r.u16()?,
            usWinDescent: r.u16()?,
            ulCodePageRange1: None,
            ulCodePageRange2: None,
            sxHeight: None,
            sCapHeight: None,
            usDefaultChar: None,
            usBreakChar: None,
            usMaxContext: None,
            usLowerOpticalPointSize: None,
            usUpperOpticalPointSize: None,
        };

        if version >= 1 {
            t.ulCodePageRange1 = Some(r.u32()?);
            t.ulCodePageRange2 = Some(r.u32()?);
        }
        if version >= 2 {
            t.sxHeight = Some(r.i16()?);
            t.sCapHeight = Some(r.i16()?);
            t.usDefaultChar = Some(r.u16()?);
            t.usBreakChar = Some(r.u16()?);
            t.usMaxContext = Some(r.u16()?);
        }
        if version == 5 {
            t.usLowerOpticalPointSize = Some(r.u16()?);
            t.usUpperOpticalPointSize = Some(r.u16()?);
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;
    use crate::opentype::sfnt::Sfnt;

    #[test]
    fn os_2() {
        let font = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let table = font.get_table(b"OS/2").unwrap();
        let os2 = OS2::init(table).unwrap();

        assert_eq!(
            OS2 {
                version: 4,
                xAvgCharWidth: 1200,
                usWeightClass: 400,
                usWidthClass: 5,
                fsType: 0,
                ySubscriptXSize: 1300,
                ySubscriptYSize: 1200,
                ySubscriptXOffset: 0,
                ySubscriptYOffset: 150,
                ySuperscriptXSize: 1300,
                ySuperscriptYSize: 1200,
                ySuperscriptXOffset: 0,
                ySuperscriptYOffset: 700,
                yStrikeoutSize: 100,
                yStrikeoutPosition: 550,
                sFamilyClass: 0,
                panose: [2, 11, 6, 9, 6, 3, 0, 2, 0, 4],
                ulUnicodeRange1: 3843162111,
                ulUnicodeRange2: 3603300351,
                ulUnicodeRange3: 117760229,
                ulUnicodeRange4: 96510060,
                achVendID: *b"corm",
                fsSelection: FSSelection {
                    regular: true,
                    use_typo_metrics: true,
                    ..Default::default()
                },
                usFirstCharIndex: 13,
                usLastCharIndex: 65535,
                sTypoAscender: 1900,
                sTypoDescender: -450,
                sTypoLineGap: 0,
                usWinAscent: 2400,
                usWinDescent: 450,
                ulCodePageRange1: Some(1613234687),
                ulCodePageRange2: Some(0),
                sxHeight: Some(1100),
                sCapHeight: Some(1450),
                usDefaultChar: Some(0),
                usBreakChar: Some(32),
                usMaxContext: Some(126),
                usLowerOpticalPointSize: None,
                usUpperOpticalPointSize: None,
            },
            os2
        );
    }

    #[test]
    fn fs_selection_roundtrip() {
        for v in 0..(1u16 << 10) {
            assert_eq!(FSSelection::from_u16(v).to_u16(), v);
        }
    }

    #[test]
    fn unsupported_version() {
        assert_eq!(OS2::init(&[0, 9]), Err(Os2Error::OS2VersionNotSupported));
        assert_eq!(OS2::init(&[0, 0, 1]), Err(Os2Error::EndOfStream));
    }
}
