# ghostty-vt W1 notes

## Ported (Zig -> Rust, under crates/ghostty-vt/src)
- Parser.zig -> parser.rs (`Parser`, `State`, `TransitionAction`, `Action`, `Csi`/`Esc`/`Dcs`, `MAX_PARAMS`, `SepList`)
- parse_table.zig -> parse_table.rs (`TABLE`, built by `const fn`; identical rule order)
- UTF8Decoder.zig -> utf8_decoder.rs (`Utf8Decoder::next -> (Option<u32>, bool)`)
- osc.zig -> osc/mod.rs; osc/encoding.zig -> osc/encoding.rs; osc/parsers/*.zig -> osc/parsers/*.rs (all 15)
- color.zig -> color.rs; x11_color.zig -> x11_color.rs (res/rgb.txt copied, `include_str!`, lazy map); kitty/color.zig -> kitty/color.rs

## Deviations / API notes
- `Action` owns its data (fixed inline arrays) rather than borrowing from the parser: use `csi.params()`, `csi.intermediates()`, `csi.final_`, `csi.params_sep`. `Print(u32)`. `Parser::next` returns `[Option<Action>; 3]`.
- `Parser::new()`; `parser.osc_parser.alloc` is a `bool` ("allocator available", Zig `?Allocator`). Stream must set `osc_parser.alloc = true` as stream.zig does, otherwise OSC 4/5/10-19/104/110-119/133/21 are rejected like in Zig.
- OSC `Command` is a Rust enum (`osc::Command`, `osc::Key` via `Command::key()`); `[:0]const u8` fields are `Vec<u8>` (no NUL, not necessarily UTF-8). `Parser::end` returns an owned `Option<Command>`.
- Fixed buffer (MAX_BUF=2048, one byte reserved for NUL where Zig writes one) vs allocating writer = one `Vec<u8>` with an optional cap. `Parser::data()` (reserves NUL) / `buffered()` (does not) replace `writer.buffered()`. Remaining edge: osc9/iterm2/text_sizing use `data()` even where Zig writes no NUL for some sub-commands (only differs at exactly 2048 payload bytes).
- semantic_prompt: `Option` enum is `OptionKey`; `printf_q_decode`/`url_percent_decode` local copies (TODO(integration): move to/consume from ghostty-os string_encoding). `color::Rgb` (alias `RGB`); `DynamicRgb.override` is `override_`.
- Clippy: test modules `#![allow(clippy::unwrap_used)]`.

## Skipped tests
- `ghostty.h Command.ProgressReport.State` (C ABI), `refAllDecls`/`test { _ = ... }` comptime-reference tests, comptime `@sizeOf(Command)` assert.
- One iterm2 test is `SkipZigTest` in Zig; kept as `#[ignore]`.
- Extra tests added: osc_112 in color parser, printf-q decode in semantic_prompt.
