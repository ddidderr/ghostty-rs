//! Glyph is a single loaded glyph for a face. Port of `font/Glyph.zig`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Glyph {
    /// width of glyph in pixels
    pub width: u32,

    /// height of glyph in pixels
    pub height: u32,

    /// left bearing
    pub offset_x: i32,

    /// top bearing
    pub offset_y: i32,

    /// coordinates in the atlas of the top-left corner. These have to
    /// be normalized to be between 0 and 1 prior to use in shaders.
    pub atlas_x: u32,
    pub atlas_y: u32,
}
