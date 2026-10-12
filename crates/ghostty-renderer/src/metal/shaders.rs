//! Shader pipelines and GPU data layouts for Metal (`metal/shaders.zig`).
//!
//! The CPU-side data layouts are shared with OpenGL
//! ([`crate::shader_types`]) except for the uniform block, see
//! [`crate::metal_layout::MetalUniforms`].
use std::time::Instant;

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{MTLDevice as _, MTLLibrary};

use super::{
    Device,
    Error,
    pipeline::{self, Attachment, Pipeline},
};
use crate::{
    graphics_api::GpuShaders,
    metal_layout::{
        FULL_SCREEN_VERTEX_FN,
        PIPELINE_DESCS,
        POST_FRAGMENT_FN,
        PipelineDesc,
        PixelFormat,
        VertexStepFunction,
    },
    shader_types::{BgImage, CellText, Image as ImageVertex},
};

/// The Metal shading language source of all built-in shaders
/// (`shaders/shaders.metal`). Ghostty compiles this to a `.metallib` at build
/// time with `xcrun metal`; we compile it with `newLibraryWithSource` when the
/// shaders are initialized.
pub const SHADERS_METAL: &str = include_str!("../../shaders/shaders.metal");

/// This contains the state for the shaders used by the Metal renderer.
pub struct Shaders {
    library: Retained<ProtocolObject<dyn MTLLibrary>>,

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

// SAFETY: MTLLibrary objects are thread-safe, and pipelines are immutable.
unsafe impl Send for Shaders {}
// SAFETY: see above.
unsafe impl Sync for Shaders {}

impl Shaders {
    /// Initialize our shader set.
    ///
    /// "post_shaders" is an optional list of postprocess shaders to run
    /// against the final drawable texture. This is an array of shader source
    /// code, not file paths.
    pub fn init<S: AsRef<str>>(
        device: &Device,
        post_shaders: &[S],
        pixel_format: PixelFormat,
    ) -> Result<Shaders, Error> {
        let library = init_library(device)?;

        let make = |desc: &PipelineDesc| -> Result<Pipeline, Error> {
            let opts = pipeline::Options {
                device,
                vertex_fn: desc.vertex_fn,
                fragment_fn: desc.fragment_fn,
                vertex_library: &library,
                fragment_library: &library,
                step_fn: desc.step_fn,
                attachments: &[Attachment {
                    pixel_format,
                    blending_enabled: desc.blending_enabled,
                }],
            };
            match desc.name {
                "cell_text" => Pipeline::init::<CellText>(&opts),
                "image" => Pipeline::init::<ImageVertex>(&opts),
                "bg_image" => Pipeline::init::<BgImage>(&opts),
                _ => Pipeline::init_no_attributes(&opts),
            }
        };
        let [bg_color, cell_bg, cell_text, image, bg_image] = &PIPELINE_DESCS;

        let bg_color = make(bg_color)?;
        let cell_bg = make(cell_bg)?;
        let cell_text = make(cell_text)?;
        let image = make(image)?;
        let bg_image = make(bg_image)?;

        let post_pipelines = init_post_pipelines(device, &library, post_shaders, pixel_format)
            .unwrap_or_else(|err| {
                // If an error happens while building postprocess shaders we
                // want to just not use any postprocess shaders since we don't
                // want to block Ghostty from working.
                log::warn!("error initializing postprocess shaders err={err}");
                Vec::new()
            });

        Ok(Shaders {
            library,
            bg_color,
            cell_bg,
            cell_text,
            image,
            bg_image,
            post_pipelines,
        })
    }

    /// The compiled library of the built-in shaders.
    pub fn library(&self) -> &ProtocolObject<dyn MTLLibrary> {
        &self.library
    }
}

impl GpuShaders<super::Metal> for Shaders {
    fn bg_color(&self) -> &Pipeline {
        &self.bg_color
    }

    fn cell_bg(&self) -> &Pipeline {
        &self.cell_bg
    }

    fn cell_text(&self) -> &Pipeline {
        &self.cell_text
    }

    fn image(&self) -> &Pipeline {
        &self.image
    }

    fn bg_image(&self) -> &Pipeline {
        &self.bg_image
    }

    fn post_pipelines(&self) -> &[Pipeline] {
        &self.post_pipelines
    }
}

/// Initialize the MTLLibrary. A MTLLibrary is a collection of shaders.
fn init_library(device: &Device) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>, Error> {
    let start = Instant::now();

    let source = NSString::from_str(SHADERS_METAL);
    let library = device
        .newLibraryWithSource_options_error(&source, None)
        .map_err(|err| Error::from_ns_error(&err))?;

    log::debug!(
        "shader library loaded time={}us",
        start.elapsed().as_micros()
    );

    Ok(library)
}

/// Initialize our custom shader pipelines.
///
/// The shaders argument is a set of shader source code, not file paths.
fn init_post_pipelines<S: AsRef<str>>(
    device: &Device,
    library: &ProtocolObject<dyn MTLLibrary>,
    shaders: &[S],
    pixel_format: PixelFormat,
) -> Result<Vec<Pipeline>, Error> {
    // If we have no shaders, do nothing. If any error happens the
    // already-built pipelines are dropped (released).
    shaders
        .iter()
        .map(|source| init_post_pipeline(device, library, source.as_ref(), pixel_format))
        .collect()
}

/// Initialize a single custom shader pipeline from shader source.
fn init_post_pipeline(
    device: &Device,
    library: &ProtocolObject<dyn MTLLibrary>,
    data: &str,
    pixel_format: PixelFormat,
) -> Result<Pipeline, Error> {
    // Create our library which has the shader source
    let source = NSString::from_str(data);
    let post_library = device
        .newLibraryWithSource_options_error(&source, None)
        .map_err(|err| Error::from_ns_error(&err))?;

    Pipeline::init_no_attributes(&pipeline::Options {
        device,
        vertex_fn: FULL_SCREEN_VERTEX_FN,
        fragment_fn: POST_FRAGMENT_FN,
        vertex_library: library,
        fragment_library: &post_library,
        step_fn: VertexStepFunction::PerVertex,
        attachments: &[Attachment {
            pixel_format,
            blending_enabled: false,
        }],
    })
}
