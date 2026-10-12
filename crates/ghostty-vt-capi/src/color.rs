//! `ghostty_color_rgb_get` (ZIG/src/terminal/c/color.zig).
#![allow(unsafe_code)]

/// C: `GhosttyColorRgb`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct GhosttyColorRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<ghostty_vt::color::Rgb> for GhosttyColorRgb {
    fn from(c: ghostty_vt::color::Rgb) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

/// Extract the RGB components of a color.
///
/// # Safety
/// `r`, `g` and `b` must be valid for writing a `u8` (Zig: non-optional
/// pointers).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_color_rgb_get(
    color: GhosttyColorRgb,
    r: *mut u8,
    g: *mut u8,
    b: *mut u8,
) {
    // SAFETY: the caller guarantees all three pointers are valid for writes.
    unsafe {
        *r = color.r;
        *g = color.g;
        *b = color.b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_get() {
        let (mut r, mut g, mut b) = (0u8, 0u8, 0u8);
        // SAFETY: pointers to live locals.
        unsafe {
            ghostty_color_rgb_get(GhosttyColorRgb { r: 1, g: 2, b: 3 }, &mut r, &mut g, &mut b);
        }
        assert_eq!((r, g, b), (1, 2, 3));
    }
}
