//! The color math of `Helpers/Extensions/OSColor+Extension.swift` and
//! `NSColor+Extension.swift` on plain sRGB / HSB components. The AppKit
//! wrappers (colorspace conversion, `NSColor` construction) live in
//! `ns_color`.

/// `OSColor.luminance` for sRGB components.
#[must_use]
pub fn luminance(r: f64, g: f64, b: f64) -> f64 {
    (0.299 * r) + (0.587 * g) + (0.114 * b)
}

/// `OSColor.isLightColor`.
#[must_use]
pub fn is_light_color(r: f64, g: f64, b: f64) -> bool {
    luminance(r, g, b) > 0.5
}

/// `OSColor.hexString` (`#RRGGBB`, components truncated like `Int(x * 255)`).
#[must_use]
pub fn hex_string(r: f64, g: f64, b: f64) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        (r * 255.0) as i64,
        (g * 255.0) as i64,
        (b * 255.0) as i64
    )
}

/// `OSColor(hex:)`: returns `(red, green, blue, alpha)` in `0.0..=1.0`.
///
/// Mirrors the Swift: whitespace trimmed, one leading `#` removed, exactly 6
/// or 8 characters, parsed with `Scanner.scanHexInt64` (optional `0x` prefix,
/// stops at the first non-hex character, fails if no digit was scanned).
#[must_use]
pub fn parse_hex(hex: &str) -> Option<(f64, f64, f64, f64)> {
    let mut cleaned = hex.trim();
    if let Some(rest) = cleaned.strip_prefix('#') {
        cleaned = rest;
    }

    let count = cleaned.chars().count();
    if count != 6 && count != 8 {
        return None;
    }

    let digits = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
        .unwrap_or(cleaned);
    let end = digits
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n = u64::from_str_radix(&digits[..end], 16).ok()?;

    let c = |v: u64| v as f64 / 255.0;
    if count == 8 {
        Some((
            c((n & 0x00FF_0000) >> 16),
            c((n & 0x0000_FF00) >> 8),
            c(n & 0xFF),
            c((n & 0xFF00_0000) >> 24),
        ))
    } else {
        Some((
            c((n & 0xFF_0000) >> 16),
            c((n & 0x00_FF00) >> 8),
            c(n & 0xFF),
            1.0,
        ))
    }
}

/// `OSColor.darken(by:)`: the new brightness.
#[must_use]
pub fn darkened_brightness(brightness: f64, amount: f64) -> f64 {
    (brightness * (1.0 - amount)).min(1.0)
}

/// `NSColor.adjustingSaturation(by:)`: the new saturation, clamped to [0, 1].
#[must_use]
pub fn adjusted_saturation(saturation: f64, factor: f64) -> f64 {
    (saturation * factor).clamp(0.0, 1.0)
}

/// `NSColor.distance(to:)`: weighted Euclidean distance in RGB space (the
/// human eye is more sensitive to green).
#[must_use]
pub fn distance(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dr = a.0 - b.0;
    let dg = a.1 - b.1;
    let db = a.2 - b.2;
    (2.0 * dr * dr + 4.0 * dg * dg + 3.0 * db * db).sqrt()
}

/// `OSColor(ghostty:)`: 8-bit components to `0.0..=1.0`.
#[must_use]
pub fn from_ghostty(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_and_light() {
        assert!(is_light_color(1.0, 1.0, 1.0));
        assert!(!is_light_color(0.0, 0.0, 0.0));
        assert!((luminance(1.0, 0.0, 0.0) - 0.299).abs() < 1e-9);
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_string(1.0, 0.0, 0.5), "#FF007F");
        let (r, g, b, a) = parse_hex("#FF8000").unwrap();
        assert_eq!((r, a), (1.0, 1.0));
        assert!((g - 128.0 / 255.0).abs() < 1e-9 && b == 0.0);
    }

    #[test]
    fn parse_hex_variants() {
        assert_eq!(parse_hex("  #80FF0000 ").unwrap().3, 128.0 / 255.0);
        assert_eq!(parse_hex("80FF0000").unwrap().0, 1.0);
        assert!(parse_hex("#12345").is_none());
        assert!(parse_hex("#1234567").is_none());
        assert!(parse_hex("ZZZZZZ").is_none());
        // scanner semantics: stops at the first non-hex character
        assert!(parse_hex("FF00ZZ").is_some());
    }

    #[test]
    fn darken_saturation_distance() {
        assert!((darkened_brightness(0.5, 0.4) - 0.3).abs() < 1e-9);
        assert_eq!(darkened_brightness(2.0, -0.1), 1.0);
        assert_eq!(adjusted_saturation(0.8, 2.0), 1.0);
        assert_eq!(adjusted_saturation(0.8, -1.0), 0.0);
        assert_eq!(distance((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)), 0.0);
        assert!((distance((0.0, 0.0, 0.0), (1.0, 0.0, 0.0)) - 2f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn ghostty_color() {
        assert_eq!(from_ghostty(255, 0, 51), (1.0, 0.0, 0.2));
    }
}
