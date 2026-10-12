//! Grapheme break detection (`grapheme.zig`), backed by uucode's
//! `x.grapheme.computeGraphemeBreakNoControl` precomputed for every
//! combination of state and break properties.

use crate::{props::GraphemeBreakNoControl as Gb, props_table::table};

/// State carried between sequential [`grapheme_break`] calls
/// (`uucode.grapheme.BreakState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum BreakState {
    #[default]
    Default = 0,
    RegionalIndicator,
    ExtendedPictographic,
    IndicConjunctBreakConsonant,
    IndicConjunctBreakLinker,
}

const ALL_STATES: [BreakState; 5] = [
    BreakState::Default,
    BreakState::RegionalIndicator,
    BreakState::ExtendedPictographic,
    BreakState::IndicConjunctBreakConsonant,
    BreakState::IndicConjunctBreakLinker,
];

const ALL_GB: [Gb; Gb::COUNT] = [
    Gb::Other,
    Gb::Prepend,
    Gb::RegionalIndicator,
    Gb::SpacingMark,
    Gb::L,
    Gb::V,
    Gb::T,
    Gb::Lv,
    Gb::Lvt,
    Gb::Zwj,
    Gb::Zwnj,
    Gb::ExtendedPictographic,
    Gb::EmojiModifierBase,
    Gb::EmojiModifier,
    Gb::IndicConjunctBreakExtend,
    Gb::IndicConjunctBreakLinker,
    Gb::IndicConjunctBreakConsonant,
];

const fn is_indic_conjunct_break_extend(gb: Gb) -> bool {
    matches!(gb, Gb::IndicConjunctBreakExtend | Gb::Zwj)
}

// Despite `emoji_modifier` being `extend` according to
// GraphemeBreakProperty.txt and UAX #29 (in addition to tests in
// GraphemeBreakTest.txt), UTS #51 states: `emoji_modifier_sequence :=
// emoji_modifier_base emoji_modifier` in ED-13 (emoji modifier sequence) under
// 1.4.4 (Emoji Modifiers), and: "When used alone, the default representation
// of these modifier characters is a color swatch... To have an effect on an
// emoji, an emoji modifier must immediately follow that base emoji
// character." in 2.4 (Diversity). Additionally it states "Skin tone
// modifiers and hair components should be displayed even in isolation" in
// ED-20 (basic emoji set) under 1.4.6 (Emoji Sets). Here we decide to diverge
// from the grapheme break spec, which is allowed under "tailored" grapheme
// clusters.
const fn is_extend(gb: Gb) -> bool {
    matches!(
        gb,
        Gb::Zwnj | Gb::IndicConjunctBreakExtend | Gb::IndicConjunctBreakLinker
    )
}

const fn is_extended_pictographic(gb: Gb) -> bool {
    matches!(gb, Gb::ExtendedPictographic | Gb::EmojiModifierBase)
}

/// Port of uucode `x.grapheme.computeGraphemeBreakNoControl`: the rules for
/// `control`, `cr`, and `lf` are ignored, since they are assumed to have been
/// handled prior or stripped from the input.
#[must_use]
pub const fn compute_grapheme_break_no_control(gb1: Gb, gb2: Gb, state: &mut BreakState) -> bool {
    // Set state back to default when `gb1` or `gb2` is not expected in sequence.
    match *state {
        BreakState::RegionalIndicator => {
            if !matches!(gb1, Gb::RegionalIndicator) || !matches!(gb2, Gb::RegionalIndicator) {
                *state = BreakState::Default;
            }
        }
        BreakState::ExtendedPictographic => {
            if !matches!(
                gb1,
                Gb::IndicConjunctBreakExtend
                    | Gb::IndicConjunctBreakLinker
                    | Gb::Zwnj
                    | Gb::Zwj
                    | Gb::ExtendedPictographic
                    | Gb::EmojiModifierBase
                    | Gb::EmojiModifier
            ) {
                *state = BreakState::Default;
            }
            if !matches!(
                gb2,
                Gb::IndicConjunctBreakExtend
                    | Gb::IndicConjunctBreakLinker
                    | Gb::Zwnj
                    | Gb::Zwj
                    | Gb::ExtendedPictographic
                    | Gb::EmojiModifierBase
                    | Gb::EmojiModifier
            ) {
                *state = BreakState::Default;
            }
        }
        BreakState::IndicConjunctBreakConsonant | BreakState::IndicConjunctBreakLinker => {
            if !matches!(
                gb1,
                Gb::IndicConjunctBreakConsonant
                    | Gb::IndicConjunctBreakLinker
                    | Gb::IndicConjunctBreakExtend
                    | Gb::Zwj
            ) {
                *state = BreakState::Default;
            }
            if !matches!(
                gb2,
                Gb::IndicConjunctBreakConsonant
                    | Gb::IndicConjunctBreakLinker
                    | Gb::IndicConjunctBreakExtend
                    | Gb::Zwj
            ) {
                *state = BreakState::Default;
            }
        }
        BreakState::Default => {}
    }

    // GB6: L x (L | V | LV | VT)
    if matches!(gb1, Gb::L) && matches!(gb2, Gb::L | Gb::V | Gb::Lv | Gb::Lvt) {
        return false;
    }

    // GB7: (LV | V) x (V | T)
    if matches!(gb1, Gb::Lv | Gb::V) && matches!(gb2, Gb::V | Gb::T) {
        return false;
    }

    // GB8: (LVT | T) x T
    if matches!(gb1, Gb::Lvt | Gb::T) && matches!(gb2, Gb::T) {
        return false;
    }

    // Handle GB9 (Extend | ZWJ) later, since it can also match the start of
    // GB9c (Indic) and GB11 (Emoji ZWJ)

    // GB9a: SpacingMark
    if matches!(gb2, Gb::SpacingMark) {
        return false;
    }

    // GB9b: Prepend
    if matches!(gb1, Gb::Prepend) {
        return false;
    }

    // GB9c: Indic
    if matches!(gb1, Gb::IndicConjunctBreakConsonant) {
        // start of sequence:
        if is_indic_conjunct_break_extend(gb2) {
            *state = BreakState::IndicConjunctBreakConsonant;
            return false;
        } else if matches!(gb2, Gb::IndicConjunctBreakLinker) {
            // jump straight to linker state
            *state = BreakState::IndicConjunctBreakLinker;
            return false;
        }
        // else, not an Indic sequence
    } else if matches!(*state, BreakState::IndicConjunctBreakConsonant) {
        // consonant state:
        if matches!(gb2, Gb::IndicConjunctBreakLinker) {
            // consonant -> linker transition
            *state = BreakState::IndicConjunctBreakLinker;
            return false;
        } else if is_indic_conjunct_break_extend(gb2) {
            // continue [extend]* sequence
            return false;
        }
        // Not a valid Indic sequence
        *state = BreakState::Default;
    } else if matches!(*state, BreakState::IndicConjunctBreakLinker) {
        // linker state:
        if matches!(gb2, Gb::IndicConjunctBreakLinker) || is_indic_conjunct_break_extend(gb2) {
            // continue [extend linker]* sequence
            return false;
        } else if matches!(gb2, Gb::IndicConjunctBreakConsonant) {
            // linker -> end of sequence
            *state = BreakState::Default;
            return false;
        }
        // Not a valid Indic sequence
        *state = BreakState::Default;
    }

    // GB11: Emoji ZWJ sequence and Emoji modifier sequence
    if is_extended_pictographic(gb1) {
        // start of sequence:
        if is_extend(gb2) || matches!(gb2, Gb::Zwj) {
            *state = BreakState::ExtendedPictographic;
            return false;
        }

        // The `emoji_modifier_sequence` case is described in the comment for
        // `is_extend` above, from UTS #51.
        if matches!(gb1, Gb::EmojiModifierBase) && matches!(gb2, Gb::EmojiModifier) {
            *state = BreakState::ExtendedPictographic;
            return false;
        }
        // else, not an Emoji ZWJ sequence
    } else if matches!(*state, BreakState::ExtendedPictographic) {
        // continue or end sequence:
        if (is_extend(gb1) || matches!(gb1, Gb::EmojiModifier))
            && (is_extend(gb2) || matches!(gb2, Gb::Zwj))
        {
            // continue extend* ZWJ sequence
            return false;
        } else if matches!(gb1, Gb::Zwj) && is_extended_pictographic(gb2) {
            // ZWJ -> end of sequence
            *state = BreakState::Default;
            return false;
        }
        // Not a valid Emoji ZWJ sequence
        *state = BreakState::Default;
    }

    // GB12 and GB13: Regional Indicator
    if matches!(gb1, Gb::RegionalIndicator) && matches!(gb2, Gb::RegionalIndicator) {
        if matches!(*state, BreakState::Default) {
            *state = BreakState::RegionalIndicator;
            return false;
        }
        *state = BreakState::Default;
        return true;
    }

    // GB9: x (Extend | ZWJ)
    if is_extend(gb2) || matches!(gb2, Gb::Zwj) {
        return false;
    }

    // GB999: Otherwise, break everywhere
    true
}

/// Result of one table lookup: the break decision and the next state.
#[derive(Debug, Clone, Copy)]
struct Value {
    result: bool,
    state: BreakState,
}

const N_GB: usize = Gb::COUNT;
const N_STATE: usize = 5;

/// Precomputed lookup table for all possible permutations of state and
/// grapheme break properties (5 states x 17 x 17 = 1445 entries).
static PRECOMPUTE: [Value; N_STATE * N_GB * N_GB] = {
    let mut data = [Value {
        result: false,
        state: BreakState::Default,
    }; N_STATE * N_GB * N_GB];
    let mut s = 0;
    while s < N_STATE {
        let mut a = 0;
        while a < N_GB {
            let mut b = 0;
            while b < N_GB {
                let mut state = ALL_STATES[s];
                let result = compute_grapheme_break_no_control(ALL_GB[a], ALL_GB[b], &mut state);
                data[index(ALL_GB[a], ALL_GB[b], ALL_STATES[s])] = Value { result, state };
                b += 1;
            }
            a += 1;
        }
        s += 1;
    }
    data
};

const fn index(gb1: Gb, gb2: Gb, state: BreakState) -> usize {
    (state as usize) * N_GB * N_GB + (gb1 as usize) * N_GB + (gb2 as usize)
}

/// Determines if there is a grapheme break between two codepoints. This
/// must be called sequentially maintaining the state between calls.
///
/// This function does NOT work with control characters. Control characters,
/// line feeds, and carriage returns are expected to be filtered out before
/// calling this function. This is because this function is tuned for
/// Ghostty.
#[inline]
pub fn grapheme_break(cp1: u32, cp2: u32, state: &mut BreakState) -> bool {
    let t = table();
    grapheme_break_props(t.get(cp1).grapheme_break, t.get(cp2).grapheme_break, state)
}

/// Same as [`grapheme_break`] but with already-looked-up break properties.
#[inline]
pub fn grapheme_break_props(gb1: Gb, gb2: Gb, state: &mut BreakState) -> bool {
    let value = PRECOMPUTE[index(gb1, gb2, *state)];
    *state = value.state;
    value.result
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn grapheme_break_emoji_modifier() {
        // Emoji and modifier
        {
            let mut state = BreakState::Default;
            assert!(!grapheme_break(0x261D, 0x1F3FF, &mut state));
        }

        // Non-emoji and emoji modifier
        {
            let mut state = BreakState::Default;
            assert!(grapheme_break(0x22, 0x1F3FF, &mut state));
        }
    }

    #[test]
    fn long_emoji_zwj_sequences() {
        let mut state = BreakState::Default;
        // (family: woman, woman, girl, boy)
        let cps: Vec<u32> = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}_"
            .chars()
            .map(|c| c as u32)
            .collect();
        assert_eq!(cps[0], 0x1F469);
        for (i, pair) in cps.windows(2).enumerate() {
            let brk = grapheme_break(pair[0], pair[1], &mut state);
            // Only the break between the last emoji and `_` is a break.
            assert_eq!(brk, i == 6, "index {i}");
        }
    }

    fn breaks(s: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut state = BreakState::Default;
        let mut start = 0;
        let mut it = s.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            let end = i + c.len_utf8();
            match it.peek() {
                None => {
                    out.push(&s[start..end]);
                }
                Some(&(_, n)) => {
                    if grapheme_break(c as u32, n as u32, &mut state) {
                        out.push(&s[start..end]);
                        start = end;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn grapheme_clusters() {
        assert_eq!(breaks("abc"), ["a", "b", "c"]);
        // Combining mark
        assert_eq!(breaks("e\u{301}x"), ["e\u{301}", "x"]);
        // VS15 / VS16
        assert_eq!(breaks("\u{2764}\u{FE0F}x"), ["\u{2764}\u{FE0F}", "x"]);
        assert_eq!(breaks("\u{2764}\u{FE0E}x"), ["\u{2764}\u{FE0E}", "x"]);
        // Keycap
        assert_eq!(breaks("1\u{FE0F}\u{20E3}2"), ["1\u{FE0F}\u{20E3}", "2"]);
        // Skin tone and ZWJ with rocket
        assert_eq!(breaks("👩🏽‍🚀🇨🇭👨🏻‍🍼"), ["👩🏽‍🚀", "🇨🇭", "👨🏻‍🍼"]);
        // Sequence of regional indicators: pairs.
        assert_eq!(
            breaks("\u{1F1FA}\u{1F1F8}\u{1F1E6}\u{1F1F9}\u{1F1FC}_\u{1F1F3}_"),
            [
                "\u{1F1FA}\u{1F1F8}",
                "\u{1F1E6}\u{1F1F9}",
                "\u{1F1FC}",
                "_",
                "\u{1F1F3}",
                "_"
            ]
        );
        // Kiss: man, man, light skin tone, dark skin tone (with modifiers).
        assert_eq!(
            breaks(
                "\u{1F468}\u{1F3FB}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F48B}\u{200D}\u{1F468}\u{1F3FF}_"
            ),
            [
                "\u{1F468}\u{1F3FB}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F48B}\u{200D}\u{1F468}\u{1F3FF}",
                "_"
            ]
        );
        // Lone emoji modifier after non-base breaks.
        assert_eq!(breaks("a\u{1F3FB}"), ["a", "\u{1F3FB}"]);
        // Hangul syllable sequence L V T
        assert_eq!(
            breaks("\u{1100}\u{1161}\u{11A8}"),
            ["\u{1100}\u{1161}\u{11A8}"]
        );
        // Prepend
        assert_eq!(breaks("\u{0600}a"), ["\u{0600}a"]);
        // Devanagari conjunct (GB9c): KA + virama + SSA
        assert_eq!(
            breaks("\u{0915}\u{094D}\u{0937}"),
            ["\u{0915}\u{094D}\u{0937}"]
        );
        // Devanagari no virama: break
        assert_eq!(breaks("\u{0915}\u{0937}"), ["\u{0915}", "\u{0937}"]);
    }

    /// Runs the official GraphemeBreakTest.txt (with controls skipped and the
    /// emoji-modifier tailoring applied, as in uucode's own test).
    #[test]
    fn grapheme_break_test_txt() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("ucd/auxiliary/GraphemeBreakTest.txt");
        let content = std::fs::read_to_string(path).unwrap();
        let t = table();
        let mut checked = 0;
        'lines: for (ln, line) in content.lines().enumerate() {
            let line = line.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let mut toks = line.split(' ');
            assert_eq!(toks.next(), Some("÷"));
            // Tokens: cp op cp op cp ... op(÷)
            let mut cps = Vec::new();
            let mut ops = Vec::new();
            while let Some(cp) = toks.next() {
                cps.push(u32::from_str_radix(cp, 16).unwrap());
                ops.push(toks.next().unwrap());
            }
            assert_eq!(*ops.last().unwrap(), "÷");
            // Skip lines with control/CR/LF (their break property was folded
            // into `other`; handled by the caller in Ghostty).
            for &cp in &cps {
                if cp == 0x0D
                    || cp == 0x0A
                    || (cp < 0x20)
                    || (0x7F..0xA0).contains(&cp)
                    || [0x2028, 0x2029].contains(&cp)
                    || is_gb_control(cp)
                {
                    continue 'lines;
                }
            }
            let mut state = BreakState::Default;
            for i in 0..cps.len() - 1 {
                let gb1 = t.get(cps[i]).grapheme_break;
                let gb2 = t.get(cps[i + 1]).grapheme_break;
                let mut expected = ops[i] == "÷";
                // Ghostty/uucode diverge from UAX #29 for emoji modifiers.
                if gb2 == Gb::EmojiModifier && gb1 != Gb::EmojiModifierBase {
                    assert!(!expected);
                    expected = true;
                }
                let actual = grapheme_break(cps[i], cps[i + 1], &mut state);
                assert_eq!(actual, expected, "line {} idx {i}: {line}", ln + 1);
                checked += 1;
            }
        }
        assert!(checked > 800, "checked {checked}");
    }

    /// Codepoints whose original Grapheme_Cluster_Break is Control.
    fn is_gb_control(cp: u32) -> bool {
        // Cc, Zl/Zp, and format characters (excluding ZWNJ/ZWJ and prepend)
        // that are `Control` in GraphemeBreakProperty.txt.
        matches!(cp, 0x00AD | 0x061C | 0x180E | 0x200B | 0x200E | 0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF | 0xFFF0..=0xFFFB | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE001F | 0xE0080..=0xE00FF | 0xE01F0..=0xE0FFF)
    }

    #[test]
    fn precompute_matches_compute() {
        for &s in &ALL_STATES {
            for &a in &ALL_GB {
                for &b in &ALL_GB {
                    let mut s1 = s;
                    let r1 = compute_grapheme_break_no_control(a, b, &mut s1);
                    let mut s2 = s;
                    let r2 = grapheme_break_props(a, b, &mut s2);
                    assert_eq!((r1, s1), (r2, s2));
                }
            }
        }
    }
}
