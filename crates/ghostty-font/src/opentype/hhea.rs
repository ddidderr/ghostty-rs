//! Horizontal Header Table (`hhea`). Port of `font/opentype/hhea.zig`.
//!
//! References:
//! - https://learn.microsoft.com/en-us/typography/opentype/spec/hhea
#![allow(non_snake_case)]

use super::{EndOfStream, Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hhea {
    /// Major version number of the horizontal header table — set to 1.
    pub majorVersion: u16,
    /// Minor version number of the horizontal header table — set to 0.
    pub minorVersion: u16,
    /// Typographic ascent (design units).
    pub ascender: i16,
    /// Typographic descent (design units).
    pub descender: i16,
    /// Typographic line gap (design units).
    pub lineGap: i16,
    /// Maximum advance width value in 'hmtx' table.
    pub advanceWidthMax: u16,
    /// Minimum left sidebearing value in 'hmtx' table.
    pub minLeftSideBearing: i16,
    /// Minimum right sidebearing value.
    pub minRightSideBearing: i16,
    /// Max(lsb + (xMax - xMin)).
    pub xMaxExtent: i16,
    /// Used to calculate the slope of the cursor (rise/run).
    pub caretSlopeRise: i16,
    pub caretSlopeRun: i16,
    /// Set to 0 for non-slanted fonts.
    pub caretOffset: i16,
    pub reserved0: i16,
    pub reserved1: i16,
    pub reserved2: i16,
    pub reserved3: i16,
    /// 0 for current format.
    pub metricDataFormat: i16,
    /// Number of hMetric entries in 'hmtx' table.
    pub numberOfHMetrics: u16,
}

impl Hhea {
    pub fn init(data: &[u8]) -> Result<Hhea, EndOfStream> {
        let mut r = Reader::new(data);
        Ok(Hhea {
            majorVersion: r.u16()?,
            minorVersion: r.u16()?,
            ascender: r.i16()?,
            descender: r.i16()?,
            lineGap: r.i16()?,
            advanceWidthMax: r.u16()?,
            minLeftSideBearing: r.i16()?,
            minRightSideBearing: r.i16()?,
            xMaxExtent: r.i16()?,
            caretSlopeRise: r.i16()?,
            caretSlopeRun: r.i16()?,
            caretOffset: r.i16()?,
            reserved0: r.i16()?,
            reserved1: r.i16()?,
            reserved2: r.i16()?,
            reserved3: r.i16()?,
            metricDataFormat: r.i16()?,
            numberOfHMetrics: r.u16()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;
    use crate::opentype::sfnt::Sfnt;

    #[test]
    fn hhea() {
        let font = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let table = font.get_table(b"hhea").unwrap();
        let hhea = Hhea::init(table).unwrap();
        assert_eq!(
            Hhea {
                majorVersion: 1,
                minorVersion: 0,
                ascender: 1900,
                descender: -450,
                lineGap: 0,
                advanceWidthMax: 1200,
                minLeftSideBearing: -1000,
                minRightSideBearing: -1889,
                xMaxExtent: 3089,
                caretSlopeRise: 1,
                caretSlopeRun: 0,
                caretOffset: 0,
                reserved0: 0,
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                metricDataFormat: 0,
                numberOfHMetrics: 2,
            },
            hhea
        );
    }
}
