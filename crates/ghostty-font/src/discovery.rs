//! Font discovery. Port of `font/discovery.zig`: the fontconfig backend
//! (Linux, in this file) and the CoreText backend (Apple targets, in
//! `discovery_coretext.rs`).

#[cfg(font_fontconfig)]
pub use fc_impl::*;

#[cfg(font_coretext)]
#[path = "discovery_coretext.rs"]
mod coretext_impl;
#[cfg(font_coretext)]
pub use coretext_impl::*;

/// The `freetype` backend has no discovery mechanism: Zig's
/// `Discover = void`. This is an uninhabited stand-in so that generic code
/// type-checks; it can never be constructed, and the callers check
/// [`crate::HAS_DISCOVERY`] (or `cfg(font_discovery)`) first.
#[cfg(not(any(font_fontconfig, font_coretext)))]
pub struct Discover(std::convert::Infallible);

#[cfg(not(any(font_fontconfig, font_coretext)))]
pub type DiscoverError = std::convert::Infallible;
#[cfg(not(any(font_fontconfig, font_coretext)))]
pub type NextError = std::convert::Infallible;

#[cfg(not(any(font_fontconfig, font_coretext)))]
pub struct DiscoverIterator(std::convert::Infallible);

#[cfg(not(any(font_fontconfig, font_coretext)))]
impl Discover {
    #[must_use]
    pub fn init() -> Discover {
        unreachable!("no font discovery mechanism in this build")
    }

    pub fn discover(&self, _desc: &Descriptor) -> Result<DiscoverIterator, DiscoverError> {
        match self.0 {}
    }
}

#[cfg(not(any(font_fontconfig, font_coretext)))]
impl DiscoverIterator {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<crate::deferred_face::DeferredFace>, NextError> {
        match self.0 {}
    }
}

/// Descriptor is used to search for fonts (canonical definition in `ghostty-config`).
pub use ghostty_config::font_types::Descriptor;

#[cfg(font_fontconfig)]
mod fc_impl {
    use std::sync::Arc;

    use super::Descriptor;
    use crate::{
        collection::Collection,
        deferred_face::{DeferredFace, Fontconfig as DeferredFontconfig},
        face::Variation,
        fontconfig as fc,
    };

    /// Discover implementation for the compile options.
    pub type Discover = Fontconfig;

    /// The error type of [`DiscoverIterator::next`].
    pub type NextError = fc::Error;

    /// Convert to Fontconfig pattern to use for lookup. The pattern does
    /// not have defaults filled/substituted (Fontconfig thing) so callers
    /// must still do this. (`Descriptor.toFcPattern`)
    pub fn to_fc_pattern(desc: &Descriptor) -> fc::Pattern {
        let mut pat = fc::Pattern::create();
        if let Some(family) = &desc.family {
            assert!(pat.add_string_bytes(fc::Property::Family, &family.to_bytes()));
        }
        if let Some(style) = &desc.style {
            assert!(pat.add_string_bytes(fc::Property::Style, &style.to_bytes()));
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
            _collection: &mut Collection,
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
            let font_pattern =
                self.config
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(font_fontconfig)]
    #[test]
    fn interior_nul_in_font_names_ends_the_c_string() {
        use ghostty_config::config::Config;
        use ghostty_os::osbytes::os_bytes;

        use crate::fontconfig::Property;
        let mut config = Config::default();
        config
            .parse_field_bytes("font-family", Some(b"monospace\0suffix"))
            .unwrap();
        config
            .parse_field_bytes("font-style", Some(b"Regular\0suffix"))
            .unwrap();
        config
            .parse_field_bytes("font-codepoint-map", Some(b"U+0041=Fam\0ily"))
            .unwrap();
        let pat = to_fc_pattern(&Descriptor {
            family: Some(config.font_family.list[0].clone()),
            style: config.font_style.name_value().cloned(),
            ..Descriptor::default()
        });
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Family, 0).unwrap()),
            b"monospace"
        );
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Style, 0).unwrap()),
            b"Regular"
        );
        let pat = to_fc_pattern(&config.font_codepoint_map.map.list[0].descriptor);
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Family, 0).unwrap()),
            b"Fam"
        );
    }

    #[cfg(font_fontconfig)]
    #[test]
    fn configured_font_names_reach_fontconfig_as_original_bytes() {
        use ghostty_config::config::Config;
        use ghostty_os::osbytes::os_bytes;

        use crate::fontconfig::Property;
        let mut config = Config::default();
        config
            .parse_field("font-family", Some("Family\u{fdd0}é"))
            .unwrap();
        config
            .parse_field("font-style", Some("Style\u{fdd0}é"))
            .unwrap();
        config
            .parse_field_bytes("font-codepoint-map", Some(b"U+0041=Family\xff"))
            .unwrap();
        let pat = to_fc_pattern(&Descriptor {
            family: Some(config.font_family.list[0].clone()),
            style: config.font_style.name_value().cloned(),
            ..Descriptor::default()
        });
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Family, 0).unwrap()),
            "Family\u{fdd0}é".as_bytes()
        );
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Style, 0).unwrap()),
            "Style\u{fdd0}é".as_bytes()
        );
        let pat = to_fc_pattern(&config.font_codepoint_map.map.list[0].descriptor);
        assert_eq!(
            os_bytes(&pat.get_os_string(Property::Family, 0).unwrap()),
            b"Family\xff"
        );
    }

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

    #[cfg(font_fontconfig)]
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

    #[cfg(font_fontconfig)]
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
