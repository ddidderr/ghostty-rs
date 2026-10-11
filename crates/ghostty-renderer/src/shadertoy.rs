//! Shadertoy-style custom shader support (`renderer/shadertoy.zig`).
//!
//! Ghostty converts user shaders (shadertoy GLSL, which only define a
//! `mainImage` function) into full GLSL for the active renderer backend. The
//! Zig implementation goes GLSL -> glslang -> SPIR-V -> spirv-cross -> target
//! GLSL/MSL. This port is pure Rust: GLSL -> `naga` (glsl-in frontend,
//! validated) -> `naga` glsl-out backend at version 430 core. See
//! `docs/rewrite/notes/ghostty-renderer-R1.md` for the (small) source
//! adaptations needed for naga's Vulkan-flavoured GLSL frontend.

use std::io::Read;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use naga::back::glsl as glsl_out;
use naga::front::glsl as glsl_in;
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};

/// The shadertoy prefix (declares the uniforms and the `main` entry point).
pub const PREFIX: &str = include_str!("../shaders/shadertoy_prefix.glsl");

/// The uniform struct used for shadertoy shaders (std140, 4496 bytes).
///
/// Field order matches `shadertoy.zig`; explicit `pad*` fields reproduce the
/// padding of the Zig `extern struct` with `align(16)` fields.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 3],
    pub time: f32,
    pub time_delta: f32,
    pub frame_rate: f32,
    pub frame: i32,
    pub pad0: [u8; 4],
    pub channel_time: [[f32; 4]; 4],
    pub channel_resolution: [[f32; 4]; 4],
    pub mouse: [f32; 4],
    pub date: [f32; 4],
    pub sample_rate: f32,
    pub pad1: [u8; 12],
    pub current_cursor: [f32; 4],
    pub previous_cursor: [f32; 4],
    pub current_cursor_color: [f32; 4],
    pub previous_cursor_color: [f32; 4],
    pub current_cursor_style: i32,
    pub previous_cursor_style: i32,
    pub cursor_visible: i32,
    pub cursor_change_time: f32,
    pub time_focus: f32,
    pub focus: i32,
    pub pad2: [u8; 8],
    pub palette: [[f32; 4]; 256],
    pub background_color: [f32; 4],
    pub foreground_color: [f32; 4],
    pub cursor_color: [f32; 4],
    pub cursor_text: [f32; 4],
    pub selection_background_color: [f32; 4],
    pub selection_foreground_color: [f32; 4],
}

impl Default for Uniforms {
    fn default() -> Self {
        Zeroable::zeroed()
    }
}

/// The target to load shaders for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Glsl,
    /// Metal is out of scope for the Rust port; converting to it fails with
    /// [`Error::Unsupported`].
    Msl,
}

/// An entry of a `custom-shader` config path list (`RepeatablePath.Value`).
// TODO(integration): replace with ghostty_config's RepeatablePath.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathItem {
    /// A path that may be missing (`?path`); missing files are skipped.
    Optional(String),
    /// A path that must exist.
    Required(String),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The shader source exceeds the 4MB limit.
    #[error("shader file too big")]
    FileTooBig,
    /// GLSL parsing failed (upstream: `error.GlslangFailed`); details are in
    /// the [`ShaderLog`].
    #[error("GLSL compilation failed")]
    GlslangFailed,
    /// The parsed module failed validation (upstream: link failure).
    #[error("shader validation failed")]
    ValidationFailed,
    /// Writing the target GLSL failed (upstream: `error.SpvcFailed`).
    #[error("GLSL generation failed: {0}")]
    SpvcFailed(String),
    #[error("unsupported shader target")]
    Unsupported,
}

/// Retrieve errors from shader compilation (`SpirvLog`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShaderLog {
    pub info: String,
    pub debug: String,
}

/// Load a set of shaders from files and convert them to the target
/// format. The shader order is preserved.
pub fn load_from_files(paths: &[PathItem], target: Target) -> Result<Vec<String>, Error> {
    let mut list = Vec::new();

    for item in paths {
        let (path, optional) = match item {
            PathItem::Optional(p) => (p, true),
            PathItem::Required(p) => (p, false),
        };

        let shader = match load_from_file(Path::new(path), target) {
            Ok(s) => s,
            Err(Error::Io(e)) if optional && e.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(e) => return Err(e),
        };
        log::info!("loaded custom shader path={path}");
        list.push(shader);
    }

    Ok(list)
}

/// Load a single shader from a file and convert it to the target language
/// ready to be used with renderers.
pub fn load_from_file(path: &Path, target: Target) -> Result<String, Error> {
    // Read it all into memory -- we don't expect shaders to be large.
    const MAX: u64 = 4 * 1024 * 1024; // 4MB
    let mut src = Vec::new();
    let file = std::fs::File::open(path)?;
    file.take(MAX + 1).read_to_end(&mut src)?;
    if src.len() as u64 > MAX {
        return Err(Error::FileTooBig);
    }
    let src = String::from_utf8_lossy(&src);

    // Convert to full GLSL, then compile.
    let mut errlog = ShaderLog::default();
    let compiled = match compile_shadertoy(&src, Some(&mut errlog)) {
        Ok(c) => c,
        Err(err) => {
            if !errlog.info.is_empty() || !errlog.debug.is_empty() {
                log::warn!(
                    "spirv error path={} info={} debug={}",
                    path.display(),
                    errlog.info,
                    errlog.debug
                );
            }
            return Err(err);
        }
    };

    match target {
        Target::Glsl => glsl_from_compiled(&compiled),
        Target::Msl => Err(Error::Unsupported),
    }
}

/// Convert a ShaderToy shader into valid GLSL.
///
/// ShaderToy shaders aren't full shaders, they're just implementing a
/// mainImage function and don't define any of the uniforms. This function
/// will convert the ShaderToy shader into a valid GLSL shader that can be
/// compiled and linked.
pub fn glsl_from_shader(src: &str) -> String {
    let mut out = String::with_capacity(PREFIX.len() + src.len() + 2);
    out.push_str(PREFIX);
    out.push_str("\n\n");
    out.push_str(src);
    out
}

/// A parsed and validated shader module, ready to be converted to target GLSL.
pub struct Compiled {
    module: naga::Module,
    info: ModuleInfo,
}

/// Compile a ShaderToy shader (just the user's source, without the prefix)
/// to a validated module. Equivalent to `glslFromShader` + `spirvFromGlsl`.
pub fn compile_shadertoy(src: &str, errlog: Option<&mut ShaderLog>) -> Result<Compiled, Error> {
    spirv_from_glsl(&glsl_from_shader(src), errlog)
}

/// Compile full fragment-shader GLSL (as produced by [`glsl_from_shader`])
/// into a validated module (`spirvFromGlsl`; the SPIR-V is naga's IR).
///
/// The prefix is written for glslang's Vulkan GLSL. naga's GLSL frontend
/// accepts a stricter subset, so the well known prefix constructs are adapted
/// first (see [`adapt_for_naga`]).
pub fn spirv_from_glsl(src: &str, errlog: Option<&mut ShaderLog>) -> Result<Compiled, Error> {
    let adapted = adapt_for_naga(src);

    let mut frontend = glsl_in::Frontend::default();
    let module = match frontend.parse(
        &glsl_in::Options::from(naga::ShaderStage::Fragment),
        &adapted,
    ) {
        Ok(m) => m,
        Err(e) => {
            if let Some(log) = errlog {
                log.info = e.emit_to_string(&adapted);
                log.debug = String::new();
            }
            return Err(Error::GlslangFailed);
        }
    };

    let info = match Validator::new(ValidationFlags::all(), Capabilities::all()).validate(&module) {
        Ok(i) => i,
        Err(e) => {
            if let Some(log) = errlog {
                log.info = e.emit_to_string(&adapted);
                log.debug = format!("{e:?}");
            }
            return Err(Error::ValidationFailed);
        }
    };

    Ok(Compiled { module, info })
}

/// Rewrites the (verbatim Ghostty) shadertoy prefix constructs that naga's
/// GLSL frontend cannot parse. Source text outside the prefix is untouched
/// except that `main` is moved after the user code.
///
/// * `#version 430 core` -> `#version 460 core` (naga accepts 440/450/460).
/// * `uniform` qualifiers on interface block members are dropped.
/// * `layout(location = 0) in vec4 gl_FragCoord;` is dropped (naga knows the
///   builtin).
/// * the combined `sampler2D iChannel0` becomes a texture + sampler pair and
///   `iChannel0` a macro that combines them, so `texture(iChannel0, uv)`
///   keeps working.
/// * the `mainImage` prototype and `main` are moved after the user code
///   (naga requires callee definitions before their callers).
fn adapt_for_naga(full: &str) -> String {
    let mut s = full
        .replacen("#version 430 core", "#version 460 core", 1)
        .replace("    uniform ", "    ")
        .replacen("layout(location = 0) in vec4 gl_FragCoord;", "", 1)
        .replacen(
            "layout(binding = 0) uniform sampler2D iChannel0;",
            "layout(set = 0, binding = 0) uniform texture2D iChannel0_tex;\n\
             layout(set = 0, binding = 2) uniform sampler iChannel0_smp;\n\
             #define iChannel0 sampler2D(iChannel0_tex, iChannel0_smp)",
            1,
        )
        .replacen(
            "layout(binding = 1, std140)",
            "layout(set = 0, binding = 1, std140)",
            1,
        )
        .replacen(
            "void mainImage( out vec4 fragColor, in vec2 fragCoord );\n",
            "",
            1,
        )
        .replacen(
            "void main() { mainImage (_fragColor, gl_FragCoord.xy); }",
            "",
            1,
        );
    s.push_str("\nvoid main() { mainImage (_fragColor, gl_FragCoord.xy); }\n");
    s
}

/// Convert a compiled shader to GLSL 430 core (`glslFromSpv`).
pub fn glsl_from_compiled(compiled: &Compiled) -> Result<String, Error> {
    const GLSL_VERSION: u16 = 430;

    let mut options = glsl_out::Options {
        version: glsl_out::Version::Desktop(GLSL_VERSION),
        writer_flags: glsl_out::WriterFlags::empty(),
        ..Default::default()
    };
    // Map the (set 0) resources to the GL binding points used by the
    // renderer: iChannel0 texture at unit 0, the Globals block at binding 1.
    for (binding, slot) in [(0u32, 0u8), (1, 1), (2, 0)] {
        options
            .binding_map
            .insert(naga::ResourceBinding { group: 0, binding }, slot);
    }
    let pipeline = glsl_out::PipelineOptions {
        shader_stage: naga::ShaderStage::Fragment,
        entry_point: "main".into(),
        multiview: None,
    };

    let mut out = String::new();
    let mut writer = glsl_out::Writer::new(
        &mut out,
        &compiled.module,
        &compiled.info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .map_err(|e| {
        log::warn!("spirv-cross error message={e}");
        Error::SpvcFailed(e.to_string())
    })?;
    writer.write().map_err(|e| {
        log::warn!("spirv-cross error message={e}");
        Error::SpvcFailed(e.to_string())
    })?;
    Ok(out)
}

const TEST_CRT: &str = include_str!("../shaders/test_shadertoy_crt.glsl");
const TEST_FOCUS: &str = include_str!("../shaders/test_shadertoy_focus.glsl");
const TEST_INVALID: &str = include_str!("../shaders/test_shadertoy_invalid.glsl");

/// Test shader sources, for use in GL integration tests.
#[doc(hidden)]
pub mod test_shaders {
    pub const CRT: &str = super::TEST_CRT;
    pub const FOCUS: &str = super::TEST_FOCUS;
    pub const INVALID: &str = super::TEST_INVALID;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn spirv() {
        let src = glsl_from_shader(TEST_CRT);
        spirv_from_glsl(&src, None).expect("crt shader compiles");
    }

    #[test]
    fn spirv_invalid() {
        let src = glsl_from_shader(TEST_INVALID);
        let mut errlog = ShaderLog::default();
        let err = spirv_from_glsl(&src, Some(&mut errlog))
            .err()
            .expect("invalid shader fails");
        assert!(matches!(err, Error::GlslangFailed));
        assert_ne!(errlog.info, "");
    }

    #[test]
    fn shadertoy_to_glsl() {
        let src = glsl_from_shader(TEST_CRT);
        let compiled = spirv_from_glsl(&src, None).unwrap();
        let glsl = glsl_from_compiled(&compiled).unwrap();
        assert!(glsl.starts_with("#version 430 core"));
        assert!(glsl.contains("layout(std140, binding = 1) uniform"));
        assert!(glsl.contains("layout(binding = 0) uniform sampler2D"));
    }

    #[test]
    fn shadertoy_to_msl_unsupported() {
        let compiled = compile_shadertoy(TEST_CRT, None).unwrap();
        assert_eq!(compiled.module.entry_points.len(), 1);
        let dir = std::env::temp_dir().join(format!("ghostty-shadertoy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("crt.glsl");
        std::fs::write(&path, TEST_CRT).unwrap();
        assert!(matches!(
            load_from_file(&path, Target::Msl),
            Err(Error::Unsupported)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shadertoy_focus_to_glsl() {
        let compiled = compile_shadertoy(TEST_FOCUS, None).unwrap();
        let glsl = glsl_from_compiled(&compiled).unwrap();
        assert!(glsl.contains("iTimeFocus") || glsl.contains("_group_0_binding_1_fs"));
    }

    #[test]
    fn glsl_from_shader_is_prefix_plus_source() {
        let out = glsl_from_shader("void mainImage(out vec4 c, in vec2 p) { c = vec4(0.0); }");
        assert!(out.starts_with(PREFIX));
        assert!(out.contains(&format!("{PREFIX}\n\nvoid mainImage")));
    }

    #[test]
    fn user_helpers_after_main_image_compile() {
        let src = "void mainImage(out vec4 c, in vec2 p) { c = vec4(helper(p), 0.0, 0.0, 1.0); }\n\
                   float helper(vec2 p) { return p.x; }\n";
        // helper is defined after use: invalid GLSL, must be rejected, not panic.
        assert!(compile_shadertoy(src, None).is_err());
        let src = "float helper(vec2 p) { return p.x / iResolution.x; }\n\
                   void mainImage(out vec4 c, in vec2 p) { c = vec4(helper(p), texture2D(iChannel0, p / iResolution.xy).g, iTime, 1.0); }\n";
        compile_shadertoy(src, None).unwrap();
    }

    #[test]
    fn load_from_files_skips_missing_optional() {
        let dir =
            std::env::temp_dir().join(format!("ghostty-shadertoy-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("crt.glsl");
        std::fs::write(&good, TEST_CRT).unwrap();
        let missing = dir.join("missing.glsl");

        let items = [
            PathItem::Optional(missing.to_string_lossy().into_owned()),
            PathItem::Required(good.to_string_lossy().into_owned()),
        ];
        let shaders = load_from_files(&items, Target::Glsl).unwrap();
        assert_eq!(shaders.len(), 1);

        let items = [PathItem::Required(missing.to_string_lossy().into_owned())];
        assert!(matches!(
            load_from_files(&items, Target::Glsl),
            Err(Error::Io(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn uniforms_std140_layout() {
        // std140 offsets of the `Globals` block in shadertoy_prefix.glsl.
        assert_eq!(offset_of!(Uniforms, resolution), 0);
        assert_eq!(offset_of!(Uniforms, time), 12);
        assert_eq!(offset_of!(Uniforms, time_delta), 16);
        assert_eq!(offset_of!(Uniforms, frame_rate), 20);
        assert_eq!(offset_of!(Uniforms, frame), 24);
        assert_eq!(offset_of!(Uniforms, channel_time), 32);
        assert_eq!(offset_of!(Uniforms, channel_resolution), 96);
        assert_eq!(offset_of!(Uniforms, mouse), 160);
        assert_eq!(offset_of!(Uniforms, date), 176);
        assert_eq!(offset_of!(Uniforms, sample_rate), 192);
        assert_eq!(offset_of!(Uniforms, current_cursor), 208);
        assert_eq!(offset_of!(Uniforms, previous_cursor), 224);
        assert_eq!(offset_of!(Uniforms, current_cursor_color), 240);
        assert_eq!(offset_of!(Uniforms, previous_cursor_color), 256);
        assert_eq!(offset_of!(Uniforms, current_cursor_style), 272);
        assert_eq!(offset_of!(Uniforms, previous_cursor_style), 276);
        assert_eq!(offset_of!(Uniforms, cursor_visible), 280);
        assert_eq!(offset_of!(Uniforms, cursor_change_time), 284);
        assert_eq!(offset_of!(Uniforms, time_focus), 288);
        assert_eq!(offset_of!(Uniforms, focus), 292);
        assert_eq!(offset_of!(Uniforms, palette), 304);
        assert_eq!(offset_of!(Uniforms, background_color), 4400);
        assert_eq!(offset_of!(Uniforms, foreground_color), 4416);
        assert_eq!(offset_of!(Uniforms, cursor_color), 4432);
        assert_eq!(offset_of!(Uniforms, cursor_text), 4448);
        assert_eq!(offset_of!(Uniforms, selection_background_color), 4464);
        assert_eq!(offset_of!(Uniforms, selection_foreground_color), 4480);
        assert_eq!(size_of::<Uniforms>(), 4496);
    }
}
