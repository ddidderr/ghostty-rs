//! Font discovery. Port of `font/discovery.zig` (fontconfig backend only;
//! the CoreText discoverer is macOS-only and out of scope).

use std::sync::Arc;

use crate::{
    collection::Collection,
    deferred_face::{DeferredFace, Fontconfig as DeferredFontconfig},
    face::Variation,
    fontconfig as fc,
};

/// Discover implementation for the compile options.
pub type Discover = Fontconfig;

/// Descriptor is used to search for fonts (canonical definition in `ghostty-config`).
pub use ghostty_config::font_types::Descriptor;

/// Convert to Fontconfig pattern to use for lookup. The pattern does
/// not have defaults filled/substituted (Fontconfig thing) so callers
/// must still do this. (`Descriptor.toFcPattern`)
pub fn to_fc_pattern(desc: &Descriptor) -> fc::Pattern {
    let mut pat = fc::Pattern::create();
    if let Some(family) = &desc.family {
        assert!(pat.add_string(fc::Property::Family, family));
    }
    if let Some(style) = &desc.style {
        assert!(pat.add_string(fc::Property::Style, style));
    }
    if desc.codepoint > 0 {
        let mut cs = fc::CharSet::create();
        assert!(cs.add_char(desc.codepoint));
        assert!(pat.add_charset(fc::Property::Charset, &cs));
    }
    if desc.size > 0.0 {
        assert!(pat.add_integer(fc::Property::Size, desc.size.round() as i32));
    }
    if desc.bold {
        assert!(pat.add_integer(fc::Property::Weight, fc::Weight::Bold as i32));
    }
    if desc.italic {
        assert!(pat.add_integer(fc::Property::Slant, fc::Slant::Italic as i32));
    }

    // For fontconfig, we always add monospace in the pattern. Since
    // fontconfig sorts by closeness to the pattern, this doesn't fully
    // exclude non-monospace but helps prefer it.
    assert!(pat.add_integer(fc::Property::Spacing, fc::Spacing::Mono as i32));

    pat
}

#[derive(Clone)]
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
        let mut pat = to_fc_pattern(desc);
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
