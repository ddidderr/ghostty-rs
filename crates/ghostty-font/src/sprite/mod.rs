//! The built-in "sprite font": box drawing, block elements, braille, powerline
//! and other glyphs rasterized by code, plus special sprites (underlines,
//! cursors). Port of `font/sprite.zig`.

pub mod canvas;
pub mod draw;
pub mod face;

pub use canvas::{Box, Canvas, Color, Point};
pub use face::Face;

/// Sprites are represented as special codepoints outside of the Unicode
/// codepoint range. Unicode maxes out at U+10FFFF (21 bits), and we use the
/// high 11 bits to hide our special characters.
///
/// These characters are ONLY used for rendering and NEVER used written to
/// text files or any other exported format, so we don't use the Private Use
/// Area of Unicode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Sprite {
    Underline = Sprite::START,
    UnderlineDouble,
    UnderlineDotted,
    UnderlineDashed,
    UnderlineCurly,

    Strikethrough,

    Overline,

    CursorRect,
    CursorHollowRect,
    CursorBar,
    CursorUnderline,
}

impl Sprite {
    /// Start 1 above the maximum Unicode codepoint.
    pub const START: u32 = 0x1F_FFFF + 1;
    pub const END: u32 = u32::MAX;

    pub const ALL: [Sprite; 11] = [
        Sprite::Underline,
        Sprite::UnderlineDouble,
        Sprite::UnderlineDotted,
        Sprite::UnderlineDashed,
        Sprite::UnderlineCurly,
        Sprite::Strikethrough,
        Sprite::Overline,
        Sprite::CursorRect,
        Sprite::CursorHollowRect,
        Sprite::CursorBar,
        Sprite::CursorUnderline,
    ];

    /// Convert a codepoint in the sprite range to a `Sprite`.
    pub fn from_u32(cp: u32) -> Option<Sprite> {
        Sprite::ALL.iter().copied().find(|s| *s as u32 == cp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_start() {
        assert_eq!(Sprite::START, Sprite::Underline as u32);
        assert_eq!(Sprite::START + 10, Sprite::CursorUnderline as u32);
    }
}
