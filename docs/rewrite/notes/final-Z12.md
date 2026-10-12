# Z12: Zig-exact numeric parsing in protocol / non-config code

`zig_fmt` moved to the new leaf crate `crates/ghostty-zigfmt` (no deps);
`ghostty_os::zig_fmt` is now `pub use ghostty_zigfmt as zig_fmt`, so existing
callers are unchanged. `ghostty-vt` and `ghostty-terminal` depend on it
directly (ghostty-os depends on ghostty-vt, so vt could not use os).
Expected values in the new tests were cross-checked with the real `zig 0.16`
(`std.fmt.parseInt/parseUnsigned/parseFloat`).

| Rust call site | Zig reference | Status |
|---|---|---|
| vt `kitty/graphics_command.rs::parse_int` | graphics_command.zig:262 `parseInt(i32/u32, _, 10)` | fixed (was hand-rolled: no `_`) |
| vt `osc/parsers/kitty_text_sizing.rs::parse_u4` | kitty_text_sizing.zig:52 `parseInt(u4,10)` | fixed |
| vt `osc/parsers/color.rs::parse_u9` | parsers/color.zig:176,249 `parseInt(u9,10)` | fixed |
| vt `color.rs::parse_unsigned_hex_u16` | color.zig:505 `parseUnsigned(u16,16)` | fixed (Rust wrongly allowed `+`, `-0`) |
| vt `color.rs::from_intensity` | color.zig:483 `parseFloat(f64)` | fixed (`str::parse` -> `parse_float`: hex floats, `_`) |
| vt `osc/parsers/osc9.rs::parse_unsigned` | osc9.zig:27,195 `parseUnsigned(u16/usize,10)` | fixed (was `str::parse`, accepted `+`) |
| vt `osc/parsers/semantic_prompt.rs` exit_code | semantic_prompt.zig:155 `parseInt(i32,10)` | fixed |
| vt `osc/parsers/context_signal.rs` pid/status | context_signal.zig:167 (digit pre-check, `parseInt(u64)`) | fixed |
| vt `kitty/color.rs::Kind::parse` | kitty/color.zig:51 `parseUnsigned(u8,10)` | fixed (no `+` accepted) |
| vt `tmux/control.rs::digits` | control.zig regex `[0-9]+` then `parseInt(usize)` | unchanged: regex guarantees digits only |
| terminal `tmux/layout.rs::parse_usize` | layout.zig:99-163 `parseInt(usize,10)` | fixed (`_`, `-0`) |
| terminal `tmux/output.rs::parse_int` | output.zig:200-214 | fixed (`_`) |
| terminal `tmux/viewer.rs` pane_tabs | viewer.zig:1026 `parseInt(usize,10) catch continue` | fixed |
| os `uri.rs` port | std.Uri `parseInt(u16,10)` | fixed (`_`, no behaviour change for `+`) |
| os `uri.rs::percent_decode` | std.Uri formatRaw `parseInt(u8,"XX",16)` | fixed (accepts `%+A`, `%-0`; rejects nothing new) |
| bench `args.rs::take` | cli/args.zig:429 `parseInt(T,_,0)` / `parseFloat` | fixed (base 0 prefixes, `_`, hex float) |
| vt `parser.rs` / `stream.rs` param_acc | Parser.zig:353, stream.zig:673 saturating digit loop | already identical |
| font `shaper/feature.rs` | feature.zig:133-153 digit loop (u32 mul) | already identical |
| config / input binding / cli ssh-cache / os desktop | Z11 | already using zig_fmt |
| vt `x11_color.rs`, `build.rs` (rgb.txt) | x11_color.zig:30 comptime on fixed data | n/a (data file) |
| cli `list_themes/preview.rs` COLORFGBG, core `GHOSTTY_DEBUG_SCREENSHOT_MS`, apprt-gtk debug_input, build-info/fake_version, renderer shadertoy | no Zig counterpart (Rust-only) | left on `str::parse` |
