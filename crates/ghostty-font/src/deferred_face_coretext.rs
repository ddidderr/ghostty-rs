//! A deferred face represents a single font face with all the information
//! necessary to load it, but defers loading the full face until it is
//! needed.
//!
//! This allows us to have many fallback fonts to look for glyphs, but
//! only load them if they're really needed.
//!
//! Port of `font/DeferredFace.zig` (CoreText variant; Apple targets).
#![allow(unsafe_code)]

use objc2_core_foundation::CFRetained;
use objc2_core_text::{CTFont, CTFontSymbolicTraits};

use crate::{
    Face,
    Library,
    Presentation,
    coretext_util::{self as util, CfBox},
    face::{self, Options, Variation},
};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Face(#[from] face::FaceError),
    /// `coretext_freetype`: the font has no file we could hand to FreeType.
    #[error("font has no file")]
    FontHasNoFile,
    /// `coretext_freetype`: the font path could not be decoded.
    #[error("font path can't be decoded")]
    FontPathCantDecode,
}

/// CoreText specific data.
pub struct CoreText {
    /// The initialized font
    pub font: CfBox<CTFont>,

    /// Variations to apply to this font. We apply the variations to the
    /// search descriptor but sometimes when the font collection is
    /// made the variation axes are reset so we have to reapply them.
    pub variations: Vec<Variation>,
}

#[derive(Default)]
pub struct DeferredFace {
    /// CoreText
    pub ct: Option<CoreText>,
}

impl DeferredFace {
    pub fn from_coretext(font: CFRetained<CTFont>, variations: Vec<Variation>) -> DeferredFace {
        DeferredFace {
            ct: Some(CoreText {
                font: CfBox::new(font),
                variations,
            }),
        }
    }

    /// Returns the family name of the font.
    pub fn family_name(&self) -> Result<String, LoadError> {
        if let Some(ct) = &self.ct {
            // SAFETY: plain CoreText call.
            return Ok(util::cf_to_string(&*unsafe { ct.font.family_name() }));
        }
        Ok(String::new())
    }

    /// Returns the name of this face.
    pub fn name(&self) -> Result<String, LoadError> {
        if let Some(ct) = &self.ct {
            // SAFETY: plain CoreText call.
            return Ok(util::cf_to_string(&*unsafe { ct.font.display_name() }));
        }
        Ok(String::new())
    }

    /// Load the deferred font face.
    #[cfg(not(font_freetype))]
    pub fn load(&mut self, _lib: &Library, opts: Options) -> Result<Face, LoadError> {
        let ct = self
            .ct
            .as_ref()
            .expect("deferred face has no coretext data");
        let mut face = Face::init_font_copy(&ct.font, opts)?;
        face.set_variations(&ct.variations, opts)?;
        Ok(face)
    }

    /// Load the deferred font face (`coretext_freetype`: FreeType renders the
    /// file CoreText found).
    #[cfg(font_freetype)]
    pub fn load(&mut self, lib: &Library, opts: Options) -> Result<Face, LoadError> {
        use objc2_core_foundation::{CFString, CFURL};
        use objc2_core_text::kCTFontURLAttribute;

        let ct = self
            .ct
            .as_ref()
            .expect("deferred face has no coretext data");

        // Get the URL for the font so we can get the filepath
        // SAFETY: valid static CFString key.
        let url =
            unsafe { ct.font.attribute(kCTFontURLAttribute) }.ok_or(LoadError::FontHasNoFile)?;
        let url = url
            .downcast::<CFURL>()
            .map_err(|_| LoadError::FontHasNoFile)?;

        // Get the path from the URL
        let path = url.path().ok_or(LoadError::FontHasNoFile)?;

        // URL decode the path
        let blank = CFString::from_str("");
        let decoded =
            CFURL::new_string_by_replacing_percent_escapes(None, Some(&path), Some(&blank))
                .ok_or(LoadError::FontPathCantDecode)?;
        let path = util::cf_to_string(&decoded);

        // Face index 0 is not always correct. We don't ship this configuration
        // in a release build. Users should use the pure CoreText builds.
        let mut face = Face::init_file(lib, &path, 0, opts)?;
        face.set_variations(&ct.variations, opts)?;
        Ok(face)
    }

    /// Returns true if this face can satisfy the given codepoint and
    /// presentation. If presentation is null, then it just checks if the
    /// codepoint is present at all.
    ///
    /// This does not require the face to be loaded (the CT font is used).
    pub fn has_codepoint(&self, cp: u32, p: Option<Presentation>) -> bool {
        // If we are using coretext, we check the loaded CT font.
        let Some(ct) = &self.ct else {
            // This is unreachable because discovery mechanisms terminate, and
            // if we're not using a discovery mechanism, the face MUST be loaded.
            unreachable!("deferred face without coretext data");
        };

        // This presentation check isn't as detailed as isColorGlyph
        // because forced presentation modes are only used for emoji and
        // emoji should always have color glyphs set. This can be
        // more correct by using the isColorGlyph logic but I'd want
        // to find a font that actually requires this so we can write
        // a test for it before changing it.
        if let Some(desired_p) = p {
            // SAFETY: plain CoreText call.
            let traits = unsafe { ct.font.symbolic_traits() };
            let actual_p = if traits.contains(CTFontSymbolicTraits::TraitColorGlyphs) {
                Presentation::Emoji
            } else {
                Presentation::Text
            };
            if actual_p != desired_p {
                return false;
            }
        }

        util::glyphs_for_codepoint(&ct.font, cp).1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{Descriptor, Discover};

    #[test]
    fn coretext() {
        // Get a deferred face from CoreText
        let mut def = {
            let disco = Discover::init();
            let mut it = disco
                .discover(&Descriptor {
                    family: Some("Monaco".into()),
                    size: 12.0,
                    ..Default::default()
                })
                .unwrap();
            it.next().unwrap().unwrap()
        };

        // Verify we can get the name
        let n = def.name().unwrap();
        assert_ne!("", n);

        // Load it and verify it works
        let lib = Library::init().unwrap();
        let face = def
            .load(&lib, Options::new(face::DesiredSize::new(12.0)))
            .unwrap();
        assert!(face.glyph_index(' ' as u32).is_some());
    }

    #[test]
    fn empty_deferred_face_has_empty_names() {
        let def = DeferredFace::default();
        assert_eq!("", def.family_name().unwrap());
        assert_eq!("", def.name().unwrap());
    }
}
