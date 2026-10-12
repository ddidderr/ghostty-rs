//! Shadertoy-style custom shader support (`renderer/shadertoy.zig`).
//!
//! Ghostty converts user shaders (shadertoy GLSL, which only define a
//! `mainImage` function) into full GLSL for the active renderer backend. The
//! Zig implementation goes GLSL -> glslang -> SPIR-V -> spirv-cross -> target
//! GLSL/MSL. This port is pure Rust: GLSL -> `naga` (glsl-in frontend,
//! validated) -> `naga` glsl-out backend at version 430 core. See
//! `docs/rewrite/notes/ghostty-renderer-R1.md` for the (small) source
//! adaptations needed for naga's Vulkan-flavoured GLSL frontend.

use std::{io::Read, path::Path};

use bytemuck::{Pod, Zeroable};
use naga::{
    back::{glsl as glsl_out, msl as msl_out},
    front::glsl as glsl_in,
    valid::{Capabilities, ModuleInfo, ValidationFlags, Validator},
};

use crate::shadertoy_sampler;

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
    /// Metal Shading Language (naga's `msl-out` replaces spirv-cross).
    Msl,
}

/// An entry of a `custom-shader` config path list (`RepeatablePath.Value`).
// TODO(integration): replace with ghostty_config's RepeatablePath.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathItem {
    /// A path that may be missing (`?path`); missing files are skipped.
    Optional(std::path::PathBuf),
    /// A path that must exist.
    Required(std::path::PathBuf),
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
    #[error("shader generation failed: {0}")]
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

        let shader = match load_from_file(path, target) {
            Ok(s) => s,
            Err(Error::Io(e)) if optional && e.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(e) => return Err(e),
        };
        log::info!("loaded custom shader path={}", path.display());
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
        Target::Msl => msl_from_compiled(&compiled),
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
    // Only the verbatim prefix gets the constructs below; the user's code is
    // only touched by the combined-sampler rewrite.
    let (prefix, user) = match full.strip_prefix(PREFIX) {
        Some(rest) => (PREFIX, rest),
        None => (full, ""),
    };
    let mut s = prefix
        .replacen("#version 430 core", "#version 460 core", 1)
        .replace("    uniform ", "    ")
        .replacen("layout(location = 0) in vec4 gl_FragCoord;", "", 1)
        // std140 pads `float iChannelTime[4]` to stride 16, which naga types
        // differently from a plain local `float[4]`, so whole-array loads and
        // copies (`float t[4] = iChannelTime;`, passing it to a function) do
        // not validate. The block member is renamed and `iChannelTime` becomes
        // a function that reconstructs the plain array from the padded
        // elements; element reads (`iChannelTime[i]`) keep working.
        .replacen(
            "    float iChannelTime[4];\n",
            "    float iChannelTimeUbo[4];\n",
            1,
        )
        .replacen(
            "\n#define CURSORSTYLE_BLOCK ",
            "\nfloat[4] iChannelTime_plain() {\n    return float[4](iChannelTimeUbo[0], iChannelTimeUbo[1], iChannelTimeUbo[2], iChannelTimeUbo[3]);\n}\n#define iChannelTime iChannelTime_plain()\n\n#define CURSORSTYLE_BLOCK ",
            1,
        )
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
    if !user.is_empty() {
        // `texture2D` is a type for naga (and the `sampler2D` parameter
        // rewrite): the prefix macro is replaced by rewriting the calls.
        s = s.replacen("#define texture2D texture", "", 1);
        s.push_str(&shadertoy_sampler::rewrite(user));
    }
    s.push_str("\nvoid main() { mainImage (_fragColor, gl_FragCoord.xy); }\n");
    s
}

/// The name of the fragment entry point in generated MSL. spirv-cross calls
/// it `main0` because `main` is reserved in MSL; naga would turn `main0` into
/// `main0_` (it suffixes names ending in a digit), so we use a name it keeps
/// as is. The Metal renderer looks the function up by this name.
pub const MSL_ENTRY_POINT: &str = "ghostty_main";

/// Convert a compiled shader to MSL (`mslFromSpv`).
///
/// The resource slots follow what spirv-cross produces with
/// `SPVC_COMPILER_OPTION_MSL_ENABLE_DECORATION_BINDING` for the shadertoy
/// prefix: the `Globals` uniform block at `[[buffer(1)]]` ("we need this to
/// properly locate the uniform block to index 1") and `iChannel0` as
/// `[[texture(0)]]` / `[[sampler(0)]]`, which is where the Metal render pass
/// binds them.
pub fn msl_from_compiled(compiled: &Compiled) -> Result<String, Error> {
    let fail = |e: &dyn std::fmt::Display| {
        log::warn!("spirv-cross error message={e}");
        Error::SpvcFailed(e.to_string())
    };

    // Rename the entry point (info is indexed by entry point order, which
    // doesn't change).
    let mut module = compiled.module.clone();
    for ep in &mut module.entry_points {
        if ep.name == "main" {
            ep.name = MSL_ENTRY_POINT.into();
        }
    }

    let mut resources = msl_out::EntryPointResources::default();
    let mut bind = |binding: u32, target: msl_out::BindTarget| {
        resources
            .resources
            .insert(naga::ResourceBinding { group: 0, binding }, target);
    };
    // iChannel0 texture (the sampler is split off at binding 2, see
    // `adapt_for_naga`).
    bind(
        0,
        msl_out::BindTarget {
            texture: Some(0),
            ..Default::default()
        },
    );
    bind(
        1,
        msl_out::BindTarget {
            buffer: Some(1),
            ..Default::default()
        },
    );
    bind(
        2,
        msl_out::BindTarget {
            sampler: Some(msl_out::BindSamplerTarget::Resource(0)),
            ..Default::default()
        },
    );

    let mut options = msl_out::Options {
        // Needed for `[[position]]` fragment inputs etc. on all supported
        // macOS versions (the metallib is built for macOS 10.14 / MSL 2.1).
        lang_version: (2, 1),
        fake_missing_bindings: false,
        ..Default::default()
    };
    options
        .per_entry_point_map
        .insert(MSL_ENTRY_POINT.into(), resources);

    let pipeline = msl_out::PipelineOptions {
        entry_point: Some((naga::ShaderStage::Fragment, MSL_ENTRY_POINT.into())),
        ..Default::default()
    };

    let (msl, _info) = msl_out::write_string(&module, &compiled.info, &options, &pipeline)
        .map_err(|e| fail(&e))?;
    Ok(fix_uniform_array_strides(&module, msl))
}

/// A uniform-block member that is an array of scalars. std140 gives it a
/// 16-byte element stride (what we upload and what spirv-cross emits padded
/// types for), but naga's MSL writer ignores the array `stride` and emits a
/// tightly packed `float[N]`.
struct StridedArray {
    member: String,
    /// naga's MSL name of the array wrapper struct (`type_<handle index>`).
    type_name: String,
    /// MSL scalar element type.
    scalar: &'static str,
    len: u32,
    /// Lanes of the padded element (stride / scalar size).
    lanes: u32,
}

fn strided_uniform_arrays(module: &naga::Module) -> Vec<StridedArray> {
    let mut out = Vec::new();
    for (_, gv) in module.global_variables.iter() {
        if gv.space != naga::AddressSpace::Uniform {
            continue;
        }
        let naga::TypeInner::Struct { members, .. } = &module.types[gv.ty].inner else {
            continue;
        };
        for m in members {
            let naga::TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(n),
                stride,
            } = &module.types[m.ty].inner
            else {
                continue;
            };
            let naga::TypeInner::Scalar(sc) = &module.types[*base].inner else {
                continue;
            };
            let scalar = match (sc.kind, sc.width) {
                (naga::ScalarKind::Float, 4) => "float",
                (naga::ScalarKind::Sint, 4) => "int",
                (naga::ScalarKind::Uint, 4) => "uint",
                _ => continue,
            };
            let size = u32::from(sc.width);
            if *stride <= size {
                continue;
            }
            let (Some(name), true) = (m.name.clone(), stride % size == 0) else {
                continue;
            };
            out.push(StridedArray {
                member: name,
                type_name: format!("type_{}", m.ty.index()),
                scalar,
                len: n.get(),
                lanes: stride / size,
            });
        }
    }
    out
}

/// Make the MSL uniform block match the std140 layout we upload: scalar
/// arrays become arrays of `vecN` (stride 16 for std140) and element reads
/// take `.x`. The writer ignores array strides, so this is done on the text.
fn fix_uniform_array_strides(module: &naga::Module, mut msl: String) -> String {
    for a in strided_uniform_arrays(module) {
        let wrapper = format!("{}_std140", a.type_name);
        // 1. The block member uses the padded wrapper.
        let decl = format!("    {} {};", a.type_name, a.member);
        if !msl.contains(&decl) {
            continue;
        }
        msl = msl.replace(&decl, &format!("    {wrapper} {};", a.member));
        // 2. Define the wrapper before the struct that uses it.
        let def = format!(
            "struct {wrapper} {{\n    metal::vec<{}, {}> inner[{}];\n}};\n",
            a.scalar, a.lanes, a.len
        );
        if let Some(at) = msl.find(&format!("\n    {wrapper} {};", a.member))
            && let Some(start) = msl[..at].rfind("\nstruct ")
        {
            msl.insert_str(start + 1, &def);
        }
        // 3. Element reads: `<expr>.member.inner[idx]` -> `... [idx].x`.
        let needle = format!(".{}.inner[", a.member);
        let mut from = 0;
        while let Some(rel) = msl[from..].find(&needle) {
            let open = from + rel + needle.len() - 1;
            let mut depth = 0usize;
            let mut close = None;
            for (k, ch) in msl[open..].char_indices() {
                match ch {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(open + k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else { break };
            msl.insert_str(close + 1, ".x");
            from = close + 3;
        }
    }
    msl
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
    use core::mem::{offset_of, size_of};

    use super::*;

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
    fn shadertoy_to_msl() {
        // `shadertoy to msl`
        let compiled = compile_shadertoy(TEST_CRT, None).unwrap();
        assert_eq!(compiled.module.entry_points.len(), 1);
        let msl = msl_from_compiled(&compiled).unwrap();
        assert!(msl.contains("#include <metal_stdlib>"));
        // The Metal renderer looks the fragment function up by this name.
        assert!(
            msl.contains("fragment ghostty_mainOutput ghostty_main("),
            "{msl}"
        );
        // Uniform block at buffer 1, channel at texture 0 / sampler 0.
        assert!(msl.contains("[[buffer(1)]]"), "{msl}");
        assert!(msl.contains("[[texture(0)]]"), "{msl}");
        assert!(msl.contains("[[sampler(0)]]"), "{msl}");
    }

    #[test]
    fn shadertoy_focus_to_msl() {
        let compiled = compile_shadertoy(TEST_FOCUS, None).unwrap();
        let msl = msl_from_compiled(&compiled).unwrap();
        assert!(msl.contains("ghostty_main("));
    }

    #[test]
    fn load_from_file_msl() {
        let dir = std::env::temp_dir().join(format!("ghostty-shadertoy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("crt.glsl");
        std::fs::write(&path, TEST_CRT).unwrap();
        let msl = load_from_file(&path, Target::Msl).unwrap();
        assert!(msl.contains("ghostty_main("));
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
    fn sampler2d_parameter_helper_compiles() {
        let src = "vec4 fetch(sampler2D tex, vec2 uv) { return texture(tex, uv); }\n\
                   void mainImage(out vec4 c, in vec2 p) { c = fetch(iChannel0, p / iResolution.xy); }\n";
        let mut log = ShaderLog::default();
        let r = compile_shadertoy(src, Some(&mut log));
        assert!(r.is_ok(), "{}", log.info);
    }

    fn compiles(src: &str) {
        let mut log = ShaderLog::default();
        let c = compile_shadertoy(src, Some(&mut log));
        assert!(c.is_ok(), "{}\n{}", log.info, log.debug);
        glsl_from_compiled(&c.unwrap()).unwrap();
    }

    #[test]
    fn sampler2d_parameter_forwarding_and_prototype() {
        compiles(
            "vec4 inner(in sampler2D s, vec2 uv);\n\
             vec4 outer(sampler2D t, vec2 uv) { return inner(t, uv) + inner(t, uv * 2.0); }\n\
             vec4 inner(in sampler2D s, vec2 uv) { return texture(s, uv); }\n\
             void mainImage(out vec4 c, in vec2 p) { c = outer(iChannel0, p / iResolution.xy); }\n",
        );
    }

    #[test]
    fn sampler2d_parameter_other_texture_functions() {
        compiles(
            "vec4 f(sampler2D s, vec2 uv) {\n\
                 vec2 sz = vec2(textureSize(s, 0));\n\
                 vec4 a = textureLod(s, uv, 0.0);\n\
                 vec4 b = texelFetch(s, ivec2(uv * sz), 0);\n\
                 return a + b + texture2D(s, uv);\n\
             }\n\
             void mainImage(out vec4 c, in vec2 p) { c = f(iChannel0, p / iResolution.xy); }\n",
        );
    }

    #[test]
    fn combined_sampler_builtin_uses_compile() {
        compiles(
            "void mainImage(out vec4 c, in vec2 p) {\n\
                 vec2 uv = p / iResolution.xy;\n\
                 c = textureLod(iChannel0, uv, 0.0) + texelFetch(iChannel0, ivec2(p), 0)\n\
                   + vec4(textureSize(iChannel0, 0), 0.0, 0.0) + texture2D(iChannel0, uv);\n\
             }\n",
        );
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
            PathItem::Optional(missing.clone()),
            PathItem::Required(good.clone()),
        ];
        let shaders = load_from_files(&items, Target::Glsl).unwrap();
        assert_eq!(shaders.len(), 1);

        let items = [PathItem::Required(missing.clone())];
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

    /// (size, align) of an MSL type as emitted by naga for the shadertoy
    /// uniform block, resolving the wrapper structs defined in `msl`.
    fn msl_size_align(msl: &str, ty: &str) -> (usize, usize) {
        match ty {
            "float" | "int" | "uint" => (4, 4),
            "metal::packed_float3" => (12, 4),
            "metal::float2" => (8, 8),
            "metal::float3" | "metal::float4" => (16, 16),
            _ => {
                if let Some(rest) = ty.strip_prefix("metal::vec<") {
                    let (scalar, n) = rest.trim_end_matches('>').split_once(", ").unwrap();
                    let n: usize = n.parse().unwrap();
                    assert!(matches!(scalar, "float" | "int" | "uint"));
                    let n = if n == 3 { 4 } else { n };
                    return (4 * n, 4 * n);
                }
                // A wrapper struct `struct <ty> { <elem> inner[N]; };`.
                let head = format!("struct {ty} {{\n    ");
                let at = msl.find(&head).unwrap_or_else(|| panic!("no struct {ty}"));
                let body = &msl[at + head.len()..];
                let line = &body[..body.find(';').unwrap()];
                let (elem, count) = line.rsplit_once(" inner[").unwrap();
                let count: usize = count.trim_end_matches(']').parse().unwrap();
                let (es, ea) = msl_size_align(msl, elem);
                (es * count, ea)
            }
        }
    }

    /// The `(offset, name)` of each member of `struct Globals` in `msl`,
    /// skipping `char _padN[k]` fillers, plus the struct size.
    fn msl_globals_layout(msl: &str) -> (Vec<(usize, String)>, usize) {
        let at = msl.find("struct Globals {\n").unwrap();
        let body = &msl[at + "struct Globals {\n".len()..];
        let body = &body[..body.find("\n};").unwrap()];
        let mut off = 0usize;
        let mut max_align = 1;
        let mut members = Vec::new();
        for line in body.lines() {
            let line = line.trim().trim_end_matches(';');
            let (ty, name) = line.rsplit_once(' ').unwrap();
            if ty == "char" {
                let n: usize = name
                    .split('[')
                    .nth(1)
                    .unwrap()
                    .trim_end_matches(']')
                    .parse()
                    .unwrap();
                off += n;
                continue;
            }
            let (size, align) = msl_size_align(msl, ty);
            off = off.next_multiple_of(align);
            max_align = max_align.max(align);
            members.push((off, name.to_owned()));
            off += size;
        }
        (members, off.next_multiple_of(max_align))
    }

    #[test]
    fn msl_uniform_block_matches_std140_upload_layout() {
        // A shader that reads the scalar array so the accessor path is used.
        check_msl_layout(
            "void mainImage(out vec4 c, in vec2 p) { \
            int i = int(p.x) & 3; c = vec4(iChannelTime[i], iChannelTime[2], iTime, iSampleRate); }",
        );
    }

    #[test]
    fn whole_array_copy_of_padded_uniform_validates_and_keeps_layout() {
        // Whole-array loads/copies of the std140-padded `iChannelTime` must
        // validate (naga types the padded array differently from a plain
        // local `float[4]`) and not disturb the MSL block layout.
        let user = "float sum4(float a[4]) { return a[0] + a[1] + a[2] + a[3]; }\n\
            void mainImage(out vec4 c, in vec2 p) { \
            float t[4] = iChannelTime; float u[4]; u = iChannelTime; \
            c = vec4(t[1] + u[2], sum4(iChannelTime), iChannelTime[3], iTime); }";
        let msl = check_msl_layout(user);
        // The block member keeps the padded wrapper and is read per element.
        assert!(msl.contains(".iChannelTimeUbo.inner[0].x"), "{msl}");
        // GLSL output (OpenGL) validates too.
        let compiled = compile_shadertoy(user, None).unwrap();
        glsl_from_compiled(&compiled).unwrap();
    }

    fn check_msl_layout(user: &str) -> String {
        use std::mem::offset_of as o;
        let compiled = compile_shadertoy(user, None).unwrap();
        let msl = msl_from_compiled(&compiled).unwrap();
        let (members, size) = msl_globals_layout(&msl);

        let expect = [
            ("iResolution", o!(Uniforms, resolution)),
            ("iTime", o!(Uniforms, time)),
            ("iTimeDelta", o!(Uniforms, time_delta)),
            ("iFrameRate", o!(Uniforms, frame_rate)),
            ("iFrame", o!(Uniforms, frame)),
            ("iChannelTimeUbo", o!(Uniforms, channel_time)),
            ("iChannelResolution", o!(Uniforms, channel_resolution)),
            ("iMouse", o!(Uniforms, mouse)),
            ("iDate", o!(Uniforms, date)),
            ("iSampleRate", o!(Uniforms, sample_rate)),
            ("iCurrentCursor", o!(Uniforms, current_cursor)),
            ("iPreviousCursor", o!(Uniforms, previous_cursor)),
            ("iCurrentCursorColor", o!(Uniforms, current_cursor_color)),
            ("iPreviousCursorColor", o!(Uniforms, previous_cursor_color)),
            ("iCurrentCursorStyle", o!(Uniforms, current_cursor_style)),
            ("iPreviousCursorStyle", o!(Uniforms, previous_cursor_style)),
            ("iCursorVisible", o!(Uniforms, cursor_visible)),
            ("iTimeCursorChange", o!(Uniforms, cursor_change_time)),
            ("iTimeFocus", o!(Uniforms, time_focus)),
            ("iFocus", o!(Uniforms, focus)),
            ("iPalette", o!(Uniforms, palette)),
            ("iBackgroundColor", o!(Uniforms, background_color)),
            ("iForegroundColor", o!(Uniforms, foreground_color)),
            ("iCursorColor", o!(Uniforms, cursor_color)),
            ("iCursorText", o!(Uniforms, cursor_text)),
            // Upstream quirk: the Zig struct (and so ours) has the selection
            // colors swapped relative to the GLSL block; both are vec3 so the
            // offsets agree.
            (
                "iSelectionForegroundColor",
                o!(Uniforms, selection_background_color),
            ),
            (
                "iSelectionBackgroundColor",
                o!(Uniforms, selection_foreground_color),
            ),
        ];
        assert_eq!(members.len(), expect.len(), "{msl}");
        for ((off, name), (ename, eoff)) in members.iter().zip(expect) {
            assert_eq!(name, ename);
            assert_eq!(*off, eoff, "MSL offset of {name}\n{msl}");
        }
        assert_eq!(size, std::mem::size_of::<Uniforms>());
        // Reads of the padded array take the first lane.
        assert!(msl.contains(".iChannelTimeUbo.inner["), "{msl}");
        assert!(msl.contains("].x"), "{msl}");
        msl
    }
}
