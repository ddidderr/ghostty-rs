//! Platform-independent parts of the Metal backend (`renderer/metal/api.zig`,
//! `metal/shaders.zig`, `metal/Texture.zig`, `Metal.zig`): raw enum values,
//! the Metal GPU data layouts, pixel format selection, vertex attribute
//! tables and the small pieces of math the backend does. Everything here is
//! plain Rust so it can be unit tested on any host; `crate::metal` converts
//! these values to `objc2-metal` types.

use bytemuck::{Pod, Zeroable};
use ghostty_config::types_b::AlphaBlending;
use ghostty_font::atlas::Format as AtlasFormat;

use crate::{
    graphics_api::ImageTextureFormat,
    math::Mat,
    shader_types::{BgImage, CellText, Image as ImageVertex, Uniforms},
};

// ---------------------------------------------------------------------------
// Resource options

/// `MTLResourceOptions.CPUCacheMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum CpuCacheMode {
    #[default]
    Default = 0,
    WriteCombined = 1,
}

/// `MTLResourceOptions.StorageMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StorageMode {
    Shared = 0,
    Managed = 1,
    Private = 2,
    Memoryless = 3,
}

/// `MTLResourceOptions.HazardTrackingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum HazardTrackingMode {
    #[default]
    Default = 0,
    Untracked = 1,
    Tracked = 2,
}

/// `MTLResourceOptions` (a `packed struct(c_ulong)` upstream: cache mode in
/// bits 0-3, storage mode in bits 4-7, hazard tracking in bits 8-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceOptions {
    pub cpu_cache_mode: CpuCacheMode,
    pub storage_mode: StorageMode,
    pub hazard_tracking_mode: HazardTrackingMode,
}

impl ResourceOptions {
    /// "Indicate that the CPU writes to this resource but never reads it."
    pub fn write_combined(storage_mode: StorageMode) -> ResourceOptions {
        ResourceOptions {
            cpu_cache_mode: CpuCacheMode::WriteCombined,
            storage_mode,
            hazard_tracking_mode: HazardTrackingMode::Default,
        }
    }

    /// The raw `MTLResourceOptions` value.
    pub fn bits(self) -> u64 {
        u64::from(self.cpu_cache_mode as u8)
            | u64::from(self.storage_mode as u8) << 4
            | u64::from(self.hazard_tracking_mode as u8) << 8
    }
}

/// `MTLTextureUsage` (a `packed struct(c_ulong)` upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextureUsage {
    pub shader_read: bool,
    pub shader_write: bool,
    pub render_target: bool,
    pub pixel_format_view: bool,
    pub shader_atomic: bool,
}

impl TextureUsage {
    /// The raw `MTLTextureUsage` value.
    pub fn bits(self) -> u64 {
        u64::from(self.shader_read)
            | u64::from(self.shader_write) << 1
            | u64::from(self.render_target) << 2
            // bit 3 is skipped by the enum
            | u64::from(self.pixel_format_view) << 4
            | u64::from(self.shader_atomic) << 5
    }

    /// Textures only read from a shader (atlases, images).
    pub fn shader_read_only() -> TextureUsage {
        TextureUsage {
            shader_read: true,
            ..TextureUsage::default()
        }
    }

    /// Custom shader textures: "require both the shader read (for when
    /// multiple shaders are chained) and render target (for the final output)
    /// usage. Disabling either of these will lead to metal validation errors
    /// in Xcode."
    pub fn custom_shader() -> TextureUsage {
        TextureUsage {
            shader_read: true,
            render_target: true,
            ..TextureUsage::default()
        }
    }
}

/// The default storage mode to use for resources created with a device.
///
/// "This is based on whether the device is a discrete GPU or not, since
/// discrete GPUs do not have unified memory and therefore do not support the
/// 'shared' storage mode, instead we have to use the 'managed' mode." The
/// managed mode is not supported by iOS.
pub fn default_storage_mode(is_ios: bool, has_unified_memory: bool) -> StorageMode {
    if is_ios || has_unified_memory {
        StorageMode::Shared
    } else {
        StorageMode::Managed
    }
}

/// The maximum 2D texture size by GPU family
/// (<https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf>).
pub fn max_texture_size(supports_apple10: bool, supports_apple3: bool) -> u32 {
    if supports_apple10 {
        32768
    } else if supports_apple3 {
        16384
    } else {
        8192
    }
}

/// Clamp a layer's pixel size (`bounds * contentsScale`) to the maximum
/// texture size ("we can't create a screen buffer (texture) larger than
/// that"). Truncates like Zig's `@intFromFloat`.
pub fn surface_size(
    bounds_width: f64,
    bounds_height: f64,
    scale: f64,
    max_texture_size: u32,
) -> (u32, u32) {
    let w = (bounds_width * scale) as u32;
    let h = (bounds_height * scale) as u32;
    (w.min(max_texture_size), h.min(max_texture_size))
}

// ---------------------------------------------------------------------------
// Pixel formats

/// The subset of `MTLPixelFormat` the renderer uses, with the raw values from
/// `api.zig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum PixelFormat {
    R8Unorm = 10,
    R8UnormSrgb = 11,
    Rgba8Unorm = 70,
    Rgba8UnormSrgb = 71,
    Bgra8Unorm = 80,
    Bgra8UnormSrgb = 81,
}

impl PixelFormat {
    /// Bytes per pixel (`Texture.bppOf`).
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::R8Unorm | PixelFormat::R8UnormSrgb => 1,
            PixelFormat::Rgba8Unorm
            | PixelFormat::Rgba8UnormSrgb
            | PixelFormat::Bgra8Unorm
            | PixelFormat::Bgra8UnormSrgb => 4,
        }
    }
}

/// The pixel format of render targets, custom shader textures and pipeline
/// color attachments.
///
/// "Using an `*_srgb` pixel format makes Metal gamma encode the pixels
/// written to it *after* blending, which means we get linear alpha blending
/// rather than gamma-incorrect blending."
pub fn color_pixel_format(blending: AlphaBlending) -> PixelFormat {
    if blending.is_linear() {
        PixelFormat::Bgra8UnormSrgb
    } else {
        PixelFormat::Bgra8Unorm
    }
}

/// The pixel format of an image texture (`ImageTextureFormat.toPixelFormat`).
pub fn image_pixel_format(format: ImageTextureFormat, srgb: bool) -> PixelFormat {
    match (format, srgb) {
        (ImageTextureFormat::Gray, true) => PixelFormat::R8UnormSrgb,
        (ImageTextureFormat::Gray, false) => PixelFormat::R8Unorm,
        (ImageTextureFormat::Rgba, true) => PixelFormat::Rgba8UnormSrgb,
        (ImageTextureFormat::Rgba, false) => PixelFormat::Rgba8Unorm,
        (ImageTextureFormat::Bgra, true) => PixelFormat::Bgra8UnormSrgb,
        (ImageTextureFormat::Bgra, false) => PixelFormat::Bgra8Unorm,
    }
}

/// The pixel format of a font atlas texture (`initAtlasTexture`). `None` for
/// formats Metal doesn't support (Zig panics).
pub fn atlas_pixel_format(format: AtlasFormat) -> Option<PixelFormat> {
    match format {
        AtlasFormat::Grayscale => Some(PixelFormat::R8Unorm),
        AtlasFormat::Bgra => Some(PixelFormat::Bgra8UnormSrgb),
        AtlasFormat::Bgr => None,
    }
}

// ---------------------------------------------------------------------------
// Vertex attributes

/// The subset of `MTLVertexFormat` used by our vertex structs (raw values from
/// `api.zig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum VertexFormat {
    Uchar4 = 3,
    Ushort2 = 13,
    Short2 = 16,
    Float = 28,
    Float2 = 29,
    Float4 = 31,
    Uint2 = 37,
    Uchar = 45,
}

/// One vertex attribute (what `Pipeline.autoAttribute` derives by reflection).
/// The attribute index is the position in the table; all attributes read from
/// buffer 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttribute {
    pub format: VertexFormat,
    pub offset: usize,
}

/// A vertex struct usable with a Metal pipeline.
pub trait MetalVertex: Pod {
    const ATTRIBUTES: &'static [VertexAttribute];
}

macro_rules! attr {
    ($ty:ty, $field:ident, $format:expr) => {
        VertexAttribute {
            format: $format,
            offset: core::mem::offset_of!($ty, $field),
        }
    };
}

impl MetalVertex for CellText {
    const ATTRIBUTES: &'static [VertexAttribute] = &[
        attr!(CellText, glyph_pos, VertexFormat::Uint2),
        attr!(CellText, glyph_size, VertexFormat::Uint2),
        attr!(CellText, bearings, VertexFormat::Short2),
        attr!(CellText, grid_pos, VertexFormat::Ushort2),
        attr!(CellText, color, VertexFormat::Uchar4),
        // `atlas` is an enum(u8): its tag type.
        attr!(CellText, atlas, VertexFormat::Uchar),
        // `bools` is a packed struct(u8): its backing integer.
        attr!(CellText, bools, VertexFormat::Uchar),
    ];
}

impl MetalVertex for ImageVertex {
    const ATTRIBUTES: &'static [VertexAttribute] = &[
        attr!(ImageVertex, grid_pos, VertexFormat::Float2),
        attr!(ImageVertex, cell_offset, VertexFormat::Float2),
        attr!(ImageVertex, source_rect, VertexFormat::Float4),
        attr!(ImageVertex, dest_size, VertexFormat::Float2),
    ];
}

impl MetalVertex for BgImage {
    const ATTRIBUTES: &'static [VertexAttribute] = &[
        attr!(BgImage, opacity, VertexFormat::Float),
        // `info` is a packed struct(u8): its backing integer.
        attr!(BgImage, info, VertexFormat::Uchar),
    ];
}

/// `MTLVertexStepFunction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum VertexStepFunction {
    Constant = 0,
    PerVertex = 1,
    PerInstance = 2,
}

// ---------------------------------------------------------------------------
// Blending

/// The color attachment blend state of a pipeline
/// ("We always use premultiplied alpha blending for now.").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendState {
    pub rgb_operation: BlendOperation,
    pub alpha_operation: BlendOperation,
    pub source_rgb: BlendFactor,
    pub source_alpha: BlendFactor,
    pub destination_rgb: BlendFactor,
    pub destination_alpha: BlendFactor,
}

/// `MTLBlendOperation` (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum BlendOperation {
    Add = 0,
}

/// `MTLBlendFactor` (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum BlendFactor {
    One = 1,
    OneMinusSourceAlpha = 5,
}

/// The blend state used by every blending pipeline.
pub const PREMULTIPLIED_ALPHA: BlendState = BlendState {
    rgb_operation: BlendOperation::Add,
    alpha_operation: BlendOperation::Add,
    source_rgb: BlendFactor::One,
    source_alpha: BlendFactor::One,
    destination_rgb: BlendFactor::OneMinusSourceAlpha,
    destination_alpha: BlendFactor::OneMinusSourceAlpha,
};

// ---------------------------------------------------------------------------
// Render pass buffer slots

/// The Metal buffer index for entry `i` of `Step.buffers`
/// (`RenderPass.step`): index 0 is the vertex buffer, and the rest start at
/// index 2 "so that we can use index 1 for the uniforms if present".
pub fn buffer_index(i: usize) -> usize {
    if i == 0 { 0 } else { i + 1 }
}

/// The Metal buffer index of the uniforms.
pub const UNIFORMS_BUFFER_INDEX: usize = 1;

// ---------------------------------------------------------------------------
// Uniforms

/// The uniforms that are passed to our Metal shaders (`metal/shaders.zig`
/// `Uniforms`, an `extern struct` with the alignments of the MSL struct).
///
/// This is the same layout as the OpenGL std140 struct except for `bools`,
/// which is four separate bytes instead of a bit mask, and the width of
/// `padding_extend`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, Pod, Zeroable)]
pub struct MetalUniforms {
    pub projection_matrix: Mat,
    pub screen_size: [f32; 2],
    pub cell_size: [f32; 2],
    pub grid_size: [u16; 2],
    pub pad0: [u8; 12],
    pub grid_padding: [f32; 4],
    /// Bit mask: left, right, up, down (LSB first).
    pub padding_extend: u8,
    pub pad1: [u8; 3],
    pub min_contrast: f32,
    pub cursor_pos: [u16; 2],
    pub cursor_color: [u8; 4],
    pub bg_color: [u8; 4],
    pub cursor_wide: u8,
    pub use_display_p3: u8,
    pub use_linear_blending: u8,
    pub use_linear_correction: u8,
    pub pad2: [u8; 8],
}

impl From<&Uniforms> for MetalUniforms {
    fn from(u: &Uniforms) -> MetalUniforms {
        use crate::shader_types::Bools;
        MetalUniforms {
            projection_matrix: u.projection_matrix,
            screen_size: u.screen_size,
            cell_size: u.cell_size,
            grid_size: u.grid_size,
            pad0: [0; 12],
            grid_padding: u.grid_padding,
            padding_extend: (u.padding_extend & 0xFF) as u8,
            pad1: [0; 3],
            min_contrast: u.min_contrast,
            cursor_pos: u.cursor_pos,
            cursor_color: u.cursor_color,
            bg_color: u.bg_color,
            cursor_wide: u8::from(u.bools & Bools::CURSOR_WIDE != 0),
            use_display_p3: u8::from(u.bools & Bools::USE_DISPLAY_P3 != 0),
            use_linear_blending: u8::from(u.bools & Bools::USE_LINEAR_BLENDING != 0),
            use_linear_correction: u8::from(u.bools & Bools::USE_LINEAR_CORRECTION != 0),
            pad2: [0; 8],
        }
    }
}

// ---------------------------------------------------------------------------
// Custom shaders

/// The entry point of the fragment function spirv-cross (and naga, see
/// `shadertoy::msl_from_compiled`) generates for custom shaders.
pub const POST_FRAGMENT_FN: &str = crate::shadertoy::MSL_ENTRY_POINT;

/// The vertex function post-processing pipelines share with the built-in
/// shaders.
pub const FULL_SCREEN_VERTEX_FN: &str = "full_screen_vertex";

/// The built-in pipelines (`pipeline_descs` in `metal/shaders.zig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineDesc {
    pub name: &'static str,
    pub vertex_fn: &'static str,
    pub fragment_fn: &'static str,
    pub step_fn: VertexStepFunction,
    pub blending_enabled: bool,
    /// Whether the pipeline has a vertex descriptor (vertex attributes).
    pub has_vertex_attributes: bool,
}

pub const PIPELINE_DESCS: [PipelineDesc; 5] = [
    PipelineDesc {
        name: "bg_color",
        vertex_fn: "full_screen_vertex",
        fragment_fn: "bg_color_fragment",
        step_fn: VertexStepFunction::PerVertex,
        blending_enabled: false,
        has_vertex_attributes: false,
    },
    PipelineDesc {
        name: "cell_bg",
        vertex_fn: "full_screen_vertex",
        fragment_fn: "cell_bg_fragment",
        step_fn: VertexStepFunction::PerVertex,
        blending_enabled: true,
        has_vertex_attributes: false,
    },
    PipelineDesc {
        name: "cell_text",
        vertex_fn: "cell_text_vertex",
        fragment_fn: "cell_text_fragment",
        step_fn: VertexStepFunction::PerInstance,
        blending_enabled: true,
        has_vertex_attributes: true,
    },
    PipelineDesc {
        name: "image",
        vertex_fn: "image_vertex",
        fragment_fn: "image_fragment",
        step_fn: VertexStepFunction::PerInstance,
        blending_enabled: true,
        has_vertex_attributes: true,
    },
    PipelineDesc {
        name: "bg_image",
        vertex_fn: "bg_image_vertex",
        fragment_fn: "bg_image_fragment",
        step_fn: VertexStepFunction::PerInstance,
        blending_enabled: true,
        has_vertex_attributes: true,
    },
];

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;
    use crate::shader_types::{Bools, PaddingExtend};

    #[test]
    fn resource_options_bits() {
        // MTLResourceCPUCacheModeWriteCombined | MTLResourceStorageModeShared
        assert_eq!(
            ResourceOptions::write_combined(StorageMode::Shared).bits(),
            1
        );
        // ... | MTLResourceStorageModeManaged (1 << 4)
        assert_eq!(
            ResourceOptions::write_combined(StorageMode::Managed).bits(),
            0x11
        );
        let private = ResourceOptions {
            cpu_cache_mode: CpuCacheMode::Default,
            storage_mode: StorageMode::Private,
            hazard_tracking_mode: HazardTrackingMode::Tracked,
        };
        assert_eq!(private.bits(), (2 << 4) | (2 << 8));
    }

    #[test]
    fn texture_usage_bits() {
        assert_eq!(TextureUsage::shader_read_only().bits(), 1);
        assert_eq!(TextureUsage::custom_shader().bits(), 1 | 4);
        let all = TextureUsage {
            shader_read: true,
            shader_write: true,
            render_target: true,
            pixel_format_view: true,
            shader_atomic: true,
        };
        // The enum skips from 4 to 16.
        assert_eq!(all.bits(), 1 | 2 | 4 | 16 | 32);
    }

    #[test]
    fn storage_mode_selection() {
        assert_eq!(default_storage_mode(false, true), StorageMode::Shared);
        assert_eq!(default_storage_mode(false, false), StorageMode::Managed);
        // Managed storage does not exist on iOS.
        assert_eq!(default_storage_mode(true, false), StorageMode::Shared);
    }

    #[test]
    fn max_texture_size_by_family() {
        assert_eq!(max_texture_size(true, true), 32768);
        assert_eq!(max_texture_size(false, true), 16384);
        assert_eq!(max_texture_size(false, false), 8192);
    }

    #[test]
    fn surface_size_clamps_and_truncates() {
        assert_eq!(surface_size(800.0, 600.0, 2.0, 16384), (1600, 1200));
        assert_eq!(surface_size(800.5, 600.5, 1.5, 16384), (1200, 900));
        assert_eq!(surface_size(10000.0, 10.0, 2.0, 8192), (8192, 20));
        assert_eq!(surface_size(0.0, 0.0, 2.0, 8192), (0, 0));
    }

    #[test]
    fn pixel_format_selection() {
        assert_eq!(
            color_pixel_format(AlphaBlending::Native),
            PixelFormat::Bgra8Unorm
        );
        assert_eq!(
            color_pixel_format(AlphaBlending::Linear),
            PixelFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            color_pixel_format(AlphaBlending::LinearCorrected),
            PixelFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            image_pixel_format(ImageTextureFormat::Gray, true),
            PixelFormat::R8UnormSrgb
        );
        assert_eq!(
            image_pixel_format(ImageTextureFormat::Rgba, false),
            PixelFormat::Rgba8Unorm
        );
        assert_eq!(
            image_pixel_format(ImageTextureFormat::Bgra, true),
            PixelFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            atlas_pixel_format(AtlasFormat::Grayscale),
            Some(PixelFormat::R8Unorm)
        );
        assert_eq!(
            atlas_pixel_format(AtlasFormat::Bgra),
            Some(PixelFormat::Bgra8UnormSrgb)
        );
        assert_eq!(atlas_pixel_format(AtlasFormat::Bgr), None);
    }

    #[test]
    fn pixel_format_raw_values_match_metal() {
        // MTLPixelFormat values (api.zig).
        assert_eq!(PixelFormat::R8Unorm as u64, 10);
        assert_eq!(PixelFormat::Rgba8Unorm as u64, 70);
        assert_eq!(PixelFormat::Bgra8Unorm as u64, 80);
        assert_eq!(PixelFormat::Bgra8UnormSrgb as u64, 81);
        assert_eq!(PixelFormat::Bgra8Unorm.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::R8UnormSrgb.bytes_per_pixel(), 1);
    }

    #[test]
    fn vertex_attribute_tables() {
        let cell_text: Vec<_> = CellText::ATTRIBUTES
            .iter()
            .map(|a| (a.format, a.offset))
            .collect();
        assert_eq!(
            cell_text,
            [
                (VertexFormat::Uint2, 0),
                (VertexFormat::Uint2, 8),
                (VertexFormat::Short2, 16),
                (VertexFormat::Ushort2, 20),
                (VertexFormat::Uchar4, 24),
                (VertexFormat::Uchar, 28),
                (VertexFormat::Uchar, 29),
            ]
        );
        let image: Vec<_> = ImageVertex::ATTRIBUTES
            .iter()
            .map(|a| (a.format, a.offset))
            .collect();
        assert_eq!(
            image,
            [
                (VertexFormat::Float2, 0),
                (VertexFormat::Float2, 8),
                (VertexFormat::Float4, 16),
                (VertexFormat::Float2, 32),
            ]
        );
        let bg: Vec<_> = BgImage::ATTRIBUTES
            .iter()
            .map(|a| (a.format, a.offset))
            .collect();
        assert_eq!(bg, [(VertexFormat::Float, 0), (VertexFormat::Uchar, 4)]);

        // MTLVertexFormat raw values.
        assert_eq!(VertexFormat::Uchar4 as u64, 3);
        assert_eq!(VertexFormat::Ushort2 as u64, 13);
        assert_eq!(VertexFormat::Short2 as u64, 16);
        assert_eq!(VertexFormat::Float as u64, 28);
        assert_eq!(VertexFormat::Float2 as u64, 29);
        assert_eq!(VertexFormat::Float4 as u64, 31);
        assert_eq!(VertexFormat::Uint2 as u64, 37);
        assert_eq!(VertexFormat::Uchar as u64, 45);
    }

    #[test]
    fn buffer_slots() {
        // Vertex buffer at 0, uniforms at 1, the rest start at 2.
        assert_eq!(buffer_index(0), 0);
        assert_eq!(buffer_index(1), 2);
        assert_eq!(buffer_index(2), 3);
        assert_eq!(UNIFORMS_BUFFER_INDEX, 1);
    }

    #[test]
    fn blend_state_is_premultiplied() {
        assert_eq!(PREMULTIPLIED_ALPHA.source_rgb as u64, 1);
        assert_eq!(PREMULTIPLIED_ALPHA.destination_rgb as u64, 5);
        assert_eq!(PREMULTIPLIED_ALPHA.rgb_operation as u64, 0);
    }

    #[test]
    fn metal_uniforms_layout_matches_msl() {
        // The MSL `Uniforms` struct in shaders/shaders.metal.
        assert_eq!(offset_of!(MetalUniforms, projection_matrix), 0);
        assert_eq!(offset_of!(MetalUniforms, screen_size), 64);
        assert_eq!(offset_of!(MetalUniforms, cell_size), 72);
        assert_eq!(offset_of!(MetalUniforms, grid_size), 80);
        assert_eq!(offset_of!(MetalUniforms, grid_padding), 96);
        assert_eq!(offset_of!(MetalUniforms, padding_extend), 112);
        assert_eq!(offset_of!(MetalUniforms, min_contrast), 116);
        assert_eq!(offset_of!(MetalUniforms, cursor_pos), 120);
        assert_eq!(offset_of!(MetalUniforms, cursor_color), 124);
        assert_eq!(offset_of!(MetalUniforms, bg_color), 128);
        assert_eq!(offset_of!(MetalUniforms, cursor_wide), 132);
        assert_eq!(offset_of!(MetalUniforms, use_linear_correction), 135);
        assert_eq!(size_of::<MetalUniforms>(), 144);
        // Same size as the OpenGL std140 block.
        assert_eq!(size_of::<MetalUniforms>(), size_of::<Uniforms>());
    }

    #[test]
    fn metal_uniforms_from_common() {
        let common = Uniforms {
            screen_size: [800.0, 600.0],
            padding_extend: PaddingExtend {
                left: true,
                right: false,
                up: false,
                down: true,
            }
            .to_bits(),
            min_contrast: 3.0,
            cursor_pos: [4, 5],
            bg_color: [1, 2, 3, 4],
            bools: Bools {
                cursor_wide: true,
                use_display_p3: false,
                use_linear_blending: true,
                use_linear_correction: false,
            }
            .to_bits(),
            ..Uniforms::default()
        };
        let m = MetalUniforms::from(&common);
        assert_eq!(m.screen_size, [800.0, 600.0]);
        assert_eq!(m.padding_extend, 0b1001);
        assert!((m.min_contrast - 3.0).abs() < f32::EPSILON);
        assert_eq!(m.cursor_pos, [4, 5]);
        assert_eq!(m.bg_color, [1, 2, 3, 4]);
        let raw: &[u8] = bytemuck::bytes_of(&m);
        // cursor_wide, use_display_p3, use_linear_blending, use_linear_correction
        assert_eq!(&raw[132..136], &[1, 0, 1, 0]);
    }

    #[test]
    fn pipeline_descs_match_shader_functions() {
        let names: Vec<_> = PIPELINE_DESCS.iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            ["bg_color", "cell_bg", "cell_text", "image", "bg_image"]
        );
        // Every function exists in shaders.metal.
        let src = include_str!("../shaders/shaders.metal");
        for d in &PIPELINE_DESCS {
            assert!(
                src.contains(&format!("{}(", d.vertex_fn)),
                "{}",
                d.vertex_fn
            );
            assert!(
                src.contains(&format!("{}(", d.fragment_fn)),
                "{}",
                d.fragment_fn
            );
        }
        assert!(src.contains(FULL_SCREEN_VERTEX_FN));
        // Instanced pipelines have vertex attributes.
        for d in &PIPELINE_DESCS {
            assert_eq!(
                d.has_vertex_attributes,
                d.step_fn == VertexStepFunction::PerInstance
            );
        }
        assert!(!PIPELINE_DESCS[0].blending_enabled);
    }

    #[test]
    fn msl_uniforms_struct_matches_rust() {
        // Guard the shader <-> Rust contract: the MSL struct has these fields
        // in this order.
        let src = include_str!("../shaders/shaders.metal");
        let start = src.find("struct Uniforms {").unwrap();
        let end = start + src[start..].find("};").unwrap();
        let fields: Vec<&str> = src[start..end]
            .lines()
            .skip(1)
            .map(|l| l.trim().trim_end_matches(';'))
            .filter(|l| !l.is_empty())
            .map(|l| l.rsplit(' ').next().unwrap())
            .collect();
        assert_eq!(
            fields,
            [
                "projection_matrix",
                "screen_size",
                "cell_size",
                "grid_size",
                "grid_padding",
                "padding_extend",
                "min_contrast",
                "cursor_pos",
                "cursor_color",
                "bg_color",
                "cursor_wide",
                "use_display_p3",
                "use_linear_blending",
                "use_linear_correction",
            ]
        );
    }
}
