//! Direct computation of Ghostty's per-codepoint Unicode properties from the
//! vendored UCD files. This reproduces what `uucode` computes with Ghostty's
//! `src/build/uucode_config.zig` (and uucode's `x/config_x/wcwidth.zig` and
//! `x/config_x/grapheme_break.zig`).
//!
//! It is used by `build.rs` to generate the lookup tables, and by the
//! `props_uucode` test module to verify that the generated tables match.
//!
//! This file is the Rust counterpart of `props_uucode.zig` /
//! `symbols_uucode.zig` (`get`).
#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::single_match_else,
    clippy::checked_conversions,
    clippy::trivially_copy_pass_by_ref,
    clippy::expect_used
)]

use crate::props::{GraphemeBreakNoControl as Gb, Properties};
use std::fs;
use std::path::Path;

/// Highest codepoint known to the UCD (`uucode.config.max_code_point`).
pub const MAX_CODE_POINT: u32 = 0x10FFFF;
const N: usize = MAX_CODE_POINT as usize + 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gc {
    Cn,
    Cc,
    Cs,
    Co,
    Zl,
    Zp,
    Mn,
    Me,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Eaw {
    Neutral,
    Wide,
    Fullwidth,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OrigGb {
    Other,
    Prepend,
    Cr,
    Lf,
    Control,
    Extend,
    RegionalIndicator,
    SpacingMark,
    L,
    V,
    T,
    Lv,
    Lvt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Incb {
    None,
    Linker,
    Consonant,
    Extend,
}

#[derive(Clone, Copy, Default)]
struct EmojiData {
    presentation: bool,
    modifier: bool,
    modifier_base: bool,
    extended_pictographic: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    None,
    Symbol,
    Other,
}

pub struct Ucd {
    gc: Vec<Gc>,
    eaw: Vec<Eaw>,
    orig_gb: Vec<OrigGb>,
    incb: Vec<Incb>,
    default_ignorable: Vec<bool>,
    emoji: Vec<EmojiData>,
    emoji_vs_base: Vec<bool>,
    block: Vec<Block>,
}

fn trim(line: &str) -> &str {
    let line = match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    };
    line.trim_matches([' ', '\t', '\r'])
}

fn parse_cp(s: &str) -> u32 {
    u32::from_str_radix(s.trim(), 16).unwrap_or_else(|_| panic!("bad codepoint {s:?}"))
}

fn parse_range(s: &str) -> (u32, u32) {
    match s.split_once("..") {
        Some((a, b)) => (parse_cp(a), parse_cp(b)),
        None => {
            let c = parse_cp(s);
            (c, c)
        }
    }
}

/// Iterate `range ; value ; rest...` data lines (comments stripped).
fn data_lines(content: &str) -> impl Iterator<Item = (u32, u32, Vec<&str>)> {
    content.lines().filter_map(|line| {
        let t = trim(line);
        if t.is_empty() {
            return None;
        }
        let mut parts = t.split(';').map(str::trim);
        let (a, b) = parse_range(parts.next()?);
        Some((a, b, parts.collect()))
    })
}

fn read(dir: &Path, rel: &str) -> String {
    let p = dir.join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

fn fill<T: Copy>(v: &mut [T], a: u32, b: u32, val: T) {
    for cp in a..=b {
        v[cp as usize] = val;
    }
}

impl Ucd {
    /// Load from a directory containing the vendored UCD files.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        // General category.
        let mut gc = vec![Gc::Cn; N];
        for (a, b, parts) in data_lines(&read(dir, "extracted/DerivedGeneralCategory.txt")) {
            let v = match parts[0] {
                "Cc" => Gc::Cc,
                "Cs" => Gc::Cs,
                "Co" => Gc::Co,
                "Zl" => Gc::Zl,
                "Zp" => Gc::Zp,
                "Mn" => Gc::Mn,
                "Me" => Gc::Me,
                "Cn" => Gc::Cn,
                _ => Gc::Other,
            };
            fill(&mut gc, a, b, v);
        }

        // East Asian width, including the `@missing` directives (only
        // Neutral and Wide occur).
        let mut eaw = vec![Eaw::Neutral; N];
        let content = read(dir, "extracted/DerivedEastAsianWidth.txt");
        for line in content.lines() {
            let t = line.trim_matches([' ', '\t', '\r']);
            if let Some(rest) = t.strip_prefix("# @missing:") {
                let mut parts = rest.split(';').map(str::trim);
                let (a, b) = parse_range(parts.next().unwrap());
                match parts.next().unwrap() {
                    "Neutral" => {}
                    "Wide" => fill(&mut eaw, a, b, Eaw::Wide),
                    other => panic!("Unknown @missing EastAsianWidth value: {other}"),
                }
                continue;
            }
            let t = trim(t);
            if t.is_empty() {
                continue;
            }
            let mut parts = t.split(';').map(str::trim);
            let (a, b) = parse_range(parts.next().unwrap());
            let v = match parts.next().unwrap() {
                "W" => Eaw::Wide,
                "F" => Eaw::Fullwidth,
                "N" => Eaw::Neutral,
                _ => Eaw::Other,
            };
            fill(&mut eaw, a, b, v);
        }

        // Original grapheme break property (unknown values are `other`).
        let mut orig_gb = vec![OrigGb::Other; N];
        for (a, b, parts) in data_lines(&read(dir, "auxiliary/GraphemeBreakProperty.txt")) {
            let v = match parts[0] {
                "Prepend" => OrigGb::Prepend,
                "CR" => OrigGb::Cr,
                "LF" => OrigGb::Lf,
                "Control" => OrigGb::Control,
                "Extend" => OrigGb::Extend,
                "Regional_Indicator" => OrigGb::RegionalIndicator,
                "SpacingMark" => OrigGb::SpacingMark,
                "L" => OrigGb::L,
                "V" => OrigGb::V,
                "T" => OrigGb::T,
                "LV" => OrigGb::Lv,
                "LVT" => OrigGb::Lvt,
                _ => OrigGb::Other,
            };
            fill(&mut orig_gb, a, b, v);
        }

        // Derived core properties: Default_Ignorable_Code_Point and InCB.
        let mut incb = vec![Incb::None; N];
        let mut default_ignorable = vec![false; N];
        for (a, b, parts) in data_lines(&read(dir, "DerivedCoreProperties.txt")) {
            match parts[0] {
                "Default_Ignorable_Code_Point" => fill(&mut default_ignorable, a, b, true),
                "InCB" => {
                    let v = match parts[1] {
                        "Linker" => Incb::Linker,
                        "Consonant" => Incb::Consonant,
                        "Extend" => Incb::Extend,
                        other => panic!("Unknown InCB value: {other}"),
                    };
                    fill(&mut incb, a, b, v);
                }
                _ => {}
            }
        }

        // Emoji data.
        let mut emoji = vec![EmojiData::default(); N];
        for (a, b, parts) in data_lines(&read(dir, "emoji/emoji-data.txt")) {
            for cp in a..=b {
                let e = &mut emoji[cp as usize];
                match parts[0] {
                    "Emoji_Presentation" => e.presentation = true,
                    "Emoji_Modifier" => e.modifier = true,
                    "Emoji_Modifier_Base" => e.modifier_base = true,
                    "Extended_Pictographic" => e.extended_pictographic = true,
                    _ => {}
                }
            }
        }

        // Emoji variation sequences (base codepoints of VS15/VS16 sequences).
        let mut emoji_vs_base = vec![false; N];
        let content = read(dir, "emoji/emoji-variation-sequences.txt");
        for line in content.lines() {
            let t = trim(line);
            if t.is_empty() {
                continue;
            }
            // "0023 FE0E  ; text style;  # ..."
            let first = t.split(';').next().unwrap();
            let mut it = first.split_whitespace();
            let cp = parse_cp(it.next().unwrap());
            let vs = parse_cp(it.next().unwrap());
            assert!(vs == 0xFE0E || vs == 0xFE0F, "Unknown VS: {vs:x}");
            emoji_vs_base[cp as usize] = true;
        }

        // Blocks (only the ones that matter for `is_symbol`).
        let mut block = vec![Block::None; N];
        for (a, b, parts) in data_lines(&read(dir, "Blocks.txt")) {
            let v = match parts[0] {
                "Arrows"
                | "Dingbats"
                | "Emoticons"
                | "Miscellaneous Symbols"
                | "Enclosed Alphanumerics"
                | "Enclosed Alphanumeric Supplement"
                | "Miscellaneous Symbols and Pictographs"
                | "Transport and Map Symbols" => Block::Symbol,
                _ => Block::Other,
            };
            fill(&mut block, a, b, v);
        }

        Self {
            gc,
            eaw,
            orig_gb,
            incb,
            default_ignorable,
            emoji,
            emoji_vs_base,
            block,
        }
    }

    /// `grapheme_break_no_control` for a codepoint (uucode tables.zig +
    /// `x/config_x/grapheme_break.zig`).
    fn grapheme_break_no_control(&self, cp: u32) -> Gb {
        let i = cp as usize;
        let emoji = self.emoji[i];
        let orig = self.orig_gb[i];
        if emoji.modifier {
            Gb::EmojiModifier
        } else if emoji.modifier_base {
            Gb::EmojiModifierBase
        } else if emoji.extended_pictographic {
            Gb::ExtendedPictographic
        } else {
            match self.incb[i] {
                Incb::None => match orig {
                    OrigGb::Extend => {
                        // `zwnj` is the only grapheme break `extend` that is
                        // `none` for Indic conjunct break.
                        assert_eq!(cp, 0x200C, "unexpected extend without InCB: {cp:x}");
                        Gb::Zwnj
                    }
                    OrigGb::Control | OrigGb::Cr | OrigGb::Lf | OrigGb::Other => Gb::Other,
                    OrigGb::Prepend => Gb::Prepend,
                    OrigGb::RegionalIndicator => Gb::RegionalIndicator,
                    OrigGb::SpacingMark => Gb::SpacingMark,
                    OrigGb::L => Gb::L,
                    OrigGb::V => Gb::V,
                    OrigGb::T => Gb::T,
                    OrigGb::Lv => Gb::Lv,
                    OrigGb::Lvt => Gb::Lvt,
                },
                Incb::Extend => {
                    if cp == 0x200D {
                        Gb::Zwj
                    } else {
                        Gb::IndicConjunctBreakExtend
                    }
                }
                Incb::Linker => Gb::IndicConjunctBreakLinker,
                Incb::Consonant => Gb::IndicConjunctBreakConsonant,
            }
        }
    }

    /// Port of `x/config_x/wcwidth.zig` `compute`: returns
    /// `(wcwidth_standalone, wcwidth_zero_in_grapheme)`.
    fn wcwidth(&self, cp: u32) -> (u8, bool) {
        let i = cp as usize;
        let gc = self.gc[i];
        let gb = self.grapheme_break_no_control(cp);
        let eaw = self.eaw[i];

        let width: u8 = if matches!(gc, Gc::Cc | Gc::Cs | Gc::Zl | Gc::Zp) {
            0
        } else if cp == 0x00AD {
            // Soft hyphen
            1
        } else if self.default_ignorable[i] {
            0
        } else if cp == 0x2E3A {
            // Two-em dash
            2
        } else if cp == 0x2E3B {
            // Three-em dash
            3
        } else if eaw == Eaw::Wide || eaw == Eaw::Fullwidth || gb == Gb::RegionalIndicator {
            2
        } else {
            1
        };

        let standalone = if cp == 0x20E3 { 2 } else { width };
        let zero = width == 0
            || self.emoji[i].modifier
            || gc == Gc::Mn
            || gc == Gc::Me
            || gb == Gb::V
            || gb == Gb::T
            || gb == Gb::Prepend;
        (standalone, zero)
    }

    /// Port of `props_uucode.zig` `get`.
    #[must_use]
    pub fn get(&self, cp: u32) -> Properties {
        if cp > MAX_CODE_POINT {
            return Properties {
                width: 1,
                width_zero_in_grapheme: true,
                grapheme_break: Gb::Other,
                emoji_vs_base: false,
            };
        }
        let i = cp as usize;
        let gb = self.grapheme_break_no_control(cp);
        let (standalone, zero) = self.wcwidth(cp);

        // See uucode_config.zig `computeWidth`: emoji modifiers and prepend
        // code points keep their standalone width.
        let width = if zero && !self.emoji[i].modifier && gb != Gb::Prepend {
            0
        } else {
            standalone.min(2)
        };
        Properties {
            width,
            width_zero_in_grapheme: zero,
            grapheme_break: gb,
            emoji_vs_base: self.emoji_vs_base[i],
        }
    }

    /// Port of `symbols_uucode.zig` (`is_symbol`, see `computeIsSymbol`).
    #[must_use]
    pub fn is_symbol(&self, cp: u32) -> bool {
        if cp > MAX_CODE_POINT {
            return false;
        }
        let i = cp as usize;
        self.gc[i] == Gc::Co || self.block[i] == Block::Symbol
    }

    /// uucode `is_emoji_presentation` (used by the font codepoint resolver).
    #[must_use]
    pub fn is_emoji_presentation(&self, cp: u32) -> bool {
        cp <= MAX_CODE_POINT && self.emoji[cp as usize].presentation
    }
}
