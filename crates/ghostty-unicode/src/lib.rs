//! Unicode properties for Ghostty: codepoint width, grapheme breaking, and
//! the "is symbol" table used by the renderer for constraint width.
//!
//! Port of `src/unicode/*`. Ghostty derives these from the `uucode` package
//! configured by `src/build/uucode_config.zig` (Unicode 17.0.0); here the
//! same values are generated at build time (`build.rs`) from vendored UCD
//! files. See `docs/rewrite/notes/ghostty-unicode.md`.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod case_folding;
pub mod emoji_table;
pub mod grapheme;
pub mod lut;
pub mod props;
pub mod props_table;
pub mod symbols_table;

#[cfg(test)]
#[path = "../build_support/ucd.rs"]
mod props_uucode;

pub use grapheme::{BreakState, grapheme_break};
pub use props::{GraphemeBreakNoControl, Properties};
pub use props_table::table;

/// Unicode version of the vendored data.
pub const UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_props_tables_match_uucode() {
        let ucd =
            props_uucode::Ucd::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ucd"));
        for cp in 0..=0x1F_FFFFu32 {
            assert_eq!(table().get(cp), ucd.get(cp), "mismatch cp=U+{cp:x}");
        }
    }

    #[test]
    fn unicode_symbols_tables_match_uucode() {
        let ucd =
            props_uucode::Ucd::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ucd"));
        for cp in 0..=0x1F_FFFFu32 {
            assert_eq!(
                symbols_table::table().get(cp),
                ucd.is_symbol(cp),
                "mismatch cp=U+{cp:x}"
            );
            assert_eq!(emoji_table::table().get(cp), ucd.is_emoji_presentation(cp));
        }
    }

    fn w(cp: u32) -> u8 {
        table().get(cp).width
    }

    #[test]
    fn widths_ascii() {
        for cp in 0x20..0x7F {
            assert_eq!(w(cp), 1, "{cp:x}");
        }
        // Control characters are zero width (handled before lookup by the
        // terminal).
        for cp in (0..0x20).chain(0x7F..0xA0) {
            assert_eq!(w(cp), 0, "{cp:x}");
        }
    }

    #[test]
    fn widths_cjk_and_fullwidth() {
        assert_eq!(w('中' as u32), 2);
        assert_eq!(w('あ' as u32), 2);
        assert_eq!(w('한' as u32), 2);
        assert_eq!(w(0xFF21), 2); // fullwidth A
        assert_eq!(w(0x3000), 2); // ideographic space
        assert_eq!(w(0xFF61), 1); // halfwidth ideographic full stop
        assert_eq!(w(0x20000), 2); // CJK ext B
        assert_eq!(w(0x3FFFD), 2);
        // Ambiguous width is narrow.
        assert_eq!(w(0x00B1), 1);
        assert_eq!(w(0x2460), 1);
    }

    #[test]
    fn widths_emoji() {
        assert_eq!(w(0x1F600), 2); // grinning face
        assert_eq!(w(0x1F680), 2);
        assert_eq!(w(0x2764), 1); // heart, text default
        assert_eq!(w(0x231A), 2); // watch, emoji presentation
        assert!(table().get(0x2764).emoji_vs_base);
        assert!(!table().get(0x1F600).emoji_vs_base);
        assert!(emoji_table::table().get(0x1F600));
        assert!(!emoji_table::table().get(0x2764));
    }

    #[test]
    fn widths_zero_and_combining() {
        assert_eq!(w(0x0301), 0); // combining acute
        assert_eq!(w(0x20DD), 0); // enclosing circle
        assert_eq!(w(0x200B), 0); // ZWSP (default ignorable)
        assert_eq!(w(0x200D), 0); // ZWJ
        assert_eq!(w(0x200C), 0); // ZWNJ
        assert_eq!(w(0xFE0E), 0); // VS15
        assert_eq!(w(0xFE0F), 0); // VS16
        assert_eq!(w(0xE0100), 0); // VS17
        assert_eq!(w(0x2028), 0); // line separator
        assert_eq!(w(0xD800), 0); // surrogate
        assert_eq!(w(0x00AD), 1); // soft hyphen
        assert_eq!(w(0x2E3A), 2); // two-em dash
        assert_eq!(w(0x2E3B), 2); // three-em dash clamped to 2
        for cp in [0x0301, 0x200D, 0xFE0F, 0x1F3FB, 0x1160, 0x11A8] {
            assert!(table().get(cp).width_zero_in_grapheme, "{cp:x}");
        }
        assert!(!table().get('a' as u32).width_zero_in_grapheme);
        assert!(!table().get(0x1F600).width_zero_in_grapheme);
    }

    #[test]
    fn widths_emoji_modifier_prepend_ri_keycap() {
        // Emoji modifiers display as colour patches on their own.
        for cp in 0x1F3FB..=0x1F3FF {
            assert_eq!(w(cp), 2);
            assert!(table().get(cp).width_zero_in_grapheme);
            assert_eq!(
                table().get(cp).grapheme_break,
                GraphemeBreakNoControl::EmojiModifier
            );
        }
        // Prepend characters keep a width.
        assert_eq!(w(0x0600), 1); // Arabic number sign (prepend, Cf)
        assert_eq!(
            table().get(0x0600).grapheme_break,
            GraphemeBreakNoControl::Prepend
        );
        assert!(table().get(0x0600).width_zero_in_grapheme);
        assert_eq!(w(0x0D4E), 1); // Malayalam letter dot reph
        // Regional indicators are wide.
        assert_eq!(w(0x1F1E6), 2);
        assert_eq!(w(0x1F1FF), 2);
        assert_eq!(
            table().get(0x1F1E6).grapheme_break,
            GraphemeBreakNoControl::RegionalIndicator
        );
        // Combining enclosing keycap is zero width in the table.
        assert_eq!(w(0x20E3), 0);
        // Hangul jamo.
        assert_eq!(w(0x1100), 2); // choseong
        assert_eq!(w(0x1160), 0); // jungseong, zero (V)
        assert_eq!(w(0x11A8), 0); // jongseong (T)
    }

    #[test]
    fn out_of_range_defaults() {
        // Beyond the Unicode range (props_uucode.zig `get`).
        let p = table().get(0x11_0000);
        assert_eq!(p.width, 1);
        assert!(p.width_zero_in_grapheme);
        assert_eq!(p.grapheme_break, GraphemeBreakNoControl::Other);
        assert!(!p.emoji_vs_base);
        assert_eq!(table().get(0x1F_FFFF), p);
    }

    #[test]
    fn symbols() {
        let t = symbols_table::table();
        assert!(t.get(0xE000)); // private use
        assert!(t.get(0xF0001)); // supplementary private use
        assert!(t.get(0x2190)); // arrows
        assert!(t.get(0x2713)); // dingbats
        assert!(t.get(0x1F600)); // emoticons
        assert!(t.get(0x2600)); // misc symbols
        assert!(t.get(0x2460)); // enclosed alphanumerics
        assert!(t.get(0x1F100)); // enclosed alphanumeric supplement
        assert!(t.get(0x1F300)); // misc symbols & pictographs
        assert!(t.get(0x1F680)); // transport & map
        assert!(!t.get('a' as u32));
        assert!(!t.get('中' as u32));
        assert!(!t.get(0x2500)); // box drawing
        assert!(!t.get(0x11_0000));
    }
}
