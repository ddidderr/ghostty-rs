# ghostty-config: types_a.rs / font_types.rs (fork B1)

Ported from Config.zig 5196-6376 and 7922-8566:
ConfirmCloseSurface, CustomShaderAnimation, NonNativeFullscreen, Fullscreen,
WindowPaddingColor, WindowSubtitle, LinkPreviews, WorkingDirectory, Color,
TerminalColor, BoldColor, ColorList, Palette, RepeatableString,
SelectionWordChars, RepeatableFontVariation, RepeatableCodepointMap,
RepeatableClipboardCodepointMap, FontStyle, FontSyntheticStyle,
FontShapingBreak, RepeatableLink, plus `CursorStyle` (from terminal/cursor.zig).

`font_types.rs`: local `Variation`/`VariationId`, `Descriptor`, `CodepointMap`
(+`CodepointMapEntry`) — ghostty-font should re-use these.

## API notes
- Zig static `X.parseCLI(input)` constructors are `Color::from_cli`,
  `TerminalColor::from_cli`, `BoldColor::from_cli` (the trait method
  `parse_cli(&mut self, ..)` wraps them). `Color::from_hex`, `format_buf`,
  `to_terminal_rgb` exist. `WorkingDirectory::finalize()` takes no allocator.
- Codepoints are `u32` (no u21 in Rust); parsing range-checks to <= 0x1FFFFF.
- `RepeatableString` PartialEq ignores `overwrite_next` (as Zig `equal`).
- Type-specific Zig errors (Overflow, InvalidCharacter, NotImplemented) are
  `ArgError::Other("<name>")` without the `error.` prefix; core cli_args
  prints `unknown error <name>`. Zig prints `error.<name>` there; core may want
  to add the prefix.
- Skipped: C API (`cval`, `*_c`), `enum(c_int)`, getGObjectType.
- UnicodeRangeParser is `pub(crate)` in types_a (`next_range`).
- RepeatableLink.parse_cli always errors `NotImplemented` (as Zig).

## Tests
All Zig tests in the ranges ported (WorkingDirectory 2, Color 4, TerminalColor 2,
BoldColor 2, ColorList 3, Palette 5, RepeatableString 5, SelectionWordChars 4,
RepeatableFontVariation 3, RepeatableCodepointMap 4, RepeatableClipboardCodepointMap 5,
FontStyle 4) plus extras, and font_types (Variation.Id x2, codepointmap). None skipped.
Test names are nested in per-type modules (`tests::color::parse_cli_with_whitespace`).
