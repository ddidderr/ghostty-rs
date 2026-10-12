//! SGR parser C API (ZIG/src/terminal/c/sgr.zig, `include/ghostty/vt/sgr.h`).
#![allow(unsafe_code)]

use std::{alloc::Layout, ffi::c_int, ptr::NonNull};

use ghostty_vt::{
    parser::{MAX_PARAMS, SepList},
    sgr::{Attribute, Parser},
};

use crate::{
    alloc::{Alloc, Allocator},
    color::GhosttyColorRgb,
    result::GhosttyResult,
};

/// C: `GhosttySgrParser` (opaque). Mirrors Zig's `ParserWrapper`: the SGR
/// parser state plus the allocator that owns the parameter copy.
pub struct SgrParser {
    alloc: Alloc,
    /// Owned copy of the parameters (allocated with `alloc`), or null.
    params: *mut u16,
    params_len: usize,
    params_sep: SepList,
    idx: usize,
}

impl SgrParser {
    fn params_layout(len: usize) -> Layout {
        Layout::array::<u16>(len).unwrap_or(Layout::new::<u16>())
    }

    fn free_params(&mut self) {
        if !self.params.is_null() {
            // SAFETY: allocated by `set_params` with this allocator/layout.
            unsafe {
                self.alloc
                    .free_raw(self.params.cast(), Self::params_layout(self.params_len));
            }
            self.params = std::ptr::null_mut();
            self.params_len = 0;
        }
    }

    fn params(&self) -> &[u16] {
        if self.params.is_null() {
            &[]
        } else {
            // SAFETY: `params` points to `params_len` initialized u16s that
            // live until `free_params`.
            unsafe { std::slice::from_raw_parts(self.params, self.params_len) }
        }
    }
}

/// C: `GhosttySgrAttributeTag`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SgrAttributeTag {
    Unset = 0,
    Unknown = 1,
    Bold = 2,
    ResetBold = 3,
    Italic = 4,
    ResetItalic = 5,
    Faint = 6,
    Underline = 7,
    UnderlineColor = 8,
    UnderlineColor256 = 9,
    ResetUnderlineColor = 10,
    Overline = 11,
    ResetOverline = 12,
    Blink = 13,
    ResetBlink = 14,
    Inverse = 15,
    ResetInverse = 16,
    Invisible = 17,
    ResetInvisible = 18,
    Strikethrough = 19,
    ResetStrikethrough = 20,
    DirectColorFg = 21,
    DirectColorBg = 22,
    Bg8 = 23,
    Fg8 = 24,
    ResetFg = 25,
    ResetBg = 26,
    BrightBg8 = 27,
    BrightFg8 = 28,
    Bg256 = 29,
    Fg256 = 30,
}

/// C: `GhosttySgrUnknown`
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SgrUnknown {
    pub full_ptr: *const u16,
    pub full_len: usize,
    pub partial_ptr: *const u16,
    pub partial_len: usize,
}

/// C: `GhosttySgrAttributeValue`
#[derive(Clone, Copy)]
#[repr(C)]
pub union SgrAttributeValue {
    pub unknown: SgrUnknown,
    /// A `GhosttySgrUnderline` (`int`).
    pub underline: c_int,
    pub underline_color: GhosttyColorRgb,
    pub underline_color_256: u8,
    pub direct_color_fg: GhosttyColorRgb,
    pub direct_color_bg: GhosttyColorRgb,
    pub bg_8: u8,
    pub fg_8: u8,
    pub bright_bg_8: u8,
    pub bright_fg_8: u8,
    pub bg_256: u8,
    pub fg_256: u8,
    pub _padding: [u64; 8],
}

/// C: `GhosttySgrAttribute`
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SgrAttribute {
    /// A [`SgrAttributeTag`].
    pub tag: c_int,
    pub value: SgrAttributeValue,
}

impl SgrAttribute {
    fn from_attr(attr: &Attribute<'_>) -> SgrAttribute {
        use SgrAttributeTag as T;
        let mut value = SgrAttributeValue { _padding: [0; 8] };
        let tag = match *attr {
            Attribute::Unset => T::Unset,
            Attribute::Unknown(u) => {
                value.unknown = SgrUnknown {
                    full_ptr: u.full.as_ptr(),
                    full_len: u.full.len(),
                    partial_ptr: u.partial.as_ptr(),
                    partial_len: u.partial.len(),
                };
                T::Unknown
            }
            Attribute::Bold => T::Bold,
            Attribute::ResetBold => T::ResetBold,
            Attribute::Italic => T::Italic,
            Attribute::ResetItalic => T::ResetItalic,
            Attribute::Faint => T::Faint,
            Attribute::Underline(u) => {
                value.underline = c_int::from(u as u8);
                T::Underline
            }
            Attribute::UnderlineColor(c) => {
                value.underline_color = c.into();
                T::UnderlineColor
            }
            Attribute::UnderlineColor256(v) => {
                value.underline_color_256 = v;
                T::UnderlineColor256
            }
            Attribute::ResetUnderlineColor => T::ResetUnderlineColor,
            Attribute::Overline => T::Overline,
            Attribute::ResetOverline => T::ResetOverline,
            Attribute::Blink => T::Blink,
            Attribute::ResetBlink => T::ResetBlink,
            Attribute::Inverse => T::Inverse,
            Attribute::ResetInverse => T::ResetInverse,
            Attribute::Invisible => T::Invisible,
            Attribute::ResetInvisible => T::ResetInvisible,
            Attribute::Strikethrough => T::Strikethrough,
            Attribute::ResetStrikethrough => T::ResetStrikethrough,
            Attribute::DirectColorFg(c) => {
                value.direct_color_fg = c.into();
                T::DirectColorFg
            }
            Attribute::DirectColorBg(c) => {
                value.direct_color_bg = c.into();
                T::DirectColorBg
            }
            Attribute::Bg8(n) => {
                value.bg_8 = n.0;
                T::Bg8
            }
            Attribute::Fg8(n) => {
                value.fg_8 = n.0;
                T::Fg8
            }
            Attribute::ResetFg => T::ResetFg,
            Attribute::ResetBg => T::ResetBg,
            Attribute::Bg8Bright(n) => {
                value.bright_bg_8 = n.0;
                T::BrightBg8
            }
            Attribute::Fg8Bright(n) => {
                value.bright_fg_8 = n.0;
                T::BrightFg8
            }
            Attribute::Bg256(v) => {
                value.bg_256 = v;
                T::Bg256
            }
            Attribute::Fg256(v) => {
                value.fg_256 = v;
                T::Fg256
            }
        };
        SgrAttribute {
            tag: tag as c_int,
            value,
        }
    }
}

/// # Safety
/// `allocator` must be null or valid; `result` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_new(
    allocator: *const Allocator,
    result: *mut *mut SgrParser,
) -> GhosttyResult {
    // SAFETY: forwarded caller contract.
    let alloc = unsafe { Alloc::from_c(allocator) };
    let Some(p) = alloc.create(SgrParser {
        alloc,
        params: std::ptr::null_mut(),
        params_len: 0,
        params_sep: SepList::init_empty(),
        idx: 0,
    }) else {
        return GhosttyResult::OutOfMemory;
    };
    // SAFETY: caller guarantees `result` is valid for writes.
    unsafe { *result = p.as_ptr() };
    GhosttyResult::Success
}

/// # Safety
/// `parser` must be null or a live parser from `ghostty_sgr_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_free(parser: *mut SgrParser) {
    let Some(mut p) = NonNull::new(parser) else {
        return;
    };
    // SAFETY: `p` came from `Alloc::create` in `ghostty_sgr_new`.
    unsafe {
        let alloc = p.as_ref().alloc;
        p.as_mut().free_params();
        alloc.destroy(p);
    }
}

/// # Safety
/// `parser` must be null or a live parser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_reset(parser: *mut SgrParser) {
    // SAFETY: caller guarantees a live parser (or null).
    if let Some(p) = unsafe { parser.as_mut() } {
        p.idx = 0;
    }
}

/// # Safety
/// `parser` must be null or live; `params` must point to `len` `u16`s;
/// `separators`, if non-null, must point to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_set_params(
    parser: *mut SgrParser,
    params: *const u16,
    separators: *const u8,
    len: usize,
) -> GhosttyResult {
    // SAFETY: caller guarantees a live parser (or null).
    let Some(p) = (unsafe { parser.as_mut() }) else {
        return GhosttyResult::InvalidValue;
    };
    if params.is_null() && len > 0 {
        return GhosttyResult::InvalidValue;
    }

    // Copy the new parameters (Zig: `alloc.dupe`).
    let layout = SgrParser::params_layout(len);
    let new = p.alloc.alloc_raw(layout).cast::<u16>();
    if new.is_null() {
        return GhosttyResult::OutOfMemory;
    }
    if len > 0 {
        // SAFETY: `params` has `len` readable elements; `new` has room for `len`.
        unsafe { std::ptr::copy_nonoverlapping(params, new, len) };
    }
    p.free_params();
    p.params = new;
    p.params_len = len;

    // If we have separators, set that state too.
    p.params_sep = SepList::init_empty();
    if !separators.is_null() {
        if len > MAX_PARAMS {
            log::warn!(
                "ghostty_sgr_set_params: separators length {len} exceeds max supported length {MAX_PARAMS}"
            );
            return GhosttyResult::InvalidValue;
        }
        // SAFETY: caller guarantees `len` readable separator bytes.
        let seps = unsafe { std::slice::from_raw_parts(separators, len) };
        for (i, sep) in seps.iter().enumerate() {
            if *sep == b':' {
                p.params_sep.set(i);
            }
        }
    }

    // Reset our parsing state.
    p.idx = 0;
    GhosttyResult::Success
}

/// # Safety
/// `parser` must be null or live; `result` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_next(
    parser: *mut SgrParser,
    result: *mut SgrAttribute,
) -> bool {
    // SAFETY: caller guarantees a live parser (or null).
    let Some(p) = (unsafe { parser.as_mut() }) else {
        return false;
    };
    let mut sp = Parser {
        params: p.params(),
        params_sep: p.params_sep,
        idx: p.idx,
    };
    let attr = sp.next();
    let idx = sp.idx;
    // `attr` borrows `p.params`; convert before touching `p` mutably.
    let Some(attr) = attr else {
        p.idx = idx;
        return false;
    };
    let c = SgrAttribute::from_attr(&attr);
    p.idx = idx;
    // SAFETY: caller guarantees `result` is valid for writes.
    unsafe { result.write(c) };
    true
}

/// # Safety
/// `ptr`, if non-null, must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_unknown_full(
    unknown: SgrUnknown,
    ptr: *mut *const u16,
) -> usize {
    if !ptr.is_null() {
        // SAFETY: non-null and valid per the contract.
        unsafe { *ptr = unknown.full_ptr };
    }
    unknown.full_len
}

/// # Safety
/// `ptr`, if non-null, must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_unknown_partial(
    unknown: SgrUnknown,
    ptr: *mut *const u16,
) -> usize {
    if !ptr.is_null() {
        // SAFETY: non-null and valid per the contract.
        unsafe { *ptr = unknown.partial_ptr };
    }
    unknown.partial_len
}

/// Returns the `GhosttySgrAttributeTag` of `attr`.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_sgr_attribute_tag(attr: SgrAttribute) -> c_int {
    attr.tag
}

/// # Safety
/// `attr` must point to a valid attribute; the result points into it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_sgr_attribute_value(
    attr: *mut SgrAttribute,
) -> *mut SgrAttributeValue {
    // SAFETY: caller guarantees `attr` is valid; no reference is created.
    unsafe { &raw mut (*attr).value }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new() -> *mut SgrParser {
        let mut p = std::ptr::null_mut();
        // SAFETY: valid out pointer, default allocator.
        assert_eq!(
            unsafe { ghostty_sgr_new(std::ptr::null(), &mut p) },
            GhosttyResult::Success
        );
        p
    }

    fn zeroed() -> SgrAttribute {
        SgrAttribute {
            tag: 0,
            value: SgrAttributeValue { _padding: [0; 8] },
        }
    }

    #[test]
    fn alloc() {
        let p = new();
        // SAFETY: live parser.
        unsafe { ghostty_sgr_free(p) };
    }

    #[test]
    fn simple_params_no_seps() {
        let p = new();
        let mut attr = zeroed();
        // SAFETY: live parser; valid buffers.
        unsafe {
            let params = [1u16];
            // Set it twice on purpose to make sure we don't leak.
            for _ in 0..2 {
                assert_eq!(
                    ghostty_sgr_set_params(p, params.as_ptr(), std::ptr::null(), 1),
                    GhosttyResult::Success
                );
            }
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::Bold as c_int);
            assert!(!ghostty_sgr_next(p, &mut attr));
            ghostty_sgr_free(p);
        }
    }

    #[test]
    fn colon_separated_underline_and_colors() {
        let p = new();
        let params: [u16; 17] = [
            4, 3, 38, 2, 51, 51, 51, 48, 2, 170, 170, 170, 58, 2, 255, 97, 136,
        ];
        let mut seps = [b';'; 17];
        seps[0] = b':';
        let mut attr = zeroed();
        // SAFETY: live parser; valid buffers.
        unsafe {
            assert_eq!(
                ghostty_sgr_set_params(p, params.as_ptr(), seps.as_ptr(), 17),
                GhosttyResult::Success
            );
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::Underline as c_int);
            assert_eq!(attr.value.underline, 3);
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::DirectColorFg as c_int);
            assert_eq!(
                attr.value.direct_color_fg,
                GhosttyColorRgb {
                    r: 51,
                    g: 51,
                    b: 51
                }
            );
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::DirectColorBg as c_int);
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::UnderlineColor as c_int);
            assert_eq!(
                attr.value.underline_color,
                GhosttyColorRgb {
                    r: 255,
                    g: 97,
                    b: 136
                }
            );
            assert!(!ghostty_sgr_next(p, &mut attr));
            ghostty_sgr_reset(p);
            assert!(ghostty_sgr_next(p, &mut attr));
            ghostty_sgr_free(p);
        }
    }

    #[test]
    fn empty_params_is_unset_and_too_many_seps_is_invalid() {
        let p = new();
        let mut attr = zeroed();
        // SAFETY: live parser; valid buffers.
        unsafe {
            assert_eq!(
                ghostty_sgr_set_params(p, [].as_ptr(), std::ptr::null(), 0),
                GhosttyResult::Success
            );
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::Unset as c_int);
            assert!(!ghostty_sgr_next(p, &mut attr));

            let params = [0u16; 25];
            let seps = [b';'; 25];
            assert_eq!(
                ghostty_sgr_set_params(p, params.as_ptr(), seps.as_ptr(), 25),
                GhosttyResult::InvalidValue
            );
            assert_eq!(
                ghostty_sgr_set_params(std::ptr::null_mut(), params.as_ptr(), std::ptr::null(), 1),
                GhosttyResult::InvalidValue
            );
            ghostty_sgr_free(p);
        }
    }

    #[test]
    fn unknown_attribute_exposes_params() {
        let p = new();
        let params = [9999u16];
        let mut attr = zeroed();
        // SAFETY: live parser; valid buffers.
        unsafe {
            ghostty_sgr_set_params(p, params.as_ptr(), std::ptr::null(), 1);
            assert!(ghostty_sgr_next(p, &mut attr));
            assert_eq!(attr.tag, SgrAttributeTag::Unknown as c_int);
            let mut ptr: *const u16 = std::ptr::null();
            let n = ghostty_sgr_unknown_full(attr.value.unknown, &mut ptr);
            assert_eq!(std::slice::from_raw_parts(ptr, n), &[9999]);
            let n = ghostty_sgr_unknown_partial(attr.value.unknown, std::ptr::null_mut());
            assert!(n <= 1);
            assert_eq!(
                ghostty_sgr_attribute_tag(attr),
                SgrAttributeTag::Unknown as c_int
            );
            assert_eq!(
                ghostty_sgr_attribute_value(&mut attr).cast::<u8>(),
                (&raw mut attr).cast::<u8>().add(8)
            );
            ghostty_sgr_free(p);
        }
    }
}
