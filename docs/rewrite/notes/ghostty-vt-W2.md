# ghostty-vt W2 notes

## Ported (Zig -> Rust, all in crates/ghostty-vt/src)
- csi.zig -> csi.rs (EraseDisplay, EraseLine/TabClear with `Other(u8)` for the non-exhaustive Zig enums, SizeReportStyle, TitlePushPop)
- sgr.zig -> sgr.rs (`Attribute<'a>`, `Parser<'a>` implementing `Iterator`; uses `crate::color::{Name, Rgb}` and `crate::parser::{SepList, MAX_PARAMS}`)
- modes.zig -> modes.rs (`Mode` enum, `ENTRIES` table, `ModePacked(u64)`, `ModeState`, `mode_from_int`)
- charsets.zig -> charsets.rs (const-built 256-entry tables)
- ansi.zig -> ansi.rs; device_status.zig -> device_status.rs; mouse_shape.zig -> mouse_shape.rs
- kitty/key.zig: three remaining tests added to existing kitty/key.rs
- kitty/graphics_command.zig -> kitty/graphics_command.rs (parser + Response + all command structs; `pub mod graphics_command` in kitty/mod.rs)
- apc.zig -> apc.rs; dcs.zig -> dcs.rs; tmux/control.zig -> tmux/control.rs (+ tmux/mod.rs, only control)
- Dependencies added: `thiserror`, `base64`.

## Deviations
- Non-exhaustive Zig enums (EraseLine, TabClear, RenditionAspect, StatusLineType) are Rust enums with `Other(raw)`; C0 uses `from_int -> Option`.
- tmux Notification owns its data (`Vec<u8>`) instead of borrowing the parser buffer. Oniguruma regexes replaced by hand-written matchers with the same greedy semantics. IDs overflowing usize are treated as non-match (Zig: `unreachable`).
- DCS/APC handlers drop allocator params; `tmux` state holds `Box<tmux::control::Parser>`.
- graphics_command base64: `simd.base64` replaced by `base64` crate (standard alphabet, padding optional, ASCII whitespace ignored like simdutf). zlib is not decoded in the parser (Zig does not either), so no flate2 dependency.
- `Parser::complete` moves the decoded payload out (Zig `decodeData` also consumes the buffer).
- modes: Zig tests `ModePacked` size == 8; Rust has an equivalent `packed_size` test (u64). Mode bit index = enum discriminant = position in ENTRIES.
- sgr uses a bounds-tolerant `sep_is_set` (Zig's StaticBitSet would assert past MAX_PARAMS).
- `Attribute` variant names: `Fg8/Bg8/Fg8Bright/Bg8Bright/Fg256/Bg256/UnderlineColor256` for Zig `@"8_fg"` etc.

## Skipped (Zig-only)
- sgr "Attribute C compat", `Attribute.C`/`cval`/`lib.Enum`/`lib.TaggedUnion` (C ABI).
- mouse_shape "ghostty.h MouseShape" (C header check) and GObject type registration (belongs to apprt-gtk; `MouseShape` is `repr(i32)` in declaration order).
- modes `test { _ = Mode; _ = ModePacked }` and charsets/tests reflection-only bits (replaced by explicit tests).
- Allocator-failure aspects (`deinit`, arenas) have no Rust equivalent.

## Known gaps / integration
- dcs.rs uses `crate::parser::Dcs` (W1). `sgr`/`dcs` depend on W1 modules compiling.
- tmux layout/output/viewer are NOT here (ghostty-terminal).
