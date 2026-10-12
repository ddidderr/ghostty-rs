//! PostScript Table (`post`). Port of `font/opentype/post.zig`.
//!
//! References:
//! - https://learn.microsoft.com/en-us/typography/opentype/spec/post
#![allow(non_snake_case)]

use super::{
    EndOfStream,
    Reader,
    sfnt::{Fixed, Version16Dot16},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Post {
    pub version: Version16Dot16,
    /// Italic angle in counter-clockwise degrees from the vertical.
    pub italicAngle: Fixed,
    /// Suggested y offset of the top of the underline stroke (design units).
    pub underlinePosition: i16,
    /// Suggested underline thickness (design units).
    pub underlineThickness: i16,
    /// Set to 0 if the font is proportionally spaced, non-zero if monospaced.
    pub isFixedPitch: u32,
    pub minMemType42: u32,
    pub maxMemType42: u32,
    pub minMemType1: u32,
    pub maxMemType1: u32,
}

impl Post {
    pub fn init(data: &[u8]) -> Result<Post, EndOfStream> {
        let mut r = Reader::new(data);
        Ok(Post {
            version: Version16Dot16::from_u32(r.u32()?),
            italicAngle: Fixed(r.i32()?),
            underlinePosition: r.i16()?,
            underlineThickness: r.i16()?,
            isFixedPitch: r.u32()?,
            minMemType42: r.u32()?,
            maxMemType42: r.u32()?,
            minMemType1: r.u32()?,
            maxMemType1: r.u32()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{embedded, opentype::sfnt::Sfnt};

    #[test]
    fn post() {
        let font = Sfnt::init(embedded::JULIA_MONO).unwrap();
        let table = font.get_table(b"post").unwrap();
        let post = Post::init(table).unwrap();
        assert_eq!(
            Post {
                version: Version16Dot16 { minor: 0, major: 2 },
                italicAngle: Fixed::from_f64(0.0),
                underlinePosition: -200,
                underlineThickness: 100,
                isFixedPitch: 1,
                minMemType42: 0,
                maxMemType42: 0,
                minMemType1: 0,
                maxMemType1: 0,
            },
            post
        );
    }
}
