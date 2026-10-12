//! Stand-in for the `SwiftUI.Color` values the Swift `Ghostty.Config` returns
//! (`backgroundColor`, `unfocusedSplitFill`, `splitDividerColor`): an sRGB
//! color with `Double` components. AppKit conversions live in
//! `helpers::extensions::ns_color`.

/// An sRGB color, components in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

impl Color {
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0);

    #[must_use]
    pub const fn new(red: f64, green: f64, blue: f64) -> Self {
        Self { red, green, blue }
    }

    /// `Color(red: Double(r) / 255, ...)` for 8-bit components.
    #[must_use]
    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::new(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        )
    }
}
