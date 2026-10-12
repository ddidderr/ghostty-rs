//! Emits the font backend `cfg`s (port of `font/backend.zig`'s `Backend`
//! helpers `hasFreetype`/`hasCoretext`/`hasFontconfig`/`hasHarfbuzz`).
//!
//! Backends (see `src/backend.rs`):
//! * Linux & other non-Apple, non-wasm targets: `fontconfig_freetype`.
//! * Any non-wasm target (Apple included) with the cargo feature
//!   `font-backend-freetype`: `freetype` (FreeType rendering, HarfBuzz
//!   shaping, no font discovery; Zig `-Dfont-backend=freetype`). On Apple
//!   it takes precedence over the `coretext-*` features.
//! * wasm32: `web_canvas`.
//! * Apple targets: `coretext` (default), or via cargo features
//!   `coretext-freetype`, `coretext-harfbuzz`, `coretext-noshape`.
//!
//! Emitted cfgs:
//! * `font_freetype`   - FreeType face / library are compiled.
//! * `font_fontconfig` - fontconfig discovery is compiled.
//! * `font_discovery`  - a discovery mechanism (fontconfig or CoreText) exists
//!   (Zig: `font.Discover != void`).
//! * `font_harfbuzz`   - the HarfBuzz shaper is compiled.
//! * `font_coretext`   - CoreText discovery/deferred face (Apple targets).
//! * `font_coretext_face` - the CoreText `Face` is the font face.
//! * `font_coretext_shaper` - the CoreText shaper is the shaper.
//! * `font_noshape`    - the no-op shaper is the shaper.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    build_harfbuzz_freetype_glue();
    for c in [
        "font_freetype",
        "font_fontconfig",
        "font_discovery",
        "font_harfbuzz",
        "font_coretext",
        "font_coretext_face",
        "font_coretext_shaper",
        "font_noshape",
    ] {
        println!("cargo:rustc-check-cfg=cfg({c})");
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();
    let feat = |n: &str| std::env::var_os(format!("CARGO_FEATURE_{n}")).is_some();

    let wasm = arch == "wasm32";
    let apple = vendor == "apple";
    let ct_ft = feat("CORETEXT_FREETYPE");
    let ct_hb = feat("CORETEXT_HARFBUZZ");
    let ct_noshape = feat("CORETEXT_NOSHAPE");

    // Zig `-Dfont-backend=freetype` is valid on every non-wasm target,
    // Darwin included (no discovery, so the embedded emoji fonts are used).
    let ft_only = feat("FONT_BACKEND_FREETYPE");
    if ft_only && wasm {
        println!("cargo:warning=feature `font-backend-freetype` does not apply to wasm32; ignored");
    }
    let ft_only = ft_only && !wasm;
    // CoreText is the backend on Apple targets unless FreeType-only is asked.
    let coretext = apple && !ft_only;

    let on = |c: &str, v: bool| {
        if v {
            println!("cargo:rustc-cfg={c}");
        }
    };

    let freetype = !wasm && (!coretext || ct_ft);
    let harfbuzz = !wasm && (!coretext || ct_ft || ct_hb);
    on("font_freetype", freetype);
    let fontconfig = !wasm && !apple && !ft_only;
    on("font_fontconfig", fontconfig);
    on("font_discovery", fontconfig || coretext);
    on("font_harfbuzz", harfbuzz);
    on("font_coretext", coretext);
    on("font_coretext_face", coretext && !ct_ft);
    // The shaper is CoreText only for the plain `coretext` backend;
    // `coretext_noshape` uses the no-op shaper.
    on("font_coretext_shaper", coretext && !harfbuzz && !ct_noshape);
    on("font_noshape", coretext && !harfbuzz && ct_noshape);
}

/// The bundled HarfBuzz (`harfbuzz-sys` `bundled`, used on Windows where there
/// is no system copy) is compiled without `HAVE_FREETYPE`, so `hb_ft_*` is
/// missing. Zig builds HarfBuzz with FreeType enabled (`pkg/harfbuzz`
/// `-DHAVE_FREETYPE=1`), so compile `hb-ft.cc` against the bundled FreeType
/// headers here.
fn build_harfbuzz_freetype_glue() {
    use std::path::PathBuf;
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(hb_src) = std::env::var_os("DEP_HARFBUZZ_INCLUDE").map(PathBuf::from) else {
        return;
    };
    // `<registry>/harfbuzz-sys-X/harfbuzz/src` -> `<registry>/freetype-sys-*/freetype2/include`.
    let Some(registry) = hb_src.ancestors().nth(3) else {
        return;
    };
    let ft_include = std::fs::read_dir(registry)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("freetype-sys-"))
        })
        .map(|p| p.join("freetype2").join("include"))
        .find(|p| p.join("ft2build.h").exists());
    let Some(ft_include) = ft_include else {
        println!("cargo:warning=bundled FreeType headers not found; hb_ft_* will be missing");
        return;
    };
    let mut cfg = cc::Build::new();
    cfg.cpp(true)
        .flag_if_supported("-std=c++11")
        .warnings(false)
        .define("HAVE_FREETYPE", "1")
        .include(&hb_src)
        .include(&ft_include)
        .file(hb_src.join("hb-ft.cc"));
    if std::env::var("TARGET").is_ok_and(|t| t.contains("windows-gnu")) {
        cfg.flag("-Wa,-mbig-obj");
    }
    cfg.compile("ghostty_hb_ft");
}
