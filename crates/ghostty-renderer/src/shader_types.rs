//! GPU data layouts shared by every graphics API (`opengl/shaders.zig` and
//! `metal/shaders.zig` define identical CPU-side structs; the only difference
//! is the Metal `Uniforms.bools` layout, see `crate::metal::shaders`).
//!
//! The CPU-side [`Uniforms`] is the std140 layout used by OpenGL; APIs with a
//! different GPU layout convert it with `GraphicsApi::gpu_uniforms`.
use bytemuck::{Pod, Zeroable};

use crate::math::Mat;

/// The uniforms that are passed to our shaders (std140 layout, 144 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, Pod, Zeroable)]
pub struct Uniforms {
    /// The projection matrix for turning world coordinates to normalized.
    /// This is calculated based on the size of the screen.
    pub projection_matrix: Mat,

    /// Size of the screen (render target) in pixels.
    pub screen_size: [f32; 2],

    /// Size of a single cell in pixels, unscaled.
    pub cell_size: [f32; 2],

    /// Size of the grid in columns and rows.
    pub grid_size: [u16; 2],

    // Explicit padding so that `grid_padding` is 16-byte aligned (std140).
    pub pad0: [u8; 12],

    /// The padding around the terminal grid in pixels. In order:
    /// top, right, bottom, left.
    pub grid_padding: [f32; 4],

    /// Bit mask defining which directions to
    /// extend cell colors in to the padding.
    /// Order, LSB first: left, right, up, down. See [`PaddingExtend`].
    pub padding_extend: u32,

    /// The minimum contrast ratio for text. The contrast ratio is calculated
    /// according to the WCAG 2.0 spec.
    pub min_contrast: f32,

    /// The cursor position and color.
    pub cursor_pos: [u16; 2],
    pub cursor_color: [u8; 4],

    /// The background color for the whole surface.
    pub bg_color: [u8; 4],

    /// Various booleans, in a packed bitfield for space efficiency. See [`Bools`].
    pub bools: u32,

    // Tail padding to the struct's 16-byte alignment (extern struct size).
    pub pad1: [u8; 8],
}

/// The `bools` bitfield of [`Uniforms`] (a `packed struct(u32)` upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bools {
    /// Whether the cursor is 2 cells wide.
    pub cursor_wide: bool,

    /// Indicates that colors provided to the shader are already in
    /// the P3 color space, so they don't need to be converted from
    /// sRGB.
    pub use_display_p3: bool,

    /// Indicates that the color attachments for the shaders have
    /// an `*_srgb` pixel format, which means the shaders need to
    /// output linear RGB colors rather than gamma encoded colors,
    /// since blending will be performed in linear space and then
    /// the GPU itself will re-encode the colors for storage.
    pub use_linear_blending: bool,

    /// Enables a weight correction step that makes text rendered
    /// with linear alpha blending have a similar apparent weight
    /// (thickness) to gamma-incorrect blending.
    pub use_linear_correction: bool,
}

impl Bools {
    pub const CURSOR_WIDE: u32 = 1;
    pub const USE_DISPLAY_P3: u32 = 2;
    pub const USE_LINEAR_BLENDING: u32 = 4;
    pub const USE_LINEAR_CORRECTION: u32 = 8;

    pub fn to_bits(self) -> u32 {
        u32::from(self.cursor_wide)
            | u32::from(self.use_display_p3) << 1
            | u32::from(self.use_linear_blending) << 2
            | u32::from(self.use_linear_correction) << 3
    }

    pub fn from_bits(bits: u32) -> Self {
        Bools {
            cursor_wide: bits & Self::CURSOR_WIDE != 0,
            use_display_p3: bits & Self::USE_DISPLAY_P3 != 0,
            use_linear_blending: bits & Self::USE_LINEAR_BLENDING != 0,
            use_linear_correction: bits & Self::USE_LINEAR_CORRECTION != 0,
        }
    }
}

/// The `padding_extend` bitfield of [`Uniforms`] (a `packed struct(u32)` upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaddingExtend {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

impl PaddingExtend {
    pub fn to_bits(self) -> u32 {
        u32::from(self.left)
            | u32::from(self.right) << 1
            | u32::from(self.up) << 2
            | u32::from(self.down) << 3
    }

    pub fn from_bits(bits: u32) -> Self {
        PaddingExtend {
            left: bits & 1 != 0,
            right: bits & 2 != 0,
            up: bits & 4 != 0,
            down: bits & 8 != 0,
        }
    }
}

/// Which atlas a glyph lives in (`CellText.Atlas`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CellTextAtlas {
    Grayscale = 0,
    Color = 1,
}

/// This is a single parameter for the terminal cell shader (32 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Pod, Zeroable)]
pub struct CellText {
    pub glyph_pos: [u32; 2],
    pub glyph_size: [u32; 2],
    pub bearings: [i16; 2],
    pub grid_pos: [u16; 2],
    pub color: [u8; 4],
    /// A [`CellTextAtlas`] value.
    pub atlas: u8,
    /// Bit 0: `no_min_contrast`, bit 1: `is_cursor_glyph`. See the consts below.
    pub bools: u8,
    pub pad: [u8; 2],
}

impl CellText {
    pub const NO_MIN_CONTRAST: u8 = 1;
    pub const IS_CURSOR_GLYPH: u8 = 2;

    pub fn atlas(&self) -> CellTextAtlas {
        if self.atlas == 0 {
            CellTextAtlas::Grayscale
        } else {
            CellTextAtlas::Color
        }
    }

    pub fn set_atlas(&mut self, a: CellTextAtlas) {
        self.atlas = a as u8;
    }
}

/// This is a single parameter for the cell bg shader.
pub type CellBg = [u8; 4];

/// Single parameter for the image shader. See shader for field details.
/// (48 bytes: the Zig extern struct is 16-byte aligned.)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, Pod, Zeroable)]
pub struct Image {
    pub grid_pos: [f32; 2],
    pub cell_offset: [f32; 2],
    pub source_rect: [f32; 4],
    pub dest_size: [f32; 2],
    pub pad: [f32; 2],
}

/// Single parameter for the bg image shader (8 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, Pod, Zeroable)]
pub struct BgImage {
    pub opacity: f32,
    /// A packed [`BgImageInfo`] byte.
    pub info: u8,
    pub pad: [u8; 3],
}

/// `BgImage.Info`, a `packed struct(u8)`: position:u4, fit:u2, repeat:u1, pad:u1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgImageInfo {
    pub position: BgImagePosition,
    pub fit: BgImageFit,
    pub repeat: bool,
}

impl BgImageInfo {
    pub fn to_bits(self) -> u8 {
        (self.position as u8) | (self.fit as u8) << 4 | u8::from(self.repeat) << 6
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BgImagePosition {
    Tl = 0,
    Tc = 1,
    Tr = 2,
    Ml = 3,
    Mc = 4,
    Mr = 5,
    Bl = 6,
    Bc = 7,
    Br = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BgImageFit {
    Contain = 0,
    Cover = 1,
    Stretch = 2,
    None = 3,
}
