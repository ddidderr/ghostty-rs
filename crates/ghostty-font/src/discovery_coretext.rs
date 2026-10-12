//! CoreText font discovery (Apple targets). Port of the `CoreText` struct in
//! `font/discovery.zig` plus `Descriptor.toCoreTextDescriptor`.
#![allow(unsafe_code)]

use std::ptr;

use objc2_core_foundation::{
    CFArray,
    CFCharacterSet,
    CFDictionary,
    CFNumber,
    CFRange,
    CFRetained,
    CFString,
    CFType,
    kCFNull,
};
use objc2_core_text::{
    CTFont,
    CTFontCollection,
    CTFontDescriptor,
    kCTFontCharacterSetAttribute,
    kCTFontFamilyNameAttribute,
    kCTFontSizeAttribute,
    kCTFontStyleNameAttribute,
    kCTFontSymbolicTrait,
    kCTFontTraitsAttribute,
};

use super::Descriptor;
use crate::{
    Style,
    collection::{Collection, Index},
    coretext_util::{self as util, CfBox},
    deferred_face::DeferredFace,
    face::{
        Variation,
        coretext_math::{self as math, Score},
    },
    opentype,
};

/// Discover implementation for the compile options.
pub type Discover = CoreText;

/// The error type of [`DiscoverIterator::next`]. CoreText iteration can't
/// fail, so this is uninhabited.
#[derive(Debug, thiserror::Error)]
pub enum NextError {}

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("coretext descriptor creation failed")]
    DescriptorFailed,
}

/// Retain a value as a `CFType`.
fn as_type<T: AsRef<CFType> + objc2_core_foundation::Type>(v: &T) -> CFRetained<CFType> {
    let r: &CFType = v.as_ref();
    objc2_core_foundation::Type::retain(r)
}

/// Convert to Core Text font descriptor to use for lookup or
/// conversion to a specific font (`Descriptor.toCoreTextDescriptor`).
pub fn to_coretext_descriptor(
    desc: &Descriptor,
) -> Result<CFRetained<CTFontDescriptor>, DiscoverError> {
    let mut keys: Vec<&CFString> = Vec::new();
    // Owned values; we collect references below.
    let mut values: Vec<CFRetained<CFType>> = Vec::new();

    // Family
    if let Some(family) = &desc.family {
        let family = CFString::from_str(family);
        // SAFETY: valid static key.
        keys.push(unsafe { kCTFontFamilyNameAttribute });
        values.push(as_type(&*family));
    }

    // Style
    if let Some(style) = &desc.style {
        let style = CFString::from_str(style);
        // SAFETY: valid static key.
        keys.push(unsafe { kCTFontStyleNameAttribute });
        values.push(as_type(&*style));
    }

    // Codepoint support
    if desc.codepoint > 0 {
        // SAFETY: plain CoreFoundation call with a valid range.
        let cs = unsafe {
            CFCharacterSet::with_characters_in_range(
                None,
                CFRange {
                    location: desc.codepoint as isize,
                    length: 1,
                },
            )
        }
        .ok_or(DiscoverError::DescriptorFailed)?;
        // SAFETY: valid static key.
        keys.push(unsafe { kCTFontCharacterSetAttribute });
        values.push(as_type(&*cs));
    }

    // Set our size attribute if set
    if desc.size > 0.0 {
        let size = CFNumber::new_i32(desc.size.round() as i32);
        // SAFETY: valid static key.
        keys.push(unsafe { kCTFontSizeAttribute });
        values.push(as_type(&*size));
    }

    // Build our traits. If we set any, then we store it in the attributes
    // otherwise we do nothing.
    let traits = math::symbolic_traits_value(desc.bold, desc.italic, desc.monospace);
    if traits > 0 {
        // Setting traits is a pain. We have to create a nested dictionary
        // of the symbolic traits value, and set that in our attributes.
        let traits_num = CFNumber::new_i32(traits as i32);
        // SAFETY: valid static key.
        let k = unsafe { kCTFontSymbolicTrait };
        let nested =
            CFDictionary::<CFString, CFType>::from_slices(&[k], &[&*as_type(&*traits_num)]);
        // SAFETY: valid static key.
        keys.push(unsafe { kCTFontTraitsAttribute });
        values.push(as_type(&*nested));
    }

    let value_refs: Vec<&CFType> = values.iter().map(|v| &**v).collect();
    let attrs = CFDictionary::<CFString, CFType>::from_slices(&keys, &value_refs);

    // SAFETY: valid attributes dictionary.
    Ok(unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) })
}

/// The CoreText discoverer. It has no state.
#[derive(Debug, Clone, Default)]
pub struct CoreText;

impl CoreText {
    pub fn init() -> CoreText {
        // Required for the "interface" but does nothing for CoreText.
        CoreText
    }

    /// Discover fonts from a descriptor. This returns an iterator that can
    /// be used to build up the deferred fonts.
    pub fn discover(&self, desc: &Descriptor) -> Result<DiscoverIterator, DiscoverError> {
        // Build our pattern that we'll search for
        let ct_desc = to_coretext_descriptor(desc)?;

        // Our descriptors have to be in an array
        let desc_arr = CFArray::from_objects(&[&*ct_desc]);

        // Build our collection
        // SAFETY: valid array, no options.
        let set =
            unsafe { CTFontCollection::with_font_descriptors(Some(desc_arr.as_opaque()), None) };
        // SAFETY: plain CoreText call.
        let list = unsafe { set.matching_font_descriptors() };

        // Copy and sort our descriptors
        let mut items: Vec<CFRetained<CTFontDescriptor>> = match list {
            Some(list) => {
                // SAFETY: the matching list contains CTFontDescriptors.
                let typed: &CFArray<CTFontDescriptor> = unsafe { list.cast_unchecked() };
                typed.to_vec()
            }
            None => Vec::new(),
        };
        sort_matching_descriptors(desc, &mut items);

        Ok(DiscoverIterator {
            list: items.into_iter().map(CfBox::new).collect(),
            variations: desc.variations.clone(),
            i: 0,
        })
    }

    pub fn discover_fallback(
        &self,
        collection: &mut Collection,
        desc: &Descriptor,
    ) -> Result<DiscoverIterator, DiscoverError> {
        // If we have a codepoint within the CJK unified ideographs block
        // then we fallback to macOS to find a font that supports it because
        // there isn't a better way manually with CoreText that I can find that
        // properly takes into account system locale.
        //
        // References:
        // - http://unicode.org/charts/PDF/U4E00.pdf
        // - https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/fonts/LocaleInFonts.md#unified-han-ideographs
        if (0x4E00..=0x9FFF).contains(&desc.codepoint)
            && let Some(han) = self.discover_codepoint(collection, desc)
        {
            return Ok(DiscoverIterator {
                list: vec![CfBox::new(han)],
                variations: desc.variations.clone(),
                i: 0,
            });
        }

        let it = self.discover(desc)?;

        // If our normal discovery doesn't find anything and we have a specific
        // codepoint, then fallback to using CTFontCreateForString to find a
        // matching font CoreText wants to use. See:
        // https://github.com/ghostty-org/ghostty/issues/2499
        if it.list.is_empty()
            && desc.codepoint > 0
            && let Some(ct_desc) = self.discover_codepoint(collection, desc)
        {
            return Ok(DiscoverIterator {
                list: vec![CfBox::new(ct_desc)],
                variations: desc.variations.clone(),
                i: 0,
            });
        }

        Ok(it)
    }

    /// Discover a font for a specific codepoint using the CoreText
    /// CTFontCreateForString API.
    #[cfg(font_freetype)]
    #[allow(clippy::unused_self)]
    fn discover_codepoint(
        &self,
        _collection: &mut Collection,
        _desc: &Descriptor,
    ) -> Option<CFRetained<CTFontDescriptor>> {
        // If we have freetype, we can't use CoreText to find a font
        // that supports a specific codepoint because we need to
        // have a CoreText font to be able to do so.
        None
    }

    /// Discover a font for a specific codepoint using the CoreText
    /// CTFontCreateForString API.
    #[cfg(not(font_freetype))]
    #[allow(clippy::unused_self)]
    fn discover_codepoint(
        &self,
        collection: &mut Collection,
        desc: &Descriptor,
    ) -> Option<CFRetained<CTFontDescriptor>> {
        assert!(desc.codepoint > 0);

        // Get our original font. This is dependent on the requested style
        // from the descriptor.
        let original_style = {
            // In all the styles below, we try to match it but if we don't
            // we always fall back to some other option. The order matters
            // here.
            if desc.bold && desc.italic && collection.count(Style::BoldItalic) > 0 {
                Style::BoldItalic
            } else if desc.bold && collection.count(Style::Bold) > 0 {
                Style::Bold
            } else if desc.italic && collection.count(Style::Italic) > 0 {
                Style::Italic
            } else {
                Style::Regular
            }
        };
        let original = collection
            .get_face(Index {
                style: original_style,
                idx: 0,
            })
            .ok()?;

        // We need a CFString for the character.
        let ch = char::from_u32(desc.codepoint)?;
        let str = CFString::from_str(ch.encode_utf8(&mut [0u8; 4]));

        // Get our range length for CTFontCreateForString. It looks like
        // the range uses UTF-16 codepoints and not UTF-32 codepoints.
        let range_len = math::utf16_units(desc.codepoint).1;

        // Get our font
        // SAFETY: valid string and range.
        let font = unsafe {
            original.ct_font().for_string(
                &str,
                CFRange {
                    location: 0,
                    length: range_len as isize,
                },
            )
        };

        // Do not allow the last resort font to go through. This is the
        // last font used by CoreText if it can't find anything else and
        // only contains replacement characters.
        // SAFETY: plain CoreText call.
        let name = util::cf_to_string(&*unsafe { font.post_script_name() });
        // If the name is "LastResort" then we don't want to use it.
        if name == "LastResort" {
            return None;
        }

        // Get the descriptor
        // SAFETY: plain CoreText call.
        Some(unsafe { font.font_descriptor() })
    }
}

fn sort_matching_descriptors(desc: &Descriptor, list: &mut [CFRetained<CTFontDescriptor>]) {
    // Compute the scores once. Higher score is "less" (earlier).
    let mut scored: Vec<(u32, CFRetained<CTFontDescriptor>)> = list
        .iter()
        .map(|d| (score(desc, d).int(), d.clone()))
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (slot, (_, d)) in list.iter_mut().zip(scored) {
        *slot = d;
    }
}

/// Score a CoreText descriptor against the requested descriptor
/// (`Score.score`).
fn score(desc: &Descriptor, ct_desc: &CTFontDescriptor) -> Score {
    let mut this = Score::default();

    // We always load the font if we can since some things can only be
    // inspected on the font itself.
    // SAFETY: valid descriptor, null matrix is allowed.
    let font: CFRetained<CTFont> =
        unsafe { CTFont::with_font_descriptor(ct_desc, 12.0, ptr::null()) };

    // We prefer fonts with more glyphs, all else being equal.
    // SAFETY: plain CoreText call.
    this.glyph_count = u16::try_from(unsafe { font.glyph_count() }).unwrap_or(u16::MAX);

    // If we're searching for a codepoint, then we
    // prioritize fonts that have that codepoint.
    if desc.codepoint > 0 {
        this.codepoint = util::glyphs_for_codepoint(&font, desc.codepoint).1;
    }

    // Get our symbolic traits for the descriptor so we can
    // compare boolean attributes like bold, monospace, etc.
    let symbolic_traits: u32 = (|| {
        // SAFETY: valid static key.
        let traits = unsafe { ct_desc.attribute(kCTFontTraitsAttribute) }?;
        let traits = traits.downcast::<CFDictionary>().ok()?;
        // SAFETY: the traits dictionary maps CFString keys to CFNumber values.
        let traits: &CFDictionary<CFString, CFNumber> = unsafe { traits.cast_unchecked() };
        // SAFETY: valid static key.
        let symbolic = traits.get(unsafe { kCTFontSymbolicTrait })?;
        symbolic.as_i32().map(|v| v as u32)
    })()
    .unwrap_or(0);

    this.monospace = symbolic_traits & math::TRAIT_MONOSPACE != 0;

    // We try to derived data from the font itself, which is generally
    // more reliable than only using the symbolic traits for this.
    let (is_bold, is_italic) = {
        // We start with initial guesses based on the symbolic traits,
        // but refine these with more information if we can get it.
        let mut is_italic = symbolic_traits & math::TRAIT_ITALIC != 0;
        let mut is_bold = symbolic_traits & math::TRAIT_BOLD != 0;

        // Read the 'head' table out of the font data if it's available.
        if let Some(head) = util::font_table_bytes(&font, b"head").and_then(|d| {
            opentype::Head::init(&d)
                .inspect_err(|e| log::warn!("error parsing head table: {e}"))
                .ok()
        }) {
            is_bold = is_bold || (head.macStyle & 1 == 1);
            is_italic = is_italic || (head.macStyle & 2 == 2);
        }

        // Read the 'OS/2' table out of the font data if it's available.
        if let Some(os2) = util::font_table_bytes(&font, b"OS/2").and_then(|d| {
            opentype::OS2::init(&d)
                .inspect_err(|e| log::warn!("error parsing OS/2 table: {e}"))
                .ok()
        }) {
            is_bold = is_bold || os2.fsSelection.bold;
            is_italic = is_italic || os2.fsSelection.italic;
        }

        // Check if we have variation axes in our descriptor, if we
        // do then we can derive weight italic-ness or both from them.
        if let Some(axes) = util::variation_axes(&font) {
            // Copy the variation values for this instance of the font.
            // if there are none then we just skip.
            if let Some(values) = util::variation_values(&font) {
                let list = axes.into_iter().map(|axis| {
                    let val = values.get(&axis.id).copied().unwrap_or(axis.default);
                    (axis.name, val)
                });
                (is_bold, is_italic) = math::bold_italic_from_axes(is_bold, is_italic, list);
            }
        }

        (is_bold, is_italic)
    };

    this.bold = desc.bold == is_bold;
    this.italic = desc.italic == is_italic;

    // Get the style string from the font.
    let style_str: String = (|| {
        // SAFETY: valid static key.
        let style = unsafe { ct_desc.attribute(kCTFontStyleNameAttribute) }?;
        style.downcast::<CFString>().ok().map(|s| s.to_string())
    })()
    .unwrap_or_default();

    // The first string in this slice will be used for the exact match,
    // and for the fuzzy match, all matching substrings will increase
    // the rank.
    let desired_styles = math::desired_styles(desc.style.as_deref(), desc.bold, desc.italic);
    let (exact, fuzzy) = math::style_scores(&style_str, &desired_styles);
    this.exact_style = exact;
    this.fuzzy_style = fuzzy;

    this
}

pub struct DiscoverIterator {
    list: Vec<CfBox<CTFontDescriptor>>,
    variations: Vec<Variation>,
    i: usize,
}

impl DiscoverIterator {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<DeferredFace>, NextError> {
        if self.i >= self.list.len() {
            return Ok(None);
        }

        // Get our descriptor. We need to remove the character set
        // limitation because we may have used that to filter but we
        // don't want it anymore because it'll restrict the characters
        // available.
        let desc = {
            // We create a copy, overwriting the character set attribute.
            // SAFETY: reading the extern static constant.
            let null: &CFType = match unsafe { kCFNull } {
                Some(n) => n.as_ref(),
                None => unreachable!("kCFNull is never null"),
            };
            // SAFETY: valid static key.
            let key = unsafe { kCTFontCharacterSetAttribute };
            let attrs = CFDictionary::<CFString, CFType>::from_slices(&[key], &[null]);
            // SAFETY: valid dictionary.
            unsafe { self.list[self.i].copy_with_attributes(attrs.as_opaque()) }
        };

        // Create our font. We need a size to initialize it so we use size
        // 12 but we will alter the size later.
        // SAFETY: valid descriptor, null matrix is allowed.
        let font = unsafe { CTFont::with_font_descriptor(&desc, 12.0, ptr::null()) };

        // Increment after we return
        self.i += 1;

        Ok(Some(DeferredFace::from_coretext(
            font,
            self.variations.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coretext() {
        let ct = CoreText::init();
        let mut it = ct
            .discover(&Descriptor {
                family: Some("Monaco".into()),
                size: 12.0,
                ..Default::default()
            })
            .unwrap();
        let face = it.next().unwrap().unwrap();
        assert!(face.has_codepoint('A' as u32, None));
    }

    #[test]
    fn coretext_codepoint() {
        let ct = CoreText::init();
        let mut it = ct
            .discover(&Descriptor {
                codepoint: 'A' as u32,
                size: 12.0,
                ..Default::default()
            })
            .unwrap();

        // The first result should have the codepoint. Later ones may not
        // because the results are sorted.
        let face = it.next().unwrap().unwrap();
        assert!(face.has_codepoint('A' as u32, None));

        // Should have other codepoints too
        assert!(face.has_codepoint('B' as u32, None));
    }

    #[test]
    fn coretext_sorting() {
        let ct = CoreText::init();
        let mut it = ct
            .discover(&Descriptor {
                family: Some("Menlo".into()),
                size: 12.0,
                ..Default::default()
            })
            .unwrap();

        // Exercise the scoring/sorting path; the first result for the
        // regular style must be the regular face.
        let first = it.next().unwrap().unwrap();
        assert!(first.name().unwrap().contains("Menlo"));
    }
}
