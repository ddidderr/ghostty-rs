//! `Helpers/Extensions/NSColor+Extension.swift` and
//! `OSColor+Extension.swift` (`OSColor` is `NSColor` on macOS). The math is in
//! `os_color`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, clippy::borrow_as_ptr, clippy::ref_as_ptr)]

use objc2::{Message, rc::Retained};
use objc2_app_kit::{NSColor, NSColorList, NSColorSpace};
use objc2_foundation::NSString;

use super::os_color;

thread_local! {
    /// Using a color list let's us get localized names.
    static APPLE_COLOR_LIST: Option<Retained<NSColorList>> =
        NSColorList::colorListNamed(&NSString::from_str("Apple"));
}

/// `NSColor(named:)`: a color from the "Apple" color list by (capitalized) name.
#[must_use]
pub fn color_named(name: &str) -> Option<Retained<NSColor>> {
    let color = APPLE_COLOR_LIST.with(|list| {
        let list = list.as_ref()?;
        list.colorWithKey(&NSString::from_str(&capitalized(name)))
    })?;
    let components = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(NSColor::colorWithRed_green_blue_alpha(
        components.redComponent(),
        components.greenComponent(),
        components.blueComponent(),
        components.alphaComponent(),
    ))
}

/// Swift's `String.capitalized`: first letter of each word upper, rest lower.
fn capitalized(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// `NSColor.colorNames`.
#[must_use]
pub fn color_names() -> Vec<String> {
    APPLE_COLOR_LIST.with(|list| {
        list.as_ref()
            .map(|l| {
                l.allKeys()
                    .iter()
                    .map(|k| k.to_string().to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// `OSColor(hex:)`.
#[must_use]
pub fn color_from_hex(hex: &str) -> Option<Retained<NSColor>> {
    let (r, g, b, a) = os_color::parse_hex(hex)?;
    Some(NSColor::colorWithRed_green_blue_alpha(r, g, b, a))
}

/// `OSColor(ghostty: ghostty_config_color_s)` from the 8-bit components.
#[must_use]
pub fn color_from_ghostty(r: u8, g: u8, b: u8) -> Retained<NSColor> {
    let (r, g, b) = os_color::from_ghostty(r, g, b);
    NSColor::colorWithRed_green_blue_alpha(r, g, b, 1.0)
}

pub trait NSColorExt {
    /// Returns a new color with its saturation multiplied by the given
    /// factor, clamped to [0, 1].
    fn adjusting_saturation(&self, factor: f64) -> Retained<NSColor>;
    /// Calculates the perceptual distance to another color in RGB space.
    fn distance_to(&self, other: &NSColor) -> f64;
    /// `OSColor.luminance`.
    fn luminance(&self) -> f64;
    /// `OSColor.isLightColor`.
    fn is_light_color(&self) -> bool;
    /// `OSColor.hexString`.
    fn hex_string(&self) -> Option<String>;
    /// `OSColor.darken(by:)`.
    fn darken(&self, amount: f64) -> Retained<NSColor>;
}

#[allow(unsafe_code)]
fn hsb(color: &NSColor) -> (f64, f64, f64, f64) {
    let (mut h, mut s, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
    // SAFETY: the out-pointers are valid for writes.
    unsafe { color.getHue_saturation_brightness_alpha(&mut h, &mut s, &mut b, &mut a) };
    (h, s, b, a)
}

fn srgb(color: &NSColor) -> Option<Retained<NSColor>> {
    color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
}

impl NSColorExt for NSColor {
    fn adjusting_saturation(&self, factor: f64) -> Retained<NSColor> {
        let hsb_color = srgb(self).unwrap_or_else(|| self.retain());
        let (h, s, b, a) = hsb(&hsb_color);
        NSColor::colorWithHue_saturation_brightness_alpha(
            h,
            os_color::adjusted_saturation(s, factor),
            b,
            a,
        )
    }

    fn distance_to(&self, other: &NSColor) -> f64 {
        let (Some(a), Some(b)) = (srgb(self), srgb(other)) else {
            return f64::INFINITY;
        };
        os_color::distance(
            (a.redComponent(), a.greenComponent(), a.blueComponent()),
            (b.redComponent(), b.greenComponent(), b.blueComponent()),
        )
    }

    #[allow(unsafe_code)]
    fn luminance(&self) -> f64 {
        // getRed:green:blue:alpha requires sRGB space
        let Some(rgb) = srgb(self) else { return 0.0 };
        os_color::luminance(
            rgb.redComponent(),
            rgb.greenComponent(),
            rgb.blueComponent(),
        )
    }

    fn is_light_color(&self) -> bool {
        self.luminance() > 0.5
    }

    fn hex_string(&self) -> Option<String> {
        let rgb = self.colorUsingColorSpace(&NSColorSpace::deviceRGBColorSpace())?;
        Some(os_color::hex_string(
            rgb.redComponent(),
            rgb.greenComponent(),
            rgb.blueComponent(),
        ))
    }

    fn darken(&self, amount: f64) -> Retained<NSColor> {
        let (h, s, b, a) = hsb(self);
        NSColor::colorWithHue_saturation_brightness_alpha(
            h,
            s,
            os_color::darkened_brightness(b, amount),
            a,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::capitalized;

    #[test]
    fn capitalized_matches_swift() {
        assert_eq!(capitalized("red"), "Red");
        assert_eq!(capitalized("DARK GRAY"), "Dark Gray");
        assert_eq!(capitalized("a-b"), "A-B");
    }
}
