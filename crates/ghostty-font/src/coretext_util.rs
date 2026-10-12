//! Small helpers over the CoreText / CoreFoundation bindings shared by the
//! CoreText face, shaper and discovery (the Zig `macos` package conveniences
//! they rely on). Apple targets only.
#![allow(unsafe_code, clippy::trivially_copy_pass_by_ref)]

use std::{ops::Deref, ptr::NonNull};

use objc2_core_foundation::{CFData, CFRetained, CFString};
use objc2_core_text::{CTFont, CTFontTableOptions};

use crate::face::coretext_math::utf16_units;

/// A retained CoreFoundation object that may be moved/shared across threads.
///
/// CoreText font objects (`CTFont`, `CTFontDescriptor`) are immutable and
/// documented as thread safe by Apple ("CTFont ... are thread-safe"), but the
/// generated bindings conservatively do not mark them `Send`/`Sync`.
pub struct CfBox<T: ?Sized>(pub(crate) CFRetained<T>);

// SAFETY: see the type docs; the wrapped objects are immutable CoreText
// objects, safe to use from any thread.
unsafe impl<T: ?Sized> Send for CfBox<T> {}
// SAFETY: as above.
unsafe impl<T: ?Sized> Sync for CfBox<T> {}

impl<T: ?Sized> CfBox<T> {
    pub(crate) fn new(v: CFRetained<T>) -> Self {
        CfBox(v)
    }
}

impl<T: ?Sized> Deref for CfBox<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: objc2_core_foundation::Type> Clone for CfBox<T> {
    fn clone(&self) -> Self {
        CfBox(self.0.clone())
    }
}

/// Convert a `CFString` to a Rust `String`.
pub(crate) fn cf_to_string(s: &CFString) -> String {
    s.to_string()
}

/// Copy the sfnt table with the given tag, if present.
pub(crate) fn font_table(font: &CTFont, tag: &[u8; 4]) -> Option<CFRetained<CFData>> {
    // SAFETY: plain CoreText call with valid arguments.
    unsafe { font.table(u32::from_be_bytes(*tag), CTFontTableOptions::NoOptions) }
}

/// Copy the bytes of the sfnt table with the given tag, if present.
pub(crate) fn font_table_bytes(font: &CTFont, tag: &[u8; 4]) -> Option<Vec<u8>> {
    font_table(font, tag).map(|d| d.to_vec())
}

/// `CTFontGetGlyphsForCharacters` for a single UTF-32 codepoint. Returns the
/// glyph ID of the first UTF-16 unit and whether *all* units mapped to glyphs
/// (the C function's return value).
pub(crate) fn glyphs_for_codepoint(font: &CTFont, cp: u32) -> (u16, bool) {
    // Turn UTF-32 into UTF-16 for the CT API
    let (mut unichars, len) = utf16_units(cp);
    let mut glyphs = [0u16; 2];
    // SAFETY: both buffers have at least `len` (<= 2) elements and outlive
    // the call.
    let ok = unsafe {
        font.glyphs_for_characters(
            NonNull::new_unchecked(unichars.as_mut_ptr()),
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            len as isize,
        )
    };
    (glyphs[0], ok)
}

/// A font variation axis as reported by CoreText.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Axis {
    pub name: String,
    pub id: u32,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// The variation axes of a font (`kCTFontVariationAxesAttribute`), or `None`
/// if the font has none.
pub(crate) fn variation_axes(font: &CTFont) -> Option<Vec<Axis>> {
    use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFType};
    use objc2_core_text::{
        kCTFontVariationAxisDefaultValueKey,
        kCTFontVariationAxisIdentifierKey,
        kCTFontVariationAxisMaximumValueKey,
        kCTFontVariationAxisMinimumValueKey,
        kCTFontVariationAxisNameKey,
    };

    // SAFETY: plain CoreText call.
    let axes = unsafe { font.variation_axes() }?;
    // SAFETY: the documented element type of the axes array is CFDictionary.
    let axes: &CFArray<CFDictionary> = unsafe { axes.cast_unchecked() };

    let mut out = Vec::with_capacity(axes.len());
    for i in 0..axes.len() {
        let Some(dict) = axes.get(i) else { continue };
        // SAFETY: the values stored under these keys are CFNumber / CFString
        // as documented for the variation axis keys.
        let (names, nums) = unsafe {
            (
                dict.cast_unchecked::<CFString, CFString>(),
                dict.cast_unchecked::<CFString, CFNumber>(),
            )
        };
        // SAFETY: the keys are valid static CFStrings.
        let (k_name, k_id, k_min, k_max, k_def) = unsafe {
            (
                kCTFontVariationAxisNameKey,
                kCTFontVariationAxisIdentifierKey,
                kCTFontVariationAxisMinimumValueKey,
                kCTFontVariationAxisMaximumValueKey,
                kCTFontVariationAxisDefaultValueKey,
            )
        };
        let _ = std::marker::PhantomData::<CFType>;
        let name = names.get(k_name).map(|s| s.to_string()).unwrap_or_default();
        let id = nums
            .get(k_id)
            .and_then(|n| n.as_i32())
            .map_or(0, |v| v as u32);
        let min = nums.get(k_min).and_then(|n| n.as_f64()).unwrap_or(0.0);
        let max = nums.get(k_max).and_then(|n| n.as_f64()).unwrap_or(0.0);
        let default = nums.get(k_def).and_then(|n| n.as_f64()).unwrap_or(0.0);
        out.push(Axis {
            name,
            id,
            min,
            max,
            default,
        });
    }
    Some(out)
}

/// The current value of each variation axis of the font instance
/// (`kCTFontVariationAttribute`), keyed by axis id.
pub(crate) fn variation_values(font: &CTFont) -> Option<std::collections::HashMap<u32, f64>> {
    use objc2_core_foundation::{CFDictionary, CFNumber};

    // SAFETY: plain CoreText call.
    let values = unsafe { font.variation() }?;
    // SAFETY: the variation dictionary maps CFNumber axis ids to CFNumber
    // values.
    let values: &CFDictionary<CFNumber, CFNumber> = unsafe { values.cast_unchecked() };
    let (keys, vals) = values.to_vecs();
    Some(
        keys.iter()
            .zip(vals.iter())
            .filter_map(|(k, v)| Some((k.as_i32()? as u32, v.as_f64()?)))
            .collect(),
    )
}
