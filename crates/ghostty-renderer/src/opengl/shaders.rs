//! Shader pipelines and GPU data layouts for OpenGL (`opengl/shaders.zig`).
use bytemuck::{Pod, Zeroable};

use super::gl::{self, AttribType, Gl};
use super::pipeline::{Attribute, Options as PipelineOptions, Pipeline, StepFunction, Vertex};
use crate::math::Mat;

/// `shaders/glsl/common.glsl`, the only `#include`d file.
const COMMON_GLSL: &str = include_str!("../../shaders/glsl/common.glsl");
const FULL_SCREEN_V: &str = include_str!("../../shaders/glsl/full_screen.v.glsl");
const BG_COLOR_F: &str = include_str!("../../shaders/glsl/bg_color.f.glsl");
const CELL_BG_F: &str = include_str!("../../shaders/glsl/cell_bg.f.glsl");
const CELL_TEXT_V: &str = include_str!("../../shaders/glsl/cell_text.v.glsl");
const CELL_TEXT_F: &str = include_str!("../../shaders/glsl/cell_text.f.glsl");
const IMAGE_V: &str = include_str!("../../shaders/glsl/image.v.glsl");
const IMAGE_F: &str = include_str!("../../shaders/glsl/image.f.glsl");
const BG_IMAGE_V: &str = include_str!("../../shaders/glsl/bg_image.v.glsl");
const BG_IMAGE_F: &str = include_str!("../../shaders/glsl/bg_image.f.glsl");

/// This contains the state for the shaders used by the OpenGL renderer.
pub struct Shaders {
    pub bg_color: Pipeline,
    pub cell_bg: Pipeline,
    pub cell_text: Pipeline,
    pub image: Pipeline,
    pub bg_image: Pipeline,

    /// Custom shaders to run against the final drawable texture. This
    /// can be used to apply a lot of effects. Each shader is run in sequence
    /// against the output of the previous shader.
    pub post_pipelines: Vec<Pipeline>,
}

impl Shaders {
    /// Initialize our shader set.
    ///
    /// "post_shaders" is an optional list of postprocess shaders to run
    /// against the final drawable texture. This is an array of shader source
    /// code, not file paths.
    pub fn init<S: AsRef<str>>(gl: &Gl, post_shaders: &[S]) -> gl::Result<Shaders> {
        let full_screen_v = process_includes(FULL_SCREEN_V);
        let cell_text_v = process_includes(CELL_TEXT_V);
        let image_v = process_includes(IMAGE_V);
        let bg_image_v = process_includes(BG_IMAGE_V);

        let bg_color = Pipeline::init_no_attributes(
            gl,
            &PipelineOptions {
                blending_enabled: false,
                ..PipelineOptions::new(&full_screen_v, &process_includes(BG_COLOR_F))
            },
        )?;
        let cell_bg = Pipeline::init_no_attributes(
            gl,
            &PipelineOptions {
                blending_enabled: true,
                ..PipelineOptions::new(&full_screen_v, &process_includes(CELL_BG_F))
            },
        )?;
        let cell_text = Pipeline::init::<CellText>(
            gl,
            &PipelineOptions {
                step_fn: StepFunction::PerInstance,
                blending_enabled: true,
                ..PipelineOptions::new(&cell_text_v, &process_includes(CELL_TEXT_F))
            },
        )?;
        let image = Pipeline::init::<Image>(
            gl,
            &PipelineOptions {
                step_fn: StepFunction::PerInstance,
                blending_enabled: true,
                ..PipelineOptions::new(&image_v, &process_includes(IMAGE_F))
            },
        )?;
        let bg_image = Pipeline::init::<BgImage>(
            gl,
            &PipelineOptions {
                step_fn: StepFunction::PerInstance,
                blending_enabled: true,
                ..PipelineOptions::new(&bg_image_v, &process_includes(BG_IMAGE_F))
            },
        )?;

        let post_pipelines = init_post_pipelines(gl, post_shaders).unwrap_or_else(|err| {
            // If an error happens while building postprocess shaders we
            // want to just not use any postprocess shaders since we don't
            // want to block Ghostty from working.
            log::warn!("error initializing postprocess shaders err={err}");
            Vec::new()
        });

        Ok(Shaders {
            bg_color,
            cell_bg,
            cell_text,
            image,
            bg_image,
            post_pipelines,
        })
    }
}

/// Initialize our custom shader pipelines. The shaders argument is a
/// set of shader source code, not file paths.
fn init_post_pipelines<S: AsRef<str>>(gl: &Gl, shaders: &[S]) -> gl::Result<Vec<Pipeline>> {
    // If we have no shaders, do nothing. Otherwise if any error happens
    // the already-built pipelines are dropped (destroyed).
    shaders
        .iter()
        .map(|source| init_post_pipeline(gl, source.as_ref()))
        .collect()
}

/// Initialize a single custom shader pipeline from shader source.
fn init_post_pipeline(gl: &Gl, data: &str) -> gl::Result<Pipeline> {
    let vertex = process_includes(FULL_SCREEN_V);
    Pipeline::init_no_attributes(gl, &PipelineOptions::new(&vertex, data))
}

/// Process `#include "file"` directives in an embedded shader source.
///
/// Like upstream this is intentionally simple: it makes a bunch of
/// assumptions about things being well formed and file names not containing
/// quote marks, and the only file that may be included is `common.glsl`
/// (included files are not themselves processed).
pub fn process_includes(contents: &str) -> String {
    let mut i = 0;
    while i < contents.len() {
        if contents[i..].starts_with("#include") {
            assert!(
                contents[i..].starts_with("#include \""),
                "malformed #include directive"
            );
            let start = i + "#include \"".len();
            let end = start
                + contents[start..]
                    .find('"')
                    .expect("unterminated #include directive");
            let name = &contents[start..end];
            let included = match name {
                "common.glsl" => COMMON_GLSL,
                other => panic!("unknown shader include {other:?}"),
            };
            return format!(
                "{}{}{}",
                &contents[..i],
                included,
                process_includes(&contents[end + 1..])
            );
        }
        if let Some(j) = contents[i..].find("\n#") {
            i += j + 1;
        } else {
            break;
        }
    }
    contents.to_owned()
}

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

impl Vertex for CellText {
    const ATTRIBUTES: &'static [Attribute] = &[
        Attribute {
            size: 2,
            ty: AttribType::U32,
            offset: core::mem::offset_of!(CellText, glyph_pos) as u32,
        },
        Attribute {
            size: 2,
            ty: AttribType::U32,
            offset: core::mem::offset_of!(CellText, glyph_size) as u32,
        },
        Attribute {
            size: 2,
            ty: AttribType::I16,
            offset: core::mem::offset_of!(CellText, bearings) as u32,
        },
        Attribute {
            size: 2,
            ty: AttribType::U16,
            offset: core::mem::offset_of!(CellText, grid_pos) as u32,
        },
        Attribute {
            size: 4,
            ty: AttribType::U8,
            offset: core::mem::offset_of!(CellText, color) as u32,
        },
        Attribute {
            size: 1,
            ty: AttribType::U8,
            offset: core::mem::offset_of!(CellText, atlas) as u32,
        },
        Attribute {
            size: 1,
            ty: AttribType::U8,
            offset: core::mem::offset_of!(CellText, bools) as u32,
        },
    ];
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

impl Vertex for Image {
    const ATTRIBUTES: &'static [Attribute] = &[
        Attribute {
            size: 2,
            ty: AttribType::F32,
            offset: core::mem::offset_of!(Image, grid_pos) as u32,
        },
        Attribute {
            size: 2,
            ty: AttribType::F32,
            offset: core::mem::offset_of!(Image, cell_offset) as u32,
        },
        Attribute {
            size: 4,
            ty: AttribType::F32,
            offset: core::mem::offset_of!(Image, source_rect) as u32,
        },
        Attribute {
            size: 2,
            ty: AttribType::F32,
            offset: core::mem::offset_of!(Image, dest_size) as u32,
        },
    ];
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

impl Vertex for BgImage {
    const ATTRIBUTES: &'static [Attribute] = &[
        Attribute {
            size: 1,
            ty: AttribType::F32,
            offset: core::mem::offset_of!(BgImage, opacity) as u32,
        },
        Attribute {
            size: 1,
            ty: AttribType::U8,
            offset: core::mem::offset_of!(BgImage, info) as u32,
        },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn uniforms_std140_layout() {
        // Offsets from the std140 rules applied to the `Globals` block in
        // shaders/glsl/common.glsl.
        assert_eq!(offset_of!(Uniforms, projection_matrix), 0);
        assert_eq!(offset_of!(Uniforms, screen_size), 64);
        assert_eq!(offset_of!(Uniforms, cell_size), 72);
        assert_eq!(offset_of!(Uniforms, grid_size), 80);
        assert_eq!(offset_of!(Uniforms, grid_padding), 96);
        assert_eq!(offset_of!(Uniforms, padding_extend), 112);
        assert_eq!(offset_of!(Uniforms, min_contrast), 116);
        assert_eq!(offset_of!(Uniforms, cursor_pos), 120);
        assert_eq!(offset_of!(Uniforms, cursor_color), 124);
        assert_eq!(offset_of!(Uniforms, bg_color), 128);
        assert_eq!(offset_of!(Uniforms, bools), 132);
        assert_eq!(size_of::<Uniforms>(), 144);
    }

    #[test]
    fn cell_text_layout() {
        // Minimizing the size of this struct is important,
        // so we test it in order to be aware of any changes.
        assert_eq!(size_of::<CellText>(), 32);
        assert_eq!(offset_of!(CellText, glyph_pos), 0);
        assert_eq!(offset_of!(CellText, glyph_size), 8);
        assert_eq!(offset_of!(CellText, bearings), 16);
        assert_eq!(offset_of!(CellText, grid_pos), 20);
        assert_eq!(offset_of!(CellText, color), 24);
        assert_eq!(offset_of!(CellText, atlas), 28);
        assert_eq!(offset_of!(CellText, bools), 29);
        let offsets: Vec<u32> = CellText::ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 8, 16, 20, 24, 28, 29]);
    }

    #[test]
    fn image_layout() {
        assert_eq!(size_of::<Image>(), 48);
        let offsets: Vec<u32> = Image::ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 8, 16, 32]);
    }

    #[test]
    fn bg_image_layout() {
        assert_eq!(size_of::<BgImage>(), 8);
        let offsets: Vec<u32> = BgImage::ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 4]);
        let info = BgImageInfo {
            position: BgImagePosition::Br,
            fit: BgImageFit::None,
            repeat: true,
        };
        assert_eq!(info.to_bits(), 0b0111_1000);
    }

    #[test]
    fn bools_bits_match_glsl() {
        // Mirrors the consts in common.glsl.
        assert!(COMMON_GLSL.contains("const uint CURSOR_WIDE = 1u;"));
        assert!(COMMON_GLSL.contains("const uint USE_DISPLAY_P3 = 2u;"));
        assert!(COMMON_GLSL.contains("const uint USE_LINEAR_BLENDING = 4u;"));
        assert!(COMMON_GLSL.contains("const uint USE_LINEAR_CORRECTION = 8u;"));
        let b = Bools {
            cursor_wide: true,
            use_display_p3: false,
            use_linear_blending: true,
            use_linear_correction: true,
        };
        assert_eq!(b.to_bits(), 1 | 4 | 8);
        assert_eq!(Bools::from_bits(b.to_bits()), b);

        assert!(COMMON_GLSL.contains("const uint EXTEND_LEFT = 1u;"));
        assert!(COMMON_GLSL.contains("const uint EXTEND_DOWN = 8u;"));
        let p = PaddingExtend {
            left: true,
            right: false,
            up: false,
            down: true,
        };
        assert_eq!(p.to_bits(), 9);
        assert_eq!(PaddingExtend::from_bits(9), p);
    }

    #[test]
    fn process_includes_inlines_common() {
        let out = process_includes(CELL_TEXT_V);
        // (common.glsl mentions the directive in a comment, so only check the start.)
        assert!(!out.starts_with("#include"));
        assert!(out.starts_with("#version 430 core"));
        assert!(out.contains("layout(binding = 1, std140) uniform Globals"));
        // Shaders without includes are returned verbatim.
        assert_eq!(process_includes(FULL_SCREEN_V), FULL_SCREEN_V);
    }

    #[test]
    fn process_includes_only_at_line_start() {
        let src =
            "#version 430 core\n// #include \"x\"\n#include \"common.glsl\"\nvoid main() {}\n";
        let out = process_includes(src);
        assert!(out.contains("// #include \"x\""));
        assert!(out.contains("uniform Globals"));
        assert!(out.ends_with("\nvoid main() {}\n"));
    }
}
