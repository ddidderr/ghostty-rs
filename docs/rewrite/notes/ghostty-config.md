# ghostty-config port notes (W6)

Sub-notes from the parallel forks live in `ghostty-config/` (`files-a.md`,
`types-a.md`, `types-b.md`, `keybinds.md`).

## What was ported (Zig -> Rust, all in `crates/ghostty-config/src/`)

| Zig | Rust |
|---|---|
| cli/args.zig | `cli_args.rs` (`ConfigField` + `CliTarget` traits, `parse`, `LineIterator`, `ArgsIterator`, `parse_auto_struct_with`, `parse_packed_struct_with`, `split_tagged_union`, `compatibility_renamed`) |
| cli/CommaSplitter.zig | `comma_splitter.rs` |
| cli/diagnostics.zig | `diagnostics.rs` |
| config/Config.zig (fields + methods) | `config.rs` (fields declared in a single `config_struct!` invocation: name, type, default, verbatim doc comment), `keys.rs` (one `Key` const per option) |
| Config.zig nested types | `types_a.rs`, `types_b.rs`, `keybinds.rs`, `font_types.rs`, `metric_modifier.rs`, `input_fields.rs` (ConfigField impls for ghostty-input types) |
| conditional.zig, key.zig, ErrorList.zig | `conditional.rs`, `key.rs`, `error_list.rs` |
| formatter.zig | `formatter.rs` (EntryFormatter) + `file_formatter.rs` (FileFormatter) |
| command/path/url/string/io/theme/edit/file_load/RepeatableStringMap/ClipboardCodepointMap | same names, snake_case |
| config/config-template, config/testdata | `src/config-template`, `src/testdata/` |
| helpgen.zig (config part) | `macros::help_string` (computed at runtime from the doc attributes) |

Skipped: CApi.zig, c_get.zig, Wasm.zig, all `cval`/`*_c` members, `enum(c_int)`
discriminants, `getGObjectType`.

## Design

* No comptime reflection: `config_struct!` (macros.rs) generates the struct,
  `parse_field` (== `parseIntoField`, incl. "empty value resets to default"),
  per-field `FieldInfo` (name, doc lines, deep-eq fn, format fn) used by
  `FileFormatter`, `Config::changed`, `ChangeIterator`.
* `config_enum!` / `config_packed!` define Zig-style enums / packed bool structs.
* Memory: all values are owned (`String`, `Vec`); there is no arena. `clone` is
  `#[derive(Clone)]`; Zig `equal` == `PartialEq` (custom where Zig's differs).
* Replay (theme / conditional state): `Config::load_replay` replaces the Zig
  `Replay.Iterator` (which aliased the config); behaviour is the same.
* `Config::default()` == Zig `Config.default` (default keybinds, command palette
  entries and the URL link are installed). `Config::raw_default()` == bare `.{}`.
* `Config::load()` == Zig `Config.load` (default files, CLI, recursive files,
  finalize). `finalize` skips the shell/passwd lookup under `cfg(test)` like Zig's
  `builtin.is_test`.
* Linux values are baked in for `builtin.os.tag` switches (e.g. `font-size = 12`,
  `copy-on-select = true`, `quit-after-last-window-closed = true`,
  `linux-cgroup = single-instance`, `alpha-blending = linear-corrected`).
  `gtk-opengl-debug` follows `cfg!(debug_assertions)` (Zig: `mode == .Debug`).
  `auto-update-channel` finalizes to `stable` (upstream release build value).
* macOS-only options (`macos-*`) are parsed and stored but otherwise ignored.

## Verification against the real binary

`ghostty +show-config --default` and `--default --docs` (Ghostty 1.3.1 installed
at /usr/bin/ghostty) were diffed against `FileFormatter` output of
`Config::default()`: identical except `gtk-opengl-debug` (debug vs release build).
A loaded config (themes light/dark, config-file includes incl. `?` optional
and missing, keybinds, palette, font-codepoint-map, errors/diagnostics) was also
diffed against `ghostty +show-config`; identical except `auto-update-channel`.

## Tests

Every Zig `test` in the ported files is ported (names snake_cased), except:
* args.zig: none skipped (36 Zig tests -> 40+ Rust tests incl. extras)
* Config.zig: C-API tests ("RepeatableCommand cval", "cval cleared") skipped
* key.zig `Value` (comptime type check) replaced by `value_key_types`
* Allocator-failure tests: none existed.
Extra tests added for config-file cycles / optional includes, docs formatting.

## Deviations / notes for integration

* `ArgError::Other(name)` renders as `unknown error error.<name>` like Zig's `{}` of an error.
* Error text for IO failures in `config-file` diagnostics uses Rust wording except
  `FileNotFound`/`AccessDenied`.
* `theme::open` takes the resources dir explicitly (`ghostty_os::resources_dir().app()`).
* URL regex (`url::REGEX`) needs Oniguruma-style lookbehind: consumers (renderer
  link matching) must use the `onig` crate (added as dev-dependency only here).
* `font_types.rs` holds local `Variation`, `Descriptor`, `CodepointMap`;
  `clipboard_codepoint_map::Entry` mirrors terminal/formatter's CodepointMap
  (TODO(integration): ghostty-font / ghostty-terminal should re-use these).
* `metric_modifier::MetricModifier` is the font `Metrics.Modifier` (font crate depends on config).
* `CursorStyle` is defined in `types_a.rs` (cannot depend on ghostty-terminal).
* macOS app-support config paths are not implemented (Linux only).
* `KeyRemapSet::PartialEq` is input's derived equality (Zig `equal` is order independent).
