# Raw-byte config parsing refactor

Scope: Ghostty 1.3.1, the nine remaining groups in `review-rv8-codex.md`, and
the config/CLI-to-consumer encoding sweep. No version change or commit.

## Design

`ArgIter` yields original byte vectors. `LineIterator` trims and splits bytes;
it never escape-encodes values. Process arguments use `collect_bytes`, including
arguments after `-e`. Replay and conditional replay retain byte vectors.

`CliTarget::parse_into_field`, generated `parse_field_bytes`, and
`ConfigField::parse_bytes` take `Option<&[u8]>`. Each field chooses its contract:

- Text/numeric/enum grammars validate UTF-8 at the field boundary and return
  `InvalidValue` for invalid input. Selection delimiters decode codepoints from
  original bytes and validate each unescaped UTF-8 sequence. Clipboard string
  replacements are validated UTF-8, stored as ordinary Rust strings.
- Byte strings, paths, environment entries, commands, font family/style names,
  binding payloads, key-table names, themes, readable input, and command-palette
  labels retain their bytes in `EncodedOs`. Font variation identifiers retain
  their four original bytes.
- Tagged unions, comma splitting, and automatic structs split ASCII syntax
  without interpreting payload bytes. Quoted automatic-struct values use the
  raw Zig string-literal decoder. `config/string.zig` parsing independently
  copies unescaped bytes and UTF-8-encodes escaped codepoints, as upstream does.
- `parse_cli(&str)` and `parse_field(..., &str)` remain convenience APIs for
  **plain UTF-8**. They forward `str::as_bytes`; they never decode OS escapes.

`EncodedOs` remains a distinct storage/OS-plumbing type. `from_bytes` and
`from_plain_text` are input constructors; `from_encoded` is reserved for
already encoded internal data. Runtime font, title, key-table, and command
consumers now carry the typed values to their final boundary. GTK display
conversion is explicit and separate from C callbacks and byte output.

The existing `fmt::Write` config formatter uses an internal encoded string.
Plain text is encoded exactly once when inserted; `+show-config` decodes once
to bytes at the writer. This preserves normal output and arbitrary byte-valued
fields. Action strings retain Zig's `std.zig.stringEscape` formatting, now
applied to the original bytes. Upstream escaping/reset/append semantics are
preserved; this does not promise that every formatted action or repeatable list
is an identity operation when loaded over existing defaults.

## RV8 finding → fix → regression coverage

| # | Fix | Tests |
|---|---|---|
| 1 | `EncodedOs::to_path` is fallible. Windows paths validate WTF-8 and report `BadPathName`; config files, themes, renderer assets and other path consumers propagate/handle the error. | `osbytes::tests::windows_paths_fail_with_bad_path_name_instead_of_replacement` |
| 2 | Windows application, cwd, argv, configured environment, and inherited environment use strict UTF-8 conversion before UTF-16/process creation. Surrogate encodings fail. | `windows_command_inputs_require_strict_utf8`; `windows_args::tests::inherited_environment_rejects_non_utf8`; Windows `env_overrides_keep_literal_fdd0_and_reject_surrogates` |
| 3 | WTF-8 validation accepts separately encoded surrogate pairs without requiring canonical re-encoding equality. The Kitty path conversion uses this corrected decoder. | `wtf8_accepts_separately_encoded_surrogate_pairs_like_zig` |
| 4 | Binding triggers parse original UTF-8, so literal U+FDD0 is one codepoint. Only action payloads and table names become typed byte storage. | `raw_bytes::literal_fdd0_is_a_single_unicode_trigger`, through both file and CLI readers |
| 5 | Selection delimiter codepoint iteration rejects invalid original UTF-8 with `InvalidValue`. | `raw_bytes::selection_word_chars_reject_invalid_utf8` |
| 6 | Clipboard replacements validate UTF-8 before storage; the consumer no longer decodes an escape representation or substitutes U+FFFD. | `raw_bytes::clipboard_replacements_require_utf8_and_keep_literal_fdd0`; clipboard plain-text consumer test |
| 7 | Font styles and codepoint-map families remain typed bytes through descriptors. Fontconfig receives original bytes; CoreText validates UTF-8 when constructing its CF strings. | `raw_bytes::font_style_and_codepoint_family_keep_original_bytes`; `discovery::tests::configured_font_names_reach_fontconfig_as_original_bytes` |
| 8 | Configured, dynamic and binding-action titles remain bytes through runtime actions and C string construction. | `c_title_actions_and_key_table_names_forward_original_bytes`; embedded integration `configured_and_binding_titles_reach_c_as_original_bytes` |
| 9 | New-window preserves valid original argument strings; incoming D-Bus text is plain UTF-8. Invalid UTF-8/interior-NUL arguments abort as Zig/GIO does, without untrusted-GVariant empty-string substitution. Flatpak environment construction reproduces GLib's whole-value `[Invalid UTF-8]` substitution. | `arguments_are_plain_dbus_strings_not_escape_encoded`; subprocess `invalid_dbus_strings_fail_without_substitution`; `glib_env_replaces_the_whole_invalid_value` |

## Additional sweep

Reviewed uses of `from_encoded`, `as_encoded`, `config_str_to_bytes`,
`config_str_to_text`, lossy UTF-8 conversion, config field implementations,
formatters, and downstream consumers across the workspace.

Additional fixes from this sweep:

- Raw command parsing, environment maps, initial input, and auto-struct quoted
  payloads no longer see encoding artifacts.
- Key-table names, command-palette titles/descriptions and C name lengths
  retain original bytes; command-palette sorting compares original bytes.
- Font feature tags pass original bytes to their backends. CoreText's ASCII
  conversion is checked. Font variation tags accept any four bytes, and their
  formatter no longer uses lossy UTF-8.
- Selection, clipboard, trigger and composed keybind formatting distinguish
  plain text from internal encoded output, preventing double escaping.
- GTK diagnostics decode the formatted bytes before insertion and mirror
  GTK's rejection of invalid UTF-8. The working-directory GObject property
  renders plain text from typed byte storage, including in tab tooltips and
  window subtitles. Theme preview display/search and theme-name sorting use
  decoded text/original bytes. Logs render decoded text; Windows edit-config
  output writes the original path bytes.
- Link regex strings are plain text and are no longer mistakenly decoded as
  OS escapes. Clipboard replacement consumers likewise use validated text.
- C binding-action input and embedded initial input enter parsing as raw
  bytes. Text/CSI/ESC output retains original payload bytes and byte lengths.
- Theme lookup carries typed paths without encode/decode detours or an empty
  path fallback. Windows filesystem and process conversion contracts are
  deliberately separate.
- Windows home expansion returns the original byte value, as Zig does. PATH
  lookup and shell-integration resource paths use checked filesystem conversion.
  Bash's extracted rcfile argument validates UTF-8 before entering a Windows
  environment string. Windows-specific regressions cover these three paths.
- Byte-based trimming matches each Zig parser: space/tab for config syntax;
  all Zig ASCII whitespace, including vertical tab, for working directories,
  environment entries, and new-window options.

Remaining encoding uses are storage/output, diagnostics, existing command
execution plumbing, and explicitly tagged GTK clipboard transport.
They do not feed escape-encoded values to config field parsers. Lossy conversion
is confined to display APIs or legacy OS helpers; Windows path and process
operations use checked conversions.

A direct GLib/GIO C probe compared trusted and untrusted serialized strings:
untrusted invalid UTF-8 became an empty D-Bus string; Zig's trusted construction
aborted in `g_dbus_message_to_blob`. The Rust IPC path explicitly aborts for
these malformed strings before creating a variant, preserving that failure
without violating the Rust GLib binding's trusted-data safety contract.

Additional regressions cover raw byte fields/auto structs, whitespace,
four-byte variation tags, `+show-config` output/reload for both byte and text
fields, and action escaping from original bytes. Existing byte-path, shell,
clipboard, key-text and preedit tests remain in the workspace suite.

## Validation

All required gates passed on the completed refactor:

| Gate | Result |
|---|---|
| `timeout 2400 cargo test --workspace --no-fail-fast` | Exit 0; 3,428 passed, 0 failed, 7 ignored across reported test summaries |
| `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| `cargo +nightly fmt --all` | Exit 0 |
| `just check-macos` | Exit 0 |
| `just check-macos x86_64-apple-darwin` | Exit 0 |
| `just check-windows` | Exit 0 |
| `just check-wasm` | Exit 0, including Node execution |
| `cargo build --bin ghostty` | Exit 0 |
| `git diff --check` | Exit 0 |

The exact required comparison,
`diff <(target/debug/ghostty +show-config --default --docs) <(ghostty +show-config --default --docs)`,
reports only line 3837: `gtk-opengl-debug = true` in Rust versus `false` in the
installed Ghostty binary. Its exit code is 1 for this single expected difference.

Eight differential `+show-config` probes against `/usr/bin/ghostty` match byte
for byte, each exiting 0: ordinary config; raw byte strings/font names;
literal U+FDD0 delimiters/replacements/triggers; rejected invalid UTF-8;
raw title/text/search actions; quoted automatic structs; environment ASCII
whitespace; and four-byte font variation tags. These probes explicitly set
`auto-update-channel = stable`: without this setting the pre-existing loaded
config default differs (`stable` in the port versus `tip` in the installed
reference). The required `--default --docs` comparison is unnormalized.

The macOS and Windows checks compile/lint platform-specific paths; they are
not native hardware execution. The WASM check also executes its Node host
smoke test. Windows conversion regressions execute as platform-independent
unit tests on Linux, with additional Windows-only tests cross-compiled. No new Zig tests were skipped
for these fixes; this refactor adapts existing ports and adds regressions.
