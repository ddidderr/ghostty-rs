//! A deferred face represents a single font face with all the information
//! necessary to load it, but defers loading the full face until it is
//! needed.
//!
//! This allows us to have many fallback fonts to look for glyphs, but
//! only load them if they're really needed.
//!
//! Port of `font/DeferredFace.zig` (fontconfig variant only).

use crate::{
    Face,
    Library,
    Presentation,
    face::{self, Options, Variation},
    fontconfig as fc,
};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Fontconfig(#[from] fc::Error),
    #[error(transparent)]
    Face(#[from] face::FaceError),
}

/// Fontconfig specific data. This is only present if building with fontconfig.
pub struct Fontconfig {
    /// The pattern for this font. This must be the "render prepared" pattern.
    /// (i.e. call FcFontRenderPrepare). The charset and langset used for
    /// quick codepoint / presentation lookups are read from this pattern.
    pub pattern: fc::Pattern,

    /// Variations to apply to this font.
    pub variations: Vec<Variation>,
}

#[derive(Default)]
pub struct DeferredFace {
    /// Fontconfig
    pub fc: Option<Fontconfig>,
}

impl DeferredFace {
    pub fn from_fontconfig(fc: Fontconfig) -> DeferredFace {
        DeferredFace { fc: Some(fc) }
    }

    /// Returns the family name of the font.
    pub fn family_name(&self) -> Result<String, LoadError> {
        if let Some(fc) = &self.fc {
            return Ok(fc.pattern.get_string(fc::Property::Family, 0)?);
        }
        Ok(String::new())
    }

    /// Returns the name of this face.
    pub fn name(&self) -> Result<String, LoadError> {
        if let Some(fc) = &self.fc {
            return Ok(fc.pattern.get_string(fc::Property::FullName, 0)?);
        }
        Ok(String::new())
    }

    /// Load the deferred font face.
    pub fn load(&mut self, lib: &Library, opts: Options) -> Result<Face, LoadError> {
        let fc = self
            .fc
            .as_ref()
            .expect("deferred face has no fontconfig data");

        // Filename and index for our face so we can load it
        let filename = fc.pattern.get_string(fc::Property::File, 0)?;
        let face_index = fc.pattern.get_integer(fc::Property::Index, 0)?;

        let mut face = Face::init_file(lib, &filename, face_index, opts)?;
        face.set_variations(&fc.variations, opts)?;
        Ok(face)
    }

    /// Returns true if this face can satisfy the given codepoint and
    /// presentation. If presentation is null, then it just checks if the
    /// codepoint is present at all.
    ///
    /// This does not require the face to be loaded (fontconfig metadata is
    /// used).
    pub fn has_codepoint(&self, cp: u32, p: Option<Presentation>) -> bool {
        // If we are using fontconfig, use the fontconfig metadata to
        // avoid loading the face.
        let Some(fc) = &self.fc else {
            // This is unreachable because discovery mechanisms terminate, and
            // if we're not using a discovery mechanism, the face MUST be loaded.
            unreachable!("deferred face without fontconfig data");
        };

        // Check if char exists
        if !fc.pattern.charset_has_char(cp).unwrap_or(false) {
            return false;
        }

        // If we have a presentation, check it matches
        if let Some(desired) = p {
            let emoji_lang = "und-zsye";
            let actual = if fc.pattern.langset_has_lang(emoji_lang).unwrap_or(false) {
                Presentation::Emoji
            } else {
                Presentation::Text
            };

            return desired == actual;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{Descriptor, Fontconfig};

    // NOTE: this test needs the real FreeType `Face` (agent A); it cannot
    // run while `Face` is a stub.
    #[test]
    fn fontconfig() {
        // Load freetype
        let lib = Library::init().unwrap();

        // Get a deferred face from fontconfig
        let mut def = {
            let fc = Fontconfig::init();
            let mut it = fc
                .discover(&Descriptor {
                    family: Some("monospace".into()),
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
        let face = def
            .load(&lib, Options::new(face::DesiredSize::new(12.0)))
            .unwrap();
        assert!(face.glyph_index(' ' as u32).is_some());
    }

    #[test]
    fn family_name_and_presentation_from_fontconfig() {
        let fc = Fontconfig::init();
        let mut it = fc
            .discover(&Descriptor {
                family: Some("monospace".into()),
                size: 12.0,
                ..Default::default()
            })
            .unwrap();
        let def = it.next().unwrap().unwrap();
        assert_ne!("", def.family_name().unwrap());
        // A monospace text font is not an emoji-presentation font.
        assert!(!def.has_codepoint('A' as u32, Some(Presentation::Emoji)));
    }

    #[test]
    fn empty_deferred_face_has_empty_names() {
        let def = DeferredFace::default();
        assert_eq!("", def.family_name().unwrap());
        assert_eq!("", def.name().unwrap());
    }
}
