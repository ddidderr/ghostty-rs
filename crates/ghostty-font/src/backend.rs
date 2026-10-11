//! Font backend selection. Port of `font/backend.zig`.
//!
//! The Rust rewrite only targets Linux, so [`OPTIONS`] always selects
//! [`Backend::FontconfigFreetype`] (fontconfig discovery, FreeType rendering,
//! HarfBuzz shaping). The other variants are kept for fidelity with the Zig
//! enum and its helper predicates.

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
        if cfg!(target_os = "macos") {
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

pub const OPTIONS: Options = Options {
    backend: Backend::FontconfigFreetype,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_backend_capabilities() {
        let b = OPTIONS.backend;
        assert!(b.has_freetype());
        assert!(b.has_fontconfig());
        assert!(b.has_harfbuzz());
        assert!(!b.has_coretext());
    }
}
