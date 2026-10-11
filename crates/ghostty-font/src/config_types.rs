//! Small local mirrors of the `ghostty-config` types that the font engine
//! consumes.
//!
//! TODO(integration): `ghostty-config` should map its own
//! `FreetypeLoadFlags`, `FontSyntheticStyle` and `FontShapingBreak` onto
//! these (or this crate should re-export the config ones once the config
//! crate exists in the dependency graph).

/// See `freetype-load-flags` in the config. Defaults match upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FreetypeLoadFlags {
    pub hinting: bool,
    pub force_autohint: bool,
    pub monochrome: bool,
    pub autohint: bool,
    pub light: bool,
}

impl Default for FreetypeLoadFlags {
    fn default() -> Self {
        Self {
            hinting: true,
            force_autohint: false,
            monochrome: false,
            autohint: true,
            light: true,
        }
    }
}

/// See `font-synthetic-style` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontSyntheticStyle {
    pub bold: bool,
    pub italic: bool,
    pub bold_italic: bool,
}

impl Default for FontSyntheticStyle {
    fn default() -> Self {
        Self {
            bold: true,
            italic: true,
            bold_italic: true,
        }
    }
}

/// See `font-shaping-break` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontShapingBreak {
    pub cursor: bool,
}

impl Default for FontShapingBreak {
    fn default() -> Self {
        Self { cursor: true }
    }
}
