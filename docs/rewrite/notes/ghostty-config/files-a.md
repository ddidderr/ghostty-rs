# ghostty-config: small files (fork A)

Ported (Zig -> Rust, in crates/ghostty-config/src): command.zig->command.rs,
path.zig->path.rs, url.zig->url.rs, string.zig->string.rs, io.zig->io.rs,
RepeatableStringMap.zig->repeatable_string_map.rs,
ClipboardCodepointMap.zig->clipboard_codepoint_map.rs, theme.zig->theme.rs,
edit.zig->edit.rs, file_load.zig->file_load.rs; config-template copied.

Tests: all Zig tests ported (command 9, path 9, string 10, io 3, repeatable_string_map 4,
url regex 1 (80 cases + 10 no-match)); a handful of extra tests. 118 pass in a scratch crate
(with a ghostty-os shim; real ghostty-os didn't compile at the time).
clipboard_codepoint_map.zig and theme/edit/file_load have no Zig tests.

Deviations
- No arenas: owned Strings. `clone`/`equal` = derive Clone/PartialEq (RepeatableStringMap has a manual order-insensitive PartialEq).
- url.rs: `REGEX` is built with `const_format::concatcp!`. The pattern needs Oniguruma
  (variable-length lookbehind); `onig` is a dev-dependency used only by the test. The renderer/link
  code must use the `onig` crate (add it as a normal dependency there).
- macOS app-support paths (file_load/edit) omitted.
- theme.rs: Zig reads `global_state.resources_dir.app()`; here it's an explicit
  `resources_dir: Option<&str>` parameter (`theme::open`, `LocationIterator::new`).
  `LocationIterator::next_location()` (not `next`) to satisfy clippy.
- Error text in diagnostics uses Rust io::Error Display instead of Zig `error.Name`.
- `Command::arg_iterator` shell splitting is a hand-written equivalent of ArgIteratorGeneral(.{}).
- Zig `u21` codepoints are `u32` (string.rs, clipboard_codepoint_map.rs).

TODO(integration)
- clipboard_codepoint_map::Entry/Replacement mirror terminal/formatter.zig CodepointMap; swap for ghostty-terminal's.
- Config::expand_paths should call `Path::expand` / `RepeatablePath::expand(base, &mut diags)`.
