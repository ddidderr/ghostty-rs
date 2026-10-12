//! The configuration types of `Ghostty.ConfigTypes.swift` and the enums /
//! option sets declared in the `Ghostty.Config` extension of
//! `Ghostty.Config.swift` (and the small config enums of
//! `GhosttyPackage.swift`).
//!
//! Swift `enum X: String` with `init?(rawValue:)` becomes a Rust enum with
//! `from_raw_value(&str) -> Option<Self>` and `raw_value()`.

use bitflags::bitflags;

/// Declares a Swift `enum Name: String` with explicit raw values.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $raw:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// `init?(rawValue:)`.
            #[must_use]
            pub fn from_raw_value(raw: &str) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// `rawValue`.
            #[must_use]
            pub const fn raw_value(self) -> &'static str {
                match self {
                    $(Self::$variant => $raw),+
                }
            }
        }
    };
}

/// A configuration path value that may be optional or required.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPath {
    pub path: String,
    pub optional: bool,
}

string_enum! {
    /// macos-icon
    MacOSIcon {
        Official = "official",
        Blueprint = "blueprint",
        Chalkboard = "chalkboard",
        Glass = "glass",
        Holographic = "holographic",
        Microchip = "microchip",
        Paper = "paper",
        Retro = "retro",
        Xray = "xray",
        Custom = "custom",
        CustomStyle = "custom-style",
    }
}

impl MacOSIcon {
    /// Bundled asset name for built-in icons
    #[must_use]
    pub const fn asset_name(self) -> Option<&'static str> {
        match self {
            Self::Official | Self::Custom | Self::CustomStyle => None,
            Self::Blueprint => Some("BlueprintImage"),
            Self::Chalkboard => Some("ChalkboardImage"),
            Self::Microchip => Some("MicrochipImage"),
            Self::Glass => Some("GlassImage"),
            Self::Holographic => Some("HolographicImage"),
            Self::Paper => Some("PaperImage"),
            Self::Retro => Some("RetroImage"),
            Self::Xray => Some("XrayImage"),
        }
    }
}

string_enum! {
    /// macos-icon-frame
    MacOSIconFrame {
        Aluminum = "aluminum",
        Beige = "beige",
        Plastic = "plastic",
        Chrome = "chrome",
    }
}

string_enum! {
    AutoUpdate {
        Off = "off",
        Check = "check",
        Download = "download",
    }
}

/// Background blur configuration that maps from the C API values.
/// Positive values represent blur radius, special negative values
/// represent macOS-specific glass effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundBlur {
    Disabled,
    Radius(i64),
    MacosGlassRegular,
    MacosGlassClear,
}

impl BackgroundBlur {
    /// `init(fromCValue:)`. The Swift code gates the glass cases on
    /// `#available(macOS 26.0, *)` and falls back to `.disabled`;
    /// `glass_available` is that check (use [`BackgroundBlur::from_c_value`]
    /// for the runtime one).
    #[must_use]
    pub const fn from_c_value_with(value: i16, glass_available: bool) -> Self {
        match value {
            0 => Self::Disabled,
            -1 => {
                if glass_available {
                    Self::MacosGlassRegular
                } else {
                    Self::Disabled
                }
            }
            -2 => {
                if glass_available {
                    Self::MacosGlassClear
                } else {
                    Self::Disabled
                }
            }
            _ => Self::Radius(value as i64),
        }
    }

    /// `init(fromCValue:)` using the running OS version for the glass check.
    #[must_use]
    pub fn from_c_value(value: i16) -> Self {
        Self::from_c_value_with(value, macos_26_available())
    }

    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Returns true if this is a macOS glass style (regular or clear).
    #[must_use]
    pub const fn is_glass_style(self) -> bool {
        matches!(self, Self::MacosGlassRegular | Self::MacosGlassClear)
    }

    /// Returns the blur radius if applicable, nil for glass effects.
    #[must_use]
    pub const fn radius(self) -> Option<i64> {
        match self {
            Self::Radius(r) => Some(r),
            _ => None,
        }
    }
}

/// `#available(macOS 26.0, *)`.
#[cfg(target_vendor = "apple")]
fn macos_26_available() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 26,
        minorVersion: 0,
        patchVersion: 0,
    })
}

#[cfg(not(target_vendor = "apple"))]
fn macos_26_available() -> bool {
    false
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct BellFeatures: u32 {
        const SYSTEM = 1 << 0;
        const AUDIO = 1 << 1;
        const ATTENTION = 1 << 2;
        const TITLE = 1 << 3;
        const BORDER = 1 << 4;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct SplitPreserveZoom: u32 {
        const NAVIGATION = 1 << 0;
    }
}

string_enum! {
    MacDockDropBehavior {
        NewTab = "new-tab",
        NewWindow = "new-window",
    }
}

string_enum! {
    MacHidden {
        Never = "never",
        Always = "always",
    }
}

string_enum! {
    MacShortcuts {
        Allow = "allow",
        Deny = "deny",
        Ask = "ask",
    }
}

string_enum! {
    Scrollbar {
        System = "system",
        Never = "never",
    }
}

string_enum! {
    ResizeOverlay {
        Always = "always",
        Never = "never",
        AfterFirst = "after-first",
    }
}

string_enum! {
    ResizeOverlayPosition {
        Center = "center",
        TopLeft = "top-left",
        TopCenter = "top-center",
        TopRight = "top-right",
        BottomLeft = "bottom-left",
        BottomCenter = "bottom-center",
        BottomRight = "bottom-right",
    }
}

impl ResizeOverlayPosition {
    #[must_use]
    pub const fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopCenter | Self::TopRight)
    }

    #[must_use]
    pub const fn bottom(self) -> bool {
        matches!(
            self,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight
        )
    }

    #[must_use]
    pub const fn left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }

    #[must_use]
    pub const fn right(self) -> bool {
        matches!(self, Self::TopRight | Self::BottomRight)
    }
}

string_enum! {
    WindowDecoration {
        None = "none",
        Client = "client",
        Server = "server",
        Auto = "auto",
    }
}

impl WindowDecoration {
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::None)
    }
}

string_enum! {
    NotifyOnCommandFinish {
        Never = "never",
        Unfocused = "unfocused",
        Always = "always",
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct NotifyOnCommandFinishAction: u32 {
        const BELL = 1 << 0;
        const NOTIFY = 1 << 1;
    }
}

string_enum! {
    MacOSTitlebarStyle {
        Native = "native",
        Transparent = "transparent",
        Tabs = "tabs",
        Hidden = "hidden",
    }
}

impl MacOSTitlebarStyle {
    /// `MacOSTitlebarStyle.default`.
    pub const DEFAULT: Self = Self::Transparent;
}

string_enum! {
    /// Enum for the macos-window-buttons config option
    MacOSWindowButtons {
        Visible = "visible",
        Hidden = "hidden",
    }
}

string_enum! {
    /// Enum for the macos-titlebar-proxy-icon config option
    MacOSTitlebarProxyIcon {
        Visible = "visible",
        Hidden = "hidden",
    }
}

string_enum! {
    /// Enum for auto-update-channel config option
    AutoUpdateChannel {
        Tip = "tip",
        Stable = "stable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_values_round_trip() {
        assert_eq!(
            ResizeOverlay::from_raw_value("after-first"),
            Some(ResizeOverlay::AfterFirst)
        );
        assert_eq!(ResizeOverlay::AfterFirst.raw_value(), "after-first");
        assert_eq!(
            MacOSIcon::from_raw_value("custom-style"),
            Some(MacOSIcon::CustomStyle)
        );
        assert_eq!(MacOSIcon::from_raw_value("nope"), None);
        assert_eq!(MacOSIcon::Xray.asset_name(), Some("XrayImage"));
        assert_eq!(MacOSIcon::Official.asset_name(), None);
    }

    #[test]
    fn background_blur() {
        assert_eq!(
            BackgroundBlur::from_c_value_with(0, true),
            BackgroundBlur::Disabled
        );
        assert_eq!(
            BackgroundBlur::from_c_value_with(-1, true),
            BackgroundBlur::MacosGlassRegular
        );
        assert_eq!(
            BackgroundBlur::from_c_value_with(-2, true),
            BackgroundBlur::MacosGlassClear
        );
        assert_eq!(
            BackgroundBlur::from_c_value_with(-2, false),
            BackgroundBlur::Disabled
        );
        let b = BackgroundBlur::from_c_value_with(20, false);
        assert_eq!(b.radius(), Some(20));
        assert!(b.is_enabled() && !b.is_glass_style());
    }

    #[test]
    fn overlay_position_edges() {
        assert!(ResizeOverlayPosition::TopRight.top() && ResizeOverlayPosition::TopRight.right());
        assert!(!ResizeOverlayPosition::Center.top());
        assert!(WindowDecoration::Auto.enabled() && !WindowDecoration::None.enabled());
    }
}
