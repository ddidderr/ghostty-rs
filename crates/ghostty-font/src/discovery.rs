//! Font discovery. Port of `font/discovery.zig` (fontconfig backend only;
//! the CoreText discoverer is macOS-only and out of scope).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::collection::Collection;
use crate::deferred_face::{DeferredFace, Fontconfig as DeferredFontconfig};
use crate::face::Variation;
use crate::fontconfig as fc;

/// Discover implementation for the compile options.
pub type Discover = Fontconfig;

/// Descriptor is used to search for fonts. The only required field
/// is "family". The rest are ignored unless they're set to a non-zero
/// value.
#[derive(Debug, Clone, Default)]
pub struct Descriptor {
    /// Font family to search for. This can be a fully qualified font
    /// name such as "Fira Code", "monospace", "serif", etc.
    ///
    /// On systems that use fontconfig (Linux), this can be a full
    /// fontconfig pattern, such as "Fira Code-14:bold".
    pub family: Option<String>,

    /// Specific font style to search for. This will filter the style
    /// string the font advertises. The "bold/italic" booleans later in this
    /// struct filter by the style trait the font has, not the string, so
    /// these can be used in conjunction or not.
    pub style: Option<String>,

    /// A codepoint that this font must be able to render.
    pub codepoint: u32,

    /// Font size in points that the font should support. For conversion
    /// to pixels, we will use 72 DPI for Mac and 96 DPI for everything else.
    /// (If pixel conversion is necessary, i.e. emoji fonts)
    pub size: f32,

    /// True if we want to search specifically for a font that supports
    /// specific styles.
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,

    /// Variation axes to apply to the font. This also impacts searching
    /// for fonts since fonts with the ability to set these variations
    /// will be preferred, but not guaranteed.
    pub variations: Vec<Variation>,
}

impl Hash for Descriptor {
    /// Hash the descriptor with the given hasher. Covers the same fields
    /// as the Zig implementation.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.family.hash(state);
        self.style.hash(state);
        self.codepoint.hash(state);
        self.size.to_bits().hash(state);
        self.bold.hash(state);
        self.italic.hash(state);
        self.monospace.hash(state);
        self.variations.len().hash(state);
        for variation in &self.variations {
            variation.id.hash(state);

            // This is not correct, but we don't currently depend on the
            // hash value being different based on decimal values of variations.
            (variation.value as i64).hash(state);
        }
    }
}

impl Descriptor {
    /// Returns a hash code that can be used to uniquely identify this
    /// action.
    pub fn hashcode(&self) -> u64 {
        // `DefaultHasher::new()` uses fixed keys, so this is deterministic.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    /// Convert to Fontconfig pattern to use for lookup. The pattern does
    /// not have defaults filled/substituted (Fontconfig thing) so callers
    /// must still do this.
    pub fn to_fc_pattern(&self) -> fc::Pattern {
        let mut pat = fc::Pattern::create();
        if let Some(family) = &self.family {
            assert!(pat.add_string(fc::Property::Family, family));
        }
        if let Some(style) = &self.style {
            assert!(pat.add_string(fc::Property::Style, style));
        }
        if self.codepoint > 0 {
            let mut cs = fc::CharSet::create();
            assert!(cs.add_char(self.codepoint));
            assert!(pat.add_charset(fc::Property::Charset, &cs));
        }
        if self.size > 0.0 {
            assert!(pat.add_integer(fc::Property::Size, self.size.round() as i32));
        }
        if self.bold {
            assert!(pat.add_integer(fc::Property::Weight, fc::Weight::Bold as i32));
        }
        if self.italic {
            assert!(pat.add_integer(fc::Property::Slant, fc::Slant::Italic as i32));
        }

        // For fontconfig, we always add monospace in the pattern. Since
        // fontconfig sorts by closeness to the pattern, this doesn't fully
        // exclude non-monospace but helps prefer it.
        assert!(pat.add_integer(fc::Property::Spacing, fc::Spacing::Mono as i32));

        pat
    }
}

pub struct Fontconfig {
    fc_config: Arc<fc::Config>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("fontconfig failed")]
    FontConfigFailed,
    #[error(transparent)]
    Fontconfig(#[from] fc::Error),
}

impl Fontconfig {
    pub fn init() -> Fontconfig {
        // safe to call multiple times and concurrently
        fc::init();
        Fontconfig {
            fc_config: Arc::new(
                fc::Config::init_load_config_and_fonts()
                    .expect("FcInitLoadConfigAndFonts returned null"),
            ),
        }
    }

    /// Discover fonts from a descriptor. This returns an iterator that can
    /// be used to build up the deferred fonts.
    pub fn discover(&self, desc: &Descriptor) -> Result<DiscoverIterator, DiscoverError> {
        // Build our pattern that we'll search for
        let mut pat = desc.to_fc_pattern();
        assert!(self.fc_config.substitute_with_pat_pattern(&mut pat));
        pat.default_substitute();

        // Search
        let set = self
            .fc_config
            .font_sort(&mut pat, false)
            .map_err(|_| DiscoverError::FontConfigFailed)?;

        Ok(DiscoverIterator {
            config: Arc::clone(&self.fc_config),
            pattern: pat,
            set,
            variations: desc.variations.clone(),
            i: 0,
        })
    }

    pub fn discover_fallback(
        &self,
        _collection: &Collection,
        desc: &Descriptor,
    ) -> Result<DiscoverIterator, DiscoverError> {
        self.discover(desc)
    }
}

pub struct DiscoverIterator {
    config: Arc<fc::Config>,
    pattern: fc::Pattern,
    set: fc::FontSet,
    variations: Vec<Variation>,
    i: usize,
}

impl DiscoverIterator {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<DeferredFace>, fc::Error> {
        if self.i >= self.set.len() {
            return Ok(None);
        }

        // Get the copied pattern from our fontset that has the
        // attributes configured for rendering.
        let font_pattern = self
            .config
            .font_render_prepare(&mut self.pattern, &self.set, self.i)?;

        // The charset and langset must exist (they are used for lookups).
        font_pattern.charset_has_char(0)?;
        font_pattern.langset_has_lang("")?;

        // Increment after we return
        self.i += 1;

        Ok(Some(DeferredFace::from_fontconfig(DeferredFontconfig {
            pattern: font_pattern,
            variations: self.variations.clone(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_hash() {
        let d = Descriptor::default();
        assert_ne!(d.hashcode(), 0);
    }

    #[test]
    fn descriptor_hash_family_names() {
        let d1 = Descriptor {
            family: Some("A".into()),
            ..Default::default()
        };
        let d2 = Descriptor {
            family: Some("B".into()),
            ..Default::default()
        };
        assert_ne!(d1.hashcode(), d2.hashcode());
    }

    #[test]
    fn descriptor_hash_is_deterministic_and_covers_fields() {
        let a = Descriptor {
            family: Some("A".into()),
            bold: true,
            ..Default::default()
        };
        assert_eq!(a.hashcode(), a.clone().hashcode());
        let mut b = a.clone();
        b.italic = true;
        assert_ne!(a.hashcode(), b.hashcode());
    }

    #[test]
    fn fontconfig() {
        let fc = Fontconfig::init();
        let _it = fc
            .discover(&Descriptor {
                family: Some("monospace".into()),
                size: 12.0,
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn fontconfig_codepoint() {
        let fc = Fontconfig::init();
        let mut it = fc
            .discover(&Descriptor {
                codepoint: 'A' as u32,
                size: 12.0,
                ..Default::default()
            })
            .unwrap();

        // The first result should have the codepoint. Later ones may not
        // because fontconfig returns all fonts sorted.
        let face = it.next().unwrap().unwrap();
        assert!(face.has_codepoint('A' as u32, None));

        // Should have other codepoints too
        assert!(face.has_codepoint('B' as u32, None));
    }
}
