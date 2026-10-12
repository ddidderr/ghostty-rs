//! CodepointResolver maps a codepoint to a font. It is more dynamic
//! than "Collection" since it supports mapping codepoint ranges to
//! specific fonts, searching for fallback fonts, and more.
//!
//! To initialize the codepoint resolver, construct it with
//! [`CodepointResolver::new`], set the fields you want set, and begin using
//! the resolver.

#![allow(clippy::collapsible_if, clippy::if_not_else, clippy::float_cmp)]

use std::collections::HashMap;

use crate::{
    Atlas,
    CodepointMap,
    Glyph,
    Presentation,
    SpriteFace,
    Style,
    collection::{AddOptions, Collection, Entry, GetFaceError, Index, PresentationMode, Special},
    discovery::Discover,
    face::{RenderError as FaceRenderError, RenderOptions},
    sprite,
};

/// Packed array of booleans to indicate if a style is enabled or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleStatus([bool; 4]);

impl StyleStatus {
    /// All styles set to `v`.
    pub fn init_fill(v: bool) -> StyleStatus {
        StyleStatus([v; 4])
    }

    pub fn get(&self, style: Style) -> bool {
        self.0[style as usize]
    }

    pub fn set(&mut self, style: Style, v: bool) {
        self.0[style as usize] = v;
    }
}

/// Errors from [`CodepointResolver::render_glyph`].
#[derive(Debug, thiserror::Error)]
pub enum RenderGlyphError {
    #[error(transparent)]
    GetFace(#[from] GetFaceError),
    #[error(transparent)]
    Face(#[from] FaceRenderError),
    #[error(transparent)]
    Sprite(#[from] sprite::face::RenderError),
    /// The sprite face was not configured on the resolver.
    #[error("sprite face not configured")]
    SpriteUnavailable,
}

impl RenderGlyphError {
    /// True if this error is the atlas-full condition (the caller should
    /// grow the atlas and retry).
    pub fn is_atlas_full(&self) -> bool {
        use crate::atlas::Error::AtlasFull;
        matches!(
            self,
            RenderGlyphError::Face(FaceRenderError::Atlas(AtlasFull))
                | RenderGlyphError::Sprite(sprite::face::RenderError::Atlas(AtlasFull))
        )
    }
}

pub struct CodepointResolver {
    /// The underlying collection of fonts. This will be modified as
    /// new fonts are found via the resolver. The resolver takes ownership
    /// of the collection.
    pub collection: Collection,

    /// The set of statuses and whether they're enabled or not. This defaults
    /// to true. This can be changed at runtime with no ill effect.
    pub styles: StyleStatus,

    /// If discovery is available, we'll look up fonts where we can't find
    /// the codepoint. This can be set after initialization.
    pub discover: Option<Discover>,

    /// A map of codepoints to font requests for codepoint-level overrides.
    pub codepoint_map: Option<CodepointMap>,

    /// The descriptor cache is used to cache the descriptor to font face
    /// mapping for codepoint maps.
    descriptor_cache: DescriptorCache,

    /// Set this to a non-null value to enable sprite glyph drawing. If this
    /// isn't enabled we'll just fall through to trying to use regular fonts
    /// to render sprite glyphs. But more than likely, if this isn't set then
    /// terminal rendering will look wrong.
    pub sprite: Option<SpriteFace>,
}

/// Map of descriptors to faces. This is used with manual codepoint maps
/// to ensure that we don't load the same font multiple times.
///
/// Note that the current implementation will load the same font multiple
/// times if the font used for a codepoint map is identical to a font used
/// for a regular style. That's just an inefficient choice made now because
/// the implementation is simpler and codepoint maps matching a regular
/// font is a rare case.
///
/// As in upstream, descriptors are compared by their `hashcode()` only.
#[derive(Default)]
struct DescriptorCache(HashMap<u64, Option<Index>>);

impl CodepointResolver {
    /// A resolver with defaults: all styles enabled, no discovery, no
    /// codepoint map, no sprite face.
    pub fn new(collection: Collection) -> CodepointResolver {
        CodepointResolver {
            collection,
            styles: StyleStatus::init_fill(true),
            discover: None,
            codepoint_map: None,
            descriptor_cache: DescriptorCache::default(),
            sprite: None,
        }
    }

    /// Looks up the font that should be used for a specific codepoint.
    /// The font index is valid as long as font faces aren't removed. This
    /// isn't cached; it is expected that downstream users handle caching if
    /// that is important.
    ///
    /// Optionally, a presentation format can be specified. This presentation
    /// format will be preferred but if it can't be found in this format,
    /// any format will be accepted. If presentation is null, the UCD
    /// (Unicode Character Database) will be used to determine the default
    /// presentation for the codepoint.
    ///
    /// This function cannot error! If an error occurs for any reason,
    /// the associated functionality is ignored and the
    /// resolver attempts to use a different method to satisfy the codepoint.
    /// This behavior is intentional to make the resolver apply best-effort
    /// logic to satisfy the codepoint since its better to render something
    /// than nothing.
    ///
    /// This logic is relatively complex so the exact algorithm is documented
    /// here. If this gets out of sync with the code, ask questions.
    ///
    ///   1. If a font style is requested that is disabled (i.e. bold),
    ///      we start over with the regular font style. The regular font style
    ///      cannot be disabled, but it can be replaced with a stylized font
    ///      face.
    ///
    ///   2. If there is a codepoint override for the codepoint, we satisfy
    ///      that requirement if we can, no matter what style or presentation.
    ///
    ///   3. If this is a sprite codepoint (such as an underline), then the
    ///      sprite font always is the result.
    ///
    ///   4. If the exact style and presentation request can be satisfied by
    ///      one of our loaded fonts, we return that value. We search loaded
    ///      fonts in the order they're added to the group, so the caller must
    ///      set the priority order.
    ///
    ///   5. If the style isn't regular, we restart this process at this point
    ///      but with the regular style. This lets us fall back to regular with
    ///      our loaded fonts before trying a fallback. We'd rather show a regular
    ///      version of a codepoint from a loaded font than find a new font in
    ///      the correct style because styles in other fonts often change
    ///      metrics like glyph widths.
    ///
    ///   6. If the style is regular, and font discovery is enabled, we look
    ///      for a fallback font to satisfy our request.
    ///
    ///   7. Finally, as a last resort, we fall back to restarting this whole
    ///      process with a regular font face satisfying ANY presentation for
    ///      the codepoint. If this fails, we return null.
    pub fn get_index(&mut self, cp: u32, style: Style, p: Option<Presentation>) -> Option<Index> {
        // If we've disabled a font style, then fall back to regular.
        if style != Style::Regular && !self.styles.get(style) {
            return self.get_index(cp, Style::Regular, p);
        }

        // Codepoint overrides.
        if let Some(idx) = self.get_index_codepoint_override(cp) {
            return Some(idx);
        }

        // If we have sprite drawing enabled, check if our sprite face can
        // handle this.
        if let Some(sprite) = &self.sprite {
            if sprite.has_codepoint(cp, p) {
                return Some(Index::init_special(Special::Sprite));
            }
        }

        // Build our presentation mode. If we don't have an explicit presentation
        // given then we use the UCD (Unicode Character Database) to determine
        // the default presentation. Note there is some inefficiency here because
        // we'll do this multiple times if we recurse, but this is a cached function
        // call higher up (GroupCache) so this should be rare.
        let p_mode = match p {
            Some(v) => PresentationMode::Explicit(v),
            None => PresentationMode::Default(if ghostty_unicode::emoji_table::table().get(cp) {
                Presentation::Emoji
            } else {
                Presentation::Text
            }),
        };

        // If we can find the exact value, then return that.
        if let Some(value) = self.collection.get_index(cp, style, p_mode) {
            return Some(value);
        }

        // If we're not a regular font style, try looking for a regular font
        // that will satisfy this request. Blindly looking for unmatched styled
        // fonts to satisfy one codepoint results in some ugly rendering.
        if style != Style::Regular {
            if let Some(value) = self.get_index(cp, Style::Regular, p) {
                return Some(value);
            }
        }

        // If we are regular, try looking for a fallback using discovery.
        if style == Style::Regular {
            if let Some(idx) = self.discover_fallback(cp, style, p_mode) {
                return Some(idx);
            }
        }

        // If this is regular with any matching presentation, then we are done
        // there is nothing more we can do. Otherwise we fall through and do
        // an any presentation search.
        if style == Style::Regular && p_mode == PresentationMode::Any {
            return None;
        }

        // For non-regular fonts, we fall back to regular with any presentation
        self.collection
            .get_index(cp, Style::Regular, PresentationMode::Any)
    }

    fn discover_fallback(
        &mut self,
        cp: u32,
        style: Style,
        p_mode: PresentationMode,
    ) -> Option<Index> {
        log::debug!("searching for a fallback font for cp={cp:X}");
        let disco = self.discover.as_ref()?;
        let load_opts = self.collection.load_options.as_ref()?;
        let desc = crate::discovery::Descriptor {
            codepoint: cp,
            size: load_opts.size.points,
            bold: style == Style::Bold || style == Style::BoldItalic,
            italic: style == Style::Italic || style == Style::BoldItalic,
            monospace: false,
            ..Default::default()
        };
        let mut disco_it = disco.discover_fallback(&mut self.collection, &desc).ok()?;

        loop {
            let deferred_face = match disco_it.next() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(err) => {
                    log::warn!("fallback search failed with error err={err}");
                    break;
                }
            };

            // Discovery is supposed to only return faces that have our
            // codepoint but we can't search presentation in discovery so
            // we have to check it here.
            let entry = Entry {
                face: crate::collection::AnyFace::Deferred(deferred_face),
                fallback: true,
                scale_factor: crate::collection::ScaleFactor::Scale(1.0),
            };
            if !entry.has_codepoint(cp, p_mode) {
                continue;
            }
            let crate::collection::AnyFace::Deferred(deferred_face) = entry.face else {
                unreachable!()
            };

            log::info!(
                "found codepoint 0x{cp:X} in fallback face={}",
                deferred_face
                    .name()
                    .unwrap_or_else(|_| "<error>".to_string())
            );
            return self
                .collection
                .add_deferred(
                    deferred_face,
                    AddOptions {
                        style,
                        fallback: true,
                        size_adjustment: crate::DEFAULT_FALLBACK_ADJUSTMENT,
                    },
                )
                .ok();
        }

        log::debug!("no fallback face found for cp={cp:X}");
        None
    }

    /// Checks if the codepoint is in the map of codepoint overrides,
    /// finds the override font, and returns it.
    fn get_index_codepoint_override(&mut self, cp: u32) -> Option<Index> {
        // Get our codepoint map. If we have no map set then we have no
        // codepoint overrides and we're done.
        let map = self.codepoint_map.as_ref()?;

        // If the codepoint is in the map, the map returns a descriptor that
        // can be used with font discovery to search for a matching font.
        let desc = map.get(cp)?;
        let key = desc.hashcode();

        // Fast path: the descriptor is already loaded. This means that we
        // already did the search before and we have an exact font for this
        // codepoint.
        let idx_: Option<Index> = if let Some(v) = self.descriptor_cache.0.get(&key) {
            *v
        } else {
            // Slow path: we have to find this descriptor and load the font
            let discover = self.discover.as_ref()?;
            let mut disco_it = match discover.discover(desc) {
                Ok(it) => it,
                Err(err) => {
                    log::warn!("codepoint override failed codepoint={cp} err={err}");
                    return None;
                }
            };

            let face = match disco_it.next() {
                Ok(Some(f)) => f,
                Ok(None) => {
                    log::warn!(
                        "font lookup for codepoint map failed codepoint={cp} err=FontNotFound"
                    );

                    // Add null to the cache so we don't do a lookup again later.
                    self.descriptor_cache.0.insert(key, None);
                    return None;
                }
                Err(err) => {
                    log::warn!("codepoint override failed codepoint={cp} err={err}");
                    return None;
                }
            };

            // Add the font to our list of fonts so we can get an index for it,
            // and ensure the index is stored in the descriptor cache for next time.
            let idx = match self.collection.add_deferred(
                face,
                AddOptions {
                    style: Style::Regular,
                    fallback: false,
                    size_adjustment: crate::DEFAULT_FALLBACK_ADJUSTMENT,
                },
            ) {
                Ok(idx) => idx,
                Err(err) => {
                    log::warn!("codepoint override failed codepoint={cp} err={err}");
                    return None;
                }
            };
            self.descriptor_cache.0.insert(key, Some(idx));

            Some(idx)
        };

        // The descriptor cache will populate null if the descriptor is not found
        // to avoid expensive discoveries later, so if it is null then we already
        // searched and found nothing.
        let idx = idx_?;

        // We need to verify that this index has the codepoint we want.
        if self
            .collection
            .has_codepoint(idx, cp, PresentationMode::Any)
        {
            log::debug!(
                "codepoint override based on config codepoint={cp} family={}",
                desc.family
                    .as_ref()
                    .map_or("", ghostty_os::osbytes::EncodedOs::as_encoded)
            );

            return Some(idx);
        }

        None
    }

    /// Returns the presentation for a specific font index. This is useful for
    /// determining what atlas is needed.
    pub fn get_presentation(
        &mut self,
        index: Index,
        glyph_index: u32,
    ) -> Result<Presentation, GetFaceError> {
        if let Some(sp) = index.special() {
            return Ok(match sp {
                Special::Sprite => Presentation::Text,
            });
        }

        let face = self.collection.get_face(index)?;
        Ok(if face.is_color_glyph(glyph_index) {
            Presentation::Emoji
        } else {
            Presentation::Text
        })
    }

    /// Render a glyph by glyph index into the given font atlas and return
    /// metadata about it.
    ///
    /// This performs no caching, it is up to the caller to cache calls to this
    /// if they want. This will also not resize the atlas if it is full.
    ///
    /// IMPORTANT: this renders by /glyph index/ and not by /codepoint/. The caller
    /// is expected to translate codepoints to glyph indexes in some way. The most
    /// trivial way to do this is to get the Face and call glyph_index. If you're
    /// doing text shaping, the text shaping library (i.e. HarfBuzz) will automatically
    /// determine glyph indexes for a text run.
    pub fn render_glyph(
        &mut self,
        atlas: &mut Atlas,
        index: Index,
        glyph_index: u32,
        opts: RenderOptions,
    ) -> Result<Glyph, RenderGlyphError> {
        // Special-case fonts are rendered directly.
        if let Some(sp) = index.special() {
            match sp {
                Special::Sprite => {
                    let sprite = self
                        .sprite
                        .as_ref()
                        .ok_or(RenderGlyphError::SpriteUnavailable)?;
                    return Ok(sprite.render_glyph(atlas, glyph_index, opts)?);
                }
            }
        }

        let face = self.collection.get_face(index)?;
        Ok(face.render_glyph(atlas, glyph_index, opts)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Face,
        Library,
        collection::{LoadOptions, SizeAdjustment},
        embedded,
        face::{DesiredSize, Options},
    };

    fn add(c: &mut Collection, lib: &Library, data: &'static [u8], style: Style, pts: f32) {
        let size = if pts == 12.0 {
            DesiredSize {
                points: 12.0,
                xdpi: 96,
                ydpi: 96,
            }
        } else {
            DesiredSize::new(pts)
        };
        c.add(
            Face::init(lib, data.into(), Options::new(size)).unwrap(),
            AddOptions {
                style,
                fallback: false,
                size_adjustment: SizeAdjustment::None,
            },
        )
        .unwrap();
    }

    #[test]
    fn get_index() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        add(&mut c, &lib, embedded::REGULAR, Style::Regular, 12.0);
        // Coretext doesn't support Noto's format
        #[cfg(not(font_coretext))]
        add(&mut c, &lib, embedded::EMOJI, Style::Regular, 12.0);
        add(&mut c, &lib, embedded::EMOJI_TEXT, Style::Regular, 12.0);

        let mut r = CodepointResolver::new(c);

        // Should find all visible ASCII
        for i in 32u32..127 {
            let idx = r.get_index(i, Style::Regular, None).unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(0, idx.idx);
        }

        // Try emoji
        {
            let idx = r.get_index('🥸' as u32, Style::Regular, None).unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(1, idx.idx);
        }

        // Try text emoji
        {
            let idx = r
                .get_index(0x270C, Style::Regular, Some(Presentation::Text))
                .unwrap();
            assert_eq!(Style::Regular, idx.style);
            let text_idx = if cfg!(font_coretext) { 1 } else { 2 };
            assert_eq!(text_idx, idx.idx);
        }
        {
            let idx = r
                .get_index(0x270C, Style::Regular, Some(Presentation::Emoji))
                .unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(1, idx.idx);
        }

        // Box glyph should be null since we didn't set a box font
        assert!(r.get_index(0x1FB00, Style::Regular, None).is_none());
    }

    #[test]
    fn get_index_disabled_font_style() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        add(&mut c, &lib, embedded::REGULAR, Style::Regular, 12.0);
        add(&mut c, &lib, embedded::REGULAR, Style::Bold, 12.0);
        add(&mut c, &lib, embedded::REGULAR, Style::Italic, 12.0);

        let mut r = CodepointResolver::new(c);
        r.styles.set(Style::Bold, false); // Disable bold

        // Regular should work fine
        {
            let idx = r.get_index('A' as u32, Style::Regular, None).unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(0, idx.idx);
        }

        // Bold should go to regular
        {
            let idx = r.get_index('A' as u32, Style::Bold, None).unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(0, idx.idx);
        }

        // Italic should still work
        {
            let idx = r.get_index('A' as u32, Style::Italic, None).unwrap();
            assert_eq!(Style::Italic, idx.style);
            assert_eq!(0, idx.idx);
        }
    }

    #[test]
    fn get_index_box_glyph() {
        let c = Collection::new();

        let mut r = CodepointResolver::new(c);
        r.sprite = Some(SpriteFace::new(crate::Metrics::calc(
            crate::metrics::FaceMetrics {
                px_per_em: 30.0,
                cell_width: 18.0,
                ascent: 30.0,
                descent: -6.0,
                line_gap: 0.0,
                ..Default::default()
            },
        )));

        // Should find a box glyph
        let idx = r.get_index(0x2500, Style::Regular, None).unwrap();
        assert_eq!(Style::Regular, idx.style);
        assert_eq!(Index::SPECIAL_START, idx.idx);
    }

    #[test]
    fn get_presentation_and_render_sprite() {
        let mut r = CodepointResolver::new(Collection::new());
        let metrics = crate::Metrics::calc(crate::metrics::FaceMetrics {
            px_per_em: 30.0,
            cell_width: 18.0,
            ascent: 30.0,
            descent: -6.0,
            line_gap: 0.0,
            ..Default::default()
        });
        r.sprite = Some(SpriteFace::new(metrics));
        let sprite_idx = Index::init_special(Special::Sprite);
        assert_eq!(
            Presentation::Text,
            r.get_presentation(sprite_idx, 0x2500).unwrap()
        );
        let mut atlas = Atlas::new(256, crate::atlas::Format::Grayscale);
        let g = r
            .render_glyph(&mut atlas, sprite_idx, 0x2500, RenderOptions::new(metrics))
            .unwrap();
        assert!(g.width > 0 && g.height > 0);
    }
}
