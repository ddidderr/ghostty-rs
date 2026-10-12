//! Shader pipelines and GPU data layouts for OpenGL (`opengl/shaders.zig`).
use super::{
    gl::{self, AttribType, Gl},
    pipeline::{Attribute, Options as PipelineOptions, Pipeline, StepFunction, Vertex},
};

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

pub use crate::shader_types::{
    BgImage,
    BgImageFit,
    BgImageInfo,
    BgImagePosition,
    Bools,
    CellBg,
    CellText,
    CellTextAtlas,
    Image,
    PaddingExtend,
    Uniforms,
};

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
    use core::mem::{offset_of, size_of};

    use super::*;

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
