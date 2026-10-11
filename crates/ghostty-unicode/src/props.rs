//! Property set per codepoint that Ghostty cares about.
//!
//! Adding to this lets you find new properties but also potentially makes
//! our lookup tables less efficient.
//!
//! NOTE: this file is also `#[path]`-included by `build.rs`, so it must not
//! depend on anything else in the crate.

/// Grapheme break property with the control/CR/LF classes removed
/// (`uucode.x.types.GraphemeBreakNoControl`). The discriminant order matches
/// uucode and is used to index the precomputed break table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum GraphemeBreakNoControl {
    #[default]
    Other = 0,
    Prepend,
    RegionalIndicator,
    SpacingMark,
    L,
    V,
    T,
    Lv,
    Lvt,
    Zwj,
    Zwnj,
    ExtendedPictographic,
    EmojiModifierBase,
    EmojiModifier,
    // extend, ==
    //   zwnj +
    //   indic_conjunct_break_extend +
    //   indic_conjunct_break_linker
    IndicConjunctBreakExtend,
    IndicConjunctBreakLinker,
    IndicConjunctBreakConsonant,
}

impl GraphemeBreakNoControl {
    /// Number of variants.
    pub const COUNT: usize = 17;

    /// Variant name in Rust syntax (used by the table generator).
    #[must_use]
    pub const fn rust_name(self) -> &'static str {
        match self {
            Self::Other => "Other",
            Self::Prepend => "Prepend",
            Self::RegionalIndicator => "RegionalIndicator",
            Self::SpacingMark => "SpacingMark",
            Self::L => "L",
            Self::V => "V",
            Self::T => "T",
            Self::Lv => "Lv",
            Self::Lvt => "Lvt",
            Self::Zwj => "Zwj",
            Self::Zwnj => "Zwnj",
            Self::ExtendedPictographic => "ExtendedPictographic",
            Self::EmojiModifierBase => "EmojiModifierBase",
            Self::EmojiModifier => "EmojiModifier",
            Self::IndicConjunctBreakExtend => "IndicConjunctBreakExtend",
            Self::IndicConjunctBreakLinker => "IndicConjunctBreakLinker",
            Self::IndicConjunctBreakConsonant => "IndicConjunctBreakConsonant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Properties {
    /// Codepoint width. We clamp to [0, 2] since Ghostty handles control
    /// characters and we max out at 2 for wide characters (i.e. 3-em dash
    /// becomes a 2-em dash). (`u2` in Zig.)
    pub width: u8,

    /// Whether the code point does not contribute to the width of a grapheme
    /// cluster (not used for single code point cells).
    pub width_zero_in_grapheme: bool,

    /// Grapheme break property.
    pub grapheme_break: GraphemeBreakNoControl,

    /// Emoji VS compatibility
    pub emoji_vs_base: bool,
}

impl Properties {
    /// Rust source expression for this value (the Zig `format`), used by the
    /// build script when writing the generated tables.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn format_rust(&self) -> String {
        format!(
            "Properties {{ width: {}, width_zero_in_grapheme: {}, grapheme_break: GraphemeBreakNoControl::{}, emoji_vs_base: {} }}",
            self.width,
            self.width_zero_in_grapheme,
            self.grapheme_break.rust_name(),
            self.emoji_vs_base,
        )
    }
}
