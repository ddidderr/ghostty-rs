//! Font backend selection. Port of `font/backend.zig`.
//!
//! [`OPTIONS`] selects the backend for the target (`build.rs` emits the
//! matching `cfg`s):
//!
//! * Linux and other non-Apple, non-wasm targets:
//!   [`Backend::FontconfigFreetype`] (fontconfig discovery, FreeType
//!   rendering, HarfBuzz shaping).
//! * Apple targets: [`Backend::Coretext`] by default. macOS also supports
//!   [`Backend::CoretextFreetype`], [`Backend::CoretextHarfbuzz`] and
//!   [`Backend::CoretextNoshape`], selected with the cargo features
//!   `coretext-freetype`, `coretext-harfbuzz` and `coretext-noshape`
//!   (Zig: `-Dfont-backend=...`). "There is no scenario that is the default"
//!   for those; they're only used by people who want to self-compile Ghostty
//!   and prefer the freetype aesthetic.
//! * wasm32: [`Backend::WebCanvas`].

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// FreeType for font rendering with no font discovery enabled.
    Freetype,
    /// Fontconfig for font discovery and FreeType for font rendering.
    FontconfigFreetype,
    /// CoreText for font discovery, rendering, and shaping (macOS).
    Coretext,
    /// CoreText for font discovery, FreeType for rendering, and
    /// HarfBuzz for shaping (macOS).
    CoretextFreetype,
    /// CoreText for font discovery and rendering, HarfBuzz for shaping
    CoretextHarfbuzz,
    /// CoreText for font discovery and rendering, no shaping.
    CoretextNoshape,
    /// Use the browser font system and the Canvas API (wasm).
    WebCanvas,
}

impl Backend {
    /// Returns the default backend for a build environment.
    pub fn default_for_target() -> Backend {
        // wasm32 always uses the browser Canvas API (Zig: `Backend.default`
        // with `wasm_target == .browser`).
        if cfg!(target_arch = "wasm32") {
            Backend::WebCanvas
        } else if cfg!(target_vendor = "apple") {
            // macOS also supports "coretext_freetype" but there is no
            // scenario that is the default.
            Backend::Coretext
        } else {
            Backend::FontconfigFreetype
        }
    }

    pub fn has_freetype(self) -> bool {
        matches!(
            self,
            Backend::Freetype | Backend::FontconfigFreetype | Backend::CoretextFreetype
        )
    }

    pub fn has_coretext(self) -> bool {
        matches!(
            self,
            Backend::Coretext
                | Backend::CoretextFreetype
                | Backend::CoretextHarfbuzz
                | Backend::CoretextNoshape
        )
    }

    pub fn has_fontconfig(self) -> bool {
        matches!(self, Backend::FontconfigFreetype)
    }

    pub fn has_harfbuzz(self) -> bool {
        matches!(
            self,
            Backend::Freetype
                | Backend::FontconfigFreetype
                | Backend::CoretextFreetype
                | Backend::CoretextHarfbuzz
        )
    }
}

/// Build options (the `font.options` of Zig).
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub backend: Backend,
}

#[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
pub const OPTIONS: Options = Options {
    backend: Backend::FontconfigFreetype,
};

#[cfg(all(target_vendor = "apple", feature = "coretext-freetype"))]
pub const OPTIONS: Options = Options {
    backend: Backend::CoretextFreetype,
};

#[cfg(all(
    target_vendor = "apple",
    feature = "coretext-harfbuzz",
    not(feature = "coretext-freetype")
))]
pub const OPTIONS: Options = Options {
    backend: Backend::CoretextHarfbuzz,
};

#[cfg(all(
    target_vendor = "apple",
    feature = "coretext-noshape",
    not(feature = "coretext-freetype"),
    not(feature = "coretext-harfbuzz")
))]
pub const OPTIONS: Options = Options {
    backend: Backend::CoretextNoshape,
};

#[cfg(all(
    target_vendor = "apple",
    not(feature = "coretext-freetype"),
    not(feature = "coretext-harfbuzz"),
    not(feature = "coretext-noshape")
))]
pub const OPTIONS: Options = Options {
    backend: Backend::Coretext,
};

#[cfg(target_arch = "wasm32")]
pub const OPTIONS: Options = Options {
    backend: Backend::WebCanvas,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_canvas_capabilities() {
        let b = Backend::WebCanvas;
        assert!(!b.has_freetype());
        assert!(!b.has_fontconfig());
        assert!(!b.has_harfbuzz());
        assert!(!b.has_coretext());
    }

    #[test]
    fn coretext_capabilities() {
        let b = Backend::Coretext;
        assert!(!b.has_freetype());
        assert!(!b.has_fontconfig());
        assert!(!b.has_harfbuzz());
        assert!(b.has_coretext());

        let b = Backend::CoretextFreetype;
        assert!(b.has_freetype());
        assert!(!b.has_fontconfig());
        assert!(b.has_harfbuzz());
        assert!(b.has_coretext());

        let b = Backend::CoretextHarfbuzz;
        assert!(!b.has_freetype());
        assert!(b.has_harfbuzz());
        assert!(b.has_coretext());

        let b = Backend::CoretextNoshape;
        assert!(!b.has_freetype());
        assert!(!b.has_harfbuzz());
        assert!(b.has_coretext());
    }

    #[test]
    fn default_backend_per_target() {
        let b = Backend::default_for_target();
        if cfg!(target_vendor = "apple") {
            assert_eq!(b, Backend::Coretext);
        } else if cfg!(target_arch = "wasm32") {
            assert_eq!(b, Backend::WebCanvas);
        } else {
            assert_eq!(b, Backend::FontconfigFreetype);
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
    #[test]
    fn linux_backend_capabilities() {
        let b = OPTIONS.backend;
        assert!(b.has_freetype());
        assert!(b.has_fontconfig());
        assert!(b.has_harfbuzz());
        assert!(!b.has_coretext());
    }
}
