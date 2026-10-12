//! Emits the font backend `cfg`s (port of `font/backend.zig`'s `Backend`
//! helpers `hasFreetype`/`hasCoretext`/`hasFontconfig`/`hasHarfbuzz`).
//!
//! Backends (see `src/backend.rs`):
//! * Linux & other non-Apple, non-wasm targets: `fontconfig_freetype`.
//! * Linux & other non-Apple, non-wasm targets with the cargo feature
//!   `font-backend-freetype`: `freetype` (FreeType rendering, HarfBuzz
//!   shaping, no font discovery; Zig `-Dfont-backend=freetype`).
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

    let ft_only = feat("FONT_BACKEND_FREETYPE");
    if ft_only && (wasm || apple) {
        println!(
            "cargo:warning=feature `font-backend-freetype` only applies to non-Apple, \
             non-wasm targets; ignored"
        );
    }
    let ft_only = ft_only && !wasm && !apple;

    let on = |c: &str, v: bool| {
        if v {
            println!("cargo:rustc-cfg={c}");
        }
    };

    let freetype = !wasm && (!apple || ct_ft);
    let harfbuzz = !wasm && (!apple || ct_ft || ct_hb);
    on("font_freetype", freetype);
    let fontconfig = !wasm && !apple && !ft_only;
    on("font_fontconfig", fontconfig);
    on("font_discovery", fontconfig || apple);
    on("font_harfbuzz", harfbuzz);
    on("font_coretext", apple);
    on("font_coretext_face", apple && !ct_ft);
    // The shaper is CoreText only for the plain `coretext` backend;
    // `coretext_noshape` uses the no-op shaper.
    on("font_coretext_shaper", apple && !harfbuzz && !ct_noshape);
    on("font_noshape", apple && !harfbuzz && ct_noshape);
}
