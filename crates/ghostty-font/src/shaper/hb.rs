//! Minimal safe wrapper over the HarfBuzz buffer/shape API (the bits of
//! the Zig `harfbuzz` package that the shaper uses).
#![allow(unsafe_code)]

use std::ptr::NonNull;

use harfbuzz_sys as sys;

use crate::face::freetype::HbFont;

/// A HarfBuzz feature (`hb_feature_t`) applied globally.
pub type Feature = sys::hb_feature_t;

pub const FEATURE_GLOBAL_START: u32 = 0;
pub const FEATURE_GLOBAL_END: u32 = u32::MAX;

/// Build a global feature from a 4-byte tag and value.
pub fn global_feature(tag: [u8; 4], value: u32) -> Feature {
    Feature {
        tag: u32::from_be_bytes(tag),
        value,
        start: FEATURE_GLOBAL_START,
        end: FEATURE_GLOBAL_END,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphInfo {
    pub codepoint: u32,
    pub cluster: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphPosition {
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// An owned `hb_buffer_t`.
pub struct Buffer(NonNull<sys::hb_buffer_t>);

// SAFETY: the buffer is uniquely owned and HarfBuzz buffers have no thread
// affinity; all access goes through `&self`/`&mut self`.
unsafe impl Send for Buffer {}

impl Buffer {
    pub fn new() -> Option<Buffer> {
        // SAFETY: hb_buffer_create always returns a valid (possibly the
        // inert "empty") buffer; we check for null anyway.
        NonNull::new(unsafe { sys::hb_buffer_create() }).map(Buffer)
    }

    pub fn reset(&mut self) {
        // SAFETY: self.0 is a valid buffer.
        unsafe { sys::hb_buffer_reset(self.0.as_ptr()) }
    }

    pub fn set_content_type_unicode(&mut self) {
        // SAFETY: self.0 is a valid buffer.
        unsafe {
            sys::hb_buffer_set_content_type(self.0.as_ptr(), sys::HB_BUFFER_CONTENT_TYPE_UNICODE);
        }
    }

    pub fn set_cluster_level_characters(&mut self) {
        // SAFETY: self.0 is a valid buffer.
        unsafe {
            sys::hb_buffer_set_cluster_level(
                self.0.as_ptr(),
                sys::HB_BUFFER_CLUSTER_LEVEL_CHARACTERS,
            );
        }
    }

    pub fn set_direction_ltr(&mut self) {
        // SAFETY: self.0 is a valid buffer.
        unsafe { sys::hb_buffer_set_direction(self.0.as_ptr(), sys::HB_DIRECTION_LTR) }
    }

    pub fn add(&mut self, cp: u32, cluster: u32) {
        // SAFETY: self.0 is a valid buffer.
        unsafe { sys::hb_buffer_add(self.0.as_ptr(), cp, cluster) }
    }

    pub fn guess_segment_properties(&mut self) {
        // SAFETY: self.0 is a valid buffer.
        unsafe { sys::hb_buffer_guess_segment_properties(self.0.as_ptr()) }
    }

    pub fn len(&self) -> usize {
        // SAFETY: self.0 is a valid buffer.
        unsafe { sys::hb_buffer_get_length(self.0.as_ptr()) as usize }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Glyph infos (valid after shaping).
    pub fn glyph_infos(&mut self) -> Vec<GlyphInfo> {
        let mut len: u32 = 0;
        // SAFETY: self.0 is valid; the returned pointer addresses `len`
        // initialized infos that live until the buffer is next mutated, and
        // we copy them out immediately.
        unsafe {
            let ptr = sys::hb_buffer_get_glyph_infos(self.0.as_ptr(), &raw mut len);
            if ptr.is_null() {
                return Vec::new();
            }
            std::slice::from_raw_parts(ptr, len as usize)
                .iter()
                .map(|i| GlyphInfo {
                    codepoint: i.codepoint,
                    cluster: i.cluster,
                })
                .collect()
        }
    }

    /// Glyph positions (valid after shaping); None if unavailable.
    pub fn glyph_positions(&mut self) -> Option<Vec<GlyphPosition>> {
        let mut len: u32 = 0;
        // SAFETY: as for `glyph_infos`.
        unsafe {
            let ptr = sys::hb_buffer_get_glyph_positions(self.0.as_ptr(), &raw mut len);
            if ptr.is_null() {
                return None;
            }
            Some(
                std::slice::from_raw_parts(ptr, len as usize)
                    .iter()
                    .map(|p| GlyphPosition {
                        x_advance: p.x_advance,
                        y_advance: p.y_advance,
                        x_offset: p.x_offset,
                        y_offset: p.y_offset,
                    })
                    .collect(),
            )
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: we own the buffer reference.
        unsafe { sys::hb_buffer_destroy(self.0.as_ptr()) }
    }
}

/// Shape the buffer with the given font and features.
pub fn shape(font: &HbFont, buf: &mut Buffer, features: &[Feature]) {
    // SAFETY: font and buffer pointers are valid; the feature slice is valid
    // for `features.len()` entries.
    unsafe {
        sys::hb_shape(
            font.as_ptr(),
            buf.0.as_ptr(),
            features.as_ptr(),
            features.len() as u32,
        );
    }
}
