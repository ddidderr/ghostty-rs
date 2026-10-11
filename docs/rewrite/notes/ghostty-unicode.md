# ghostty-unicode

Port of `ZIG/src/unicode/*`.

## Module map

| Zig | Rust (`crates/ghostty-unicode`) |
|---|---|
| `main.zig` | `src/lib.rs` (re-exports `table`, `Properties`, `grapheme_break`, `BreakState`) |
| `lut.zig` | `src/lut.rs` (3-stage table generator + lookup; also included by `build.rs`) |
| `props.zig` | `src/props.rs` (`Properties`, `GraphemeBreakNoControl`) |
| `props_table.zig` | `src/props_table.rs` (`table()`; generated into `$OUT_DIR`) |
| `props_uucode.zig`, `symbols_uucode.zig` (the generators) | `build.rs` + `build_support/ucd.rs` (UCD parser and uucode-equivalent computation) |
| `symbols_table.zig` | `src/symbols_table.rs` |
| `grapheme.zig` | `src/grapheme.rs` |
| (uucode `is_emoji_presentation`, used by font `CodepointResolver`) | `src/emoji_table.rs` (extra) |

## How the data is derived (uucode)

Ghostty gets its data from the `uucode` Zig package (v0.2.0, vendored UCD =
**Unicode 17.0.0**), configured in `ZIG/src/build/uucode_config.zig`. Neither
a Rust crate nor runtime data is used; instead the exact UCD files uucode ships
(trimmed to the used properties) are vendored in `ucd/` and `build.rs`
re-implements uucode's computations, so results are identical to Ghostty:

* `ucd/extracted/DerivedGeneralCategory.txt`, `DerivedEastAsianWidth.txt`
  (incl. `@missing` Wide ranges), `auxiliary/GraphemeBreakProperty.txt`,
  `DerivedCoreProperties.txt` (trimmed to `Default_Ignorable_Code_Point` and
  `InCB`), `emoji/emoji-data.txt`, `emoji/emoji-variation-sequences.txt`,
  `Blocks.txt`, `auxiliary/GraphemeBreakTest.txt` (tests only).
  (uucode uses `UnicodeData.txt` for general category; `DerivedGeneralCategory`
  carries the same values and is 10x smaller.)
* **Width** (`uucode x/config_x/wcwidth.zig` + Ghostty `computeWidth`):
  Cc/Cs/Zl/Zp -> 0; U+00AD -> 1; Default_Ignorable -> 0; U+2E3A -> 2;
  U+2E3B -> 3 (clamped to 2 in `width`); EAW W/F -> 2; Regional Indicator -> 2;
  else 1; U+20E3 standalone = 2. `width_zero_in_grapheme` = width 0, emoji
  modifier, Mn, Me, GCB=V, GCB=T, or Prepend. Ghostty's final `width`:
  `0` if `zero_in_grapheme && !emoji_modifier && gcb != prepend`, else
  `min(2, standalone)` (so emoji modifiers and Prepend characters keep a width;
  VS15/VS16/ZWJ/combining marks are 0).
* **Grapheme break** property (`grapheme_break_no_control`): emoji modifier,
  emoji modifier base, extended pictographic, then InCB (linker / consonant /
  extend; ZWJ and ZWNJ split out), else GCB with control/CR/LF folded into
  `other` (the terminal filters controls first, as in Zig).
* **`graphemeBreak(cp1, cp2, state)`**: uucode's
  `computeGraphemeBreakNoControl` (UAX #29 incl. GB9c Indic conjuncts, GB11,
  GB12/13), with Ghostty/uucode's tailoring that emoji modifiers only extend
  an emoji modifier base. Precomputed at compile time (const fn) into a
  5 x 17 x 17 table. `grapheme_break_props(gb1, gb2, state)` takes
  already-looked-up properties (hot path for the terminal).
* **`is_symbol`** (`symbols_table::table()`): general category Co or block in
  {Arrows, Dingbats, Emoticons, Miscellaneous Symbols, Enclosed Alphanumerics,
  Enclosed Alphanumeric Supplement, Miscellaneous Symbols and Pictographs,
  Transport and Map Symbols}.
* Code points above U+10FFFF return `{width: 1, width_zero_in_grapheme: true,
  Other, emoji_vs_base: false}` (as in `props_uucode.get`).

## API differences

* Code points are `u32` (Zig `u21`). `table().get(cp)` is branch-free:
  three array loads. `table()` returns a tiny `Tables` view over static data.
* `Properties.width` is a `u8` in 0..=2 (Zig `u2`).
* Not ported: `uucode.get(.case_folding_full)` (used by `input/Binding.zig`;
  the input crate should use `char::to_lowercase`/own folding or ask for this
  later), `x.grapheme.wcwidth` (unused by Ghostty), uucode's UTF-8 iterator,
  and the `main()` brute-force verification binary in `grapheme.zig` (replaced
  by the `GraphemeBreakTest.txt` test below).

## Tests

Zig tests ported (4/4): `grapheme break: emoji modifier`,
`long emoji zwj sequences`, `unicode props: tables match uucode`
(`unicode_props_tables_match_uucode`, compares the generated table with a
direct computation from the UCD files for every code point),
`unicode symbols: tables match uucode`. Extras: official
`GraphemeBreakTest.txt` (with controls skipped and the emoji-modifier
tailoring, like uucode's own test), width tests for ASCII/CJK/emoji/combining/
ZWJ/VS15/VS16/RI/keycap/Hangul/Prepend, cluster tests (flags, ZWJ families,
skin tones, Indic conjunct), precompute-vs-compute equality, lut round-trip.

## Known gaps / notes

* Build time of the crate is a few seconds (generator walks 2M code points).
* Updating Unicode: replace files in `ucd/` (see uucode `bin/fetch-ucd.sh`).
