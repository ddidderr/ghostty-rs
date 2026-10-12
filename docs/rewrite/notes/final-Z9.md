# Z9: RV7 gaps and the string-field decode audit

Builds on `final-Z7.md` / `final-Z8.md` (`EncodedOs`, the reversible `U+FDD0`
escape in `ghostty_os::osbytes`). Contract used below:

* **encoded** = a config/CLI `String` in the reversible encoding. It must be
  decoded (`EncodedOs::{to_os,to_path,to_bytes,to_wide}`,
  `config_str_to_bytes`, or for UTF-8-only UI text `config_str_to_text`) before it
  reaches the OS, the pty, the clipboard, or C.
* **plain** = real text; it may only become an encoded value through
  `from_plain_text` / `valid_str_to_config_string` (escapes a literal `U+FDD0`).

New helpers: `osbytes::{InvalidWtf8, try_wtf8_to_wide, config_str_to_text}`,
`EncodedOs::{try_to_wide, try_to_wide_nul, try_to_os}`,
`ghostty_config::string::parse_bytes`, `ghostty_os::windows_nls::upcase_w`.

## 1. The nine RV7 gaps

| # | Fix | Test |
|---|---|---|
| 1 Windows env key casing | `windows_nls.rs` is a port of `std/os/windows/nls.zig` (the 2544-entry `l_intl.nls` table and `upcaseW`); `EnvMap` keys use it per UTF-16 unit instead of `char::to_uppercase` | `windows_nls::tests::*`, `env::windows_key_tests::keys_use_the_windows_table_not_unicode_mappings` (U+1F80 == U+1F88, Georgian distinct, no `ß` == `SS`) |
| 2 Windows locale init | `locale.rs` `posix` module now also builds on Windows: `setlocale` with Zig's literal `LC_ALL = 6` (the UCRT rejects it exactly like Zig, so the chain continues) and `_putenv_s` for the `LANG` updates (unsafe with SAFETY comments). `ensure_locale` is only a no-op on wasm | the mock-`Sys` chain tests run everywhere; Windows glue compiled by `check-windows` |
| 3 bash `--rcfile` | `GHOSTTY_BASH_RCFILE` = `EncodedOs::from_encoded(arg).to_os()` | `bash_rcfile_keeps_original_bytes` (byte FF, literal U+FDD0) |
| 4 configured `TERM` | termio `Config.term` is now `EncodedOs`; Unix exports `to_os()`, Windows `try_to_os()?` | `configured_term_is_exported_decoded` |
| 5 GTK inherited `LANG` | `saved_language` is `Option<OsString>` read with `var_os`; the `language` config override is decoded; `set_env` takes `AsRef<OsStr>`; env restore factored into `restore_language` | `inherited_lang_keeps_non_utf8_bytes` |
| 6 Windows Kitty paths | `os_path_from_bytes`: Unix bytes, Windows WTF-8 via `EncodedOs::try_to_os` (lone surrogates ok, malformed -> `InvalidData`, no empty-path fallback) | `payload_path_bytes_are_kept` |
| 7 malformed WTF-8 | `try_wtf8_to_wide` (strict; lossy `wtf8_to_wide` kept for non-launch use). `Command::start` (path, cwd, argv) uses `try_to_wide*` -> `StartError::InvalidWtf8`; the Windows exec env block/overrides/`GHOSTTY_RESOURCES_DIR`/`TERM`/`TERMINFO`/`PWD` use `try_to_os` -> `Error::InvalidWtf8` | `strict_wtf8_rejects_malformed_input`, `malformed_wtf8_env_overrides_fail` (cfg(windows)) |
| 8 embedded C API | `ghostty_surface_complete_clipboard_request` passes bytes (`CoreSurface::complete_clipboard_request` / `Surface::complete_clipboard_request` now take `&[u8]`; the confirm callback gets the caller's `text` pointer unchanged); `ghostty_surface_binding_action` encodes the bytes with `bytes_to_config_string` before `Action::parse` | `clipboard_completion_and_binding_action_keep_raw_bytes` (child receives `61 ff` / `62 ff`) |
| 9 text/csi/esc bindings | `Text` uses new `string::parse_bytes`; `Csi`/`Esc` write `config_str_to_bytes(data)` (128-byte limit applies to bytes). `string::parse` now escapes a literal `U+FDD0` that an escape sequence produces | `config_loaded_text_csi_esc_write_decoded_bytes`, `string::tests::parse_bytes_decodes_the_reversible_encoding` |

## 2. Systematic audit: config fields -> consumers -> decode status

"OK" = decoded at the boundary (or is the typed `EncodedOs`/bytes); "plain" =
guaranteed text with no OS meaning; "FIXED" = changed in Z9.

| Field / type | Consumers that reach OS / pty / clipboard / C | Status |
|---|---|---|
| `language` (String) | GTK `LANG` override | FIXED (decoded; was raw encoded) |
| `font-family*` (RepeatableString) | `Descriptor.family` -> fontconfig / CoreText | FIXED (`config_str_to_text` in `shared_grid_set::Key::new`; font names are UTF-8 for every backend) |
| `font-feature` | HarfBuzz/CoreText feature tags | plain (ASCII tags parsed from text) |
| `font-variation*`, `font-codepoint-map`, `adjust-*`, `font-style*` | descriptors/metrics | plain (tags, numbers, names) |
| `clipboard-codepoint-map` replacement | clipboard text from the formatter | FIXED (`decode_codepoint_map`; the formatter emits text, so non-UTF-8 replacement bytes can only become U+FFFD, a literal U+FDD0 is restored) |
| `title` | `SetTitle` action (all apprts, C `set_title`) | FIXED (`config_str_to_text` in core + GTK window) |
| `class` | GTK application id (`id_is_valid`) | plain (must be an ASCII-ish app id; encoded junk is rejected as invalid) |
| `window-title-font-family` | GTK CSS | FIXED (decoded) |
| `gtk-quick-terminal-namespace` | layer-shell namespace (`&str` API) | FIXED (decoded text) |
| `macos-custom-icon` | only via C `ghostty_config_get` | OK (`config_capi::cstring_value` decodes) |
| `term` | child env `TERM` (Unix, Windows) | FIXED (`EncodedOs` through `exec::Config`) |
| `enquiry-response` | pty write on ENQ | FIXED (was `String::into_bytes()` of the encoded text; `DerivedConfig.enquiry_response` is now decoded `Vec<u8>`; test `enquiry_response_is_decoded_to_bytes`) |
| `command`, `initial-command` (`Command`) | exec argv/shell string, Windows `CreateProcessW`, flatpak `ay`, macOS login | OK (decoded in `command.rs` `config_str_to_bytes`; Windows strict `try_to_wide`); window title from argv[0]/`xdg-terminal-exec` is FIXED (decoded text) |
| `env` (RepeatableStringMap) | child environment | OK (Unix `config_str_to_os`; Windows strict `try_to_os`) |
| `input` (RepeatableReadableIO) | pty write; file read | `Raw`: OK (decoded after `clone_parsed`). `Path`: FIXED, now typed `EncodedOs` (was opened undecoded); embedded `initial_input` escape works on bytes (`string_escape`) so `parse_bytes(string_escape(b)) == b`; test `initial_input_escape_round_trips_bytes`, `readable_io_path_decodes_to_the_original_bytes` |
| `link` regex | onig regex over terminal text | FIXED (decoded text; a pattern byte >0x7F that is not UTF-8 cannot match terminal text anyway) |
| `working-directory`, `background-image`, `custom-shader`, `gtk-custom-css`, `bell-audio-path`, `config-file`, `theme` | filesystem | OK (typed `EncodedOs` since Z7) |
| `command-palette-entry` title/description | GTK palette labels | FIXED (decoded text); its `action` follows the keybind row |
| `keybind` payloads: `text`, `csi`, `esc` | pty | FIXED (gap 9) |
| `keybind` `search:` | search needle bytes | FIXED (decoded; the GTK search entry encodes its plain UI text with `valid_str_to_config_string`) |
| `keybind` `set_surface_title`, `set_tab_title` | `SetTitle` action | FIXED (decoded text) |
| `keybind` `activate_key_table*` | table-name lookup (config vs config) | plain/consistent (both sides encoded) |
| `keybind` `cursor_key` | pty | not config-parsable (raw `String` built in code) |
| `selection-word-chars` | codepoints | plain |
| `Diagnostic.{key,message}` | `+validate-config` stdout, `+show-face` stderr | FIXED (print decoded bytes); GUI diagnostics are display text |
| CLI/C: `ghostty_init` argv, `ghostty_config_load_file`, surface options (`working_directory`, `command`, `env_vars`, `initial_input`), `ghostty_surface_text`, clipboard completion, binding action, `ghostty_config_get`/`open_path` | | OK (bytes in, encoded or bytes) |
| C key event `text`, preedit | `String` (lossy) | documented limit: UTF-8 text by the `ghostty.h` contract (IME/key text); would need a byte `KeyEvent.utf8` across `ghostty-input` |

## 3. OS data entering the system (byte preserving)

| Source | Entry point | Status |
|---|---|---|
| process environment | `std::env::var_os` / `vars_os` everywhere (`get_env_map`, `getenv`, `homedir`, `xdg`, `shell_from_env`, TMPDIR/TMP, GTK `LANG`, `PATH`); `std::env::var` remains only for ASCII-only developer switches (`GHOSTTY_LOG`, `RUST_LOG`, `COLORFGBG`, `GHOSTTY_DEBUG_*`) | OK |
| argv | `os::args` (`args_os` / C `ghostty_init` bytes) -> `collect_config_strings` | OK |
| config files, theme files | read as bytes -> encoded strings | OK |
| passwd (`getpwuid`), `uname` | bytes -> `EncodedOs` (hostname: text for comparison) | OK |
| OSC 7 pwd, OSC 8 URIs, kitty file paths | bytes kept (`set_pwd(&[u8])`, `os_path_from_bytes`) | OK |
| GTK file drops | `os_bytes(path)` into the shell-escaped paste | OK |
| clipboard (C API, OSC 52, GTK text) | bytes (`&[u8]` in `complete_clipboard_request`; GTK text is UTF-8) | OK |
| D-Bus / flatpak | cwd and argv `ay` bytes; environment is `a{ss}` (UTF-8 by D-Bus type) | known limit |
| `+new-window` over D-Bus | arguments are GVariant strings | known limit (UTF-8 by type, as Zig) |
| Windows APIs | WTF-16 via `EncodedOs::{from_wide,to_wide}`; process launch is strict | OK |

## 4. Typed where practical

`exec::Config.term` (`EncodedOs`), `DerivedConfig.enquiry_response` (`Vec<u8>`),
`ReadableIO::Path` (`EncodedOs`), `ghostty_apprt::CoreSurface::complete_clipboard_request`
(`&[u8]`). The remaining `String` config fields are either UI/plain text (above)
or go through the generic config machinery (macro, formatter, C value) where a
newtype would touch every field; each consumer is listed and decoded at its
boundary instead.

## 5. Gate

`cargo test --workspace`, `clippy --workspace --all-targets -D warnings`,
`cargo +nightly fmt --all`, `just check-macos` (both targets), `just check-windows`,
`just check-wasm`, `+show-config --default --docs` vs `/usr/bin/ghostty` (only the
`gtk-opengl-debug` line differs): see the final report. Windows and macOS code is
compile-checked, not run natively.
