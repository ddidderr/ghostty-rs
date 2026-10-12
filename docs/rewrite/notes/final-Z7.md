# Z7 final pass: typed OS strings + RV5 findings

Owner: Z7. Fixes the RV5 rows (`review-rv5-codex.md`) structurally: escaped OS
strings can no longer reach an OS boundary undecoded, and plain text can no
longer be decoded by accident.

## Structural change: `ghostty_os::osbytes::EncodedOs`

`EncodedOs` is a newtype over the reversible `U+FDD0` escape encoding (Z1/Z5).
It is deliberately **not** a `str`, not `Display`, and not `AsRef<Path/OsStr>`:

* Producing: `from_os`, `from_path`, `from_bytes`, `from_wide` (Windows WTF-16),
  `from_plain_text` (escapes a literal `U+FDD0`, i.e. text is never decoded),
  `from_encoded` (adopt a CLI/config string that is *already* encoded; the only
  "trust me" constructor).
* Consuming (explicit decode at the boundary): `to_os`, `to_path`, `to_bytes`,
  `to_wide`, `to_wide_nul`. `as_encoded` is for config files/`+show-config`
  output (unchanged format).

Typed APIs (compiler-enforced):

| Area | Before | After |
|---|---|---|
| `os::env::getenv[_not_empty]`, `homedir::home`, `expand_home[_with_capacity]`, `xdg::{config,cache,state}` (+`Options.home`), `path::expand`, `passwd::Entry` fields, `ResourcesDir` fields/`app()`/`host()` | `String` | `EncodedOs` |
| `os::open::open(kind, url)` | `&str` (always decoded) | `&OsStr`: callers decide; `apprt::action::OpenUrl.url` and `Pwd.pwd` are `EncodedOs` |
| `os::i18n::init` | `&str` | `&Path` |
| `os::flatpak::FlatpakHostCommand.cwd`, termio `Command.cwd`, termio/exec `Config.{working_directory,resources_dir}`, `shell_integration::setup*` resource dir, `darwin_bundle_env` | `String` | `EncodedOs` |
| `config::Path` (Optional/Required), `WorkingDirectory::Path`, `file_load::*`, `edit::open_path`, `theme::{open,Location,LocationIterator,OpenedTheme}`, `Config::{load_file,load_optional_file}`, `ReplayStep::Expand` | `String` | `EncodedOs` |
| `Surface::pwd()`, `apprt::surface::new_config(prev_pwd)`, GTK surface/window/app `working_directory` overrides, cli `ThemeListElement.path`, list-themes `Effect::Copy` | `String` | `EncodedOs` |
| renderer `PathItem` (custom shaders) | `String` | `PathBuf` (decoded once in `DerivedConfig`) |
| `RuntimeSurface::set_clipboard` contents | `(String, String)` | `(String, Vec<u8>)` (Zig `[:0]const u8`) |
| crash `Report.name`, bench `Args` values/data path | `String` | `OsString`/`PathBuf` |

Config file output/format is byte-identical (`as_encoded`); no config key
changed its syntax.

Boundaries that still hold encoded `String`s on purpose (each decodes at its
own boundary): `Command`/argv items (`config::Command`, termio `Command.args`,
`exec_command`) and `RepeatableStringMap` env pairs (decoded in
`Subprocess::new`/`prepare`); the GObject `pwd` property of the GTK surface
(a GLib string, converted in `get_pwd`/`set_pwd`); `CValue::{String,Path}` (now
decoded by `config_capi::cstring_value` when handed to C, previously the
escapes leaked to the embedder); `macos_custom_icon` (a string field, goes
through that C decode).

## RV5 findings -> fixes

| Finding | Fix | Test |
|---|---|---|
| Surface copy (`binding.rs:635`) | `write_screen_temp_file` returns a `PathBuf`; copy/paste send `os_bytes(path)` (clipboard contents are now bytes), open sends `EncodedOs::from_path` | `screen_temp_file_keeps_non_utf8_path_bytes` (TMPDIR non-UTF-8) |
| open.rs regression (`open.rs:33`, `mouse.rs:1663`) | `open` takes `&OsStr`; regex links use `open_link_target` -> `EncodedOs::from_plain_text` (never decoded); resolved relative paths use `from_path`; OSC 8 URIs `from_bytes(raw)` | `open_link_plain_text_is_never_decoded`, `open_link_resolves_against_non_utf8_pwd`, `osc8_uri_bytes_are_carried_unchanged`, `os::open::encoded_paths_decode_but_plain_urls_do_not` |
| Windows WTF-8 / lone surrogates in osbytes | `os_bytes`/`os_from_bytes` on Windows use `encode_wide`/`from_wide` through portable `wide_to_wtf8`/`wtf8_to_wide` (lone surrogates are 3-byte WTF-8, escaped by the encoding); `EncodedOs::{from_wide,to_wide}` | `wtf8_round_trips_unpaired_surrogates`, `wtf8_invalid_bytes_become_replacement`, `encoded_os_wide_round_trips_lone_surrogates` (platform independent) |
| Windows termio `GHOSTTY_RESOURCES_DIR`/`TERMINFO` (`exec_windows.rs:606,618`) | decoded with `to_os()`; `terminfo_dir` builds the OS string (no lossy) | `terminfo_dir_is_sibling_of_resources_and_lossless` (cfg(windows), compiled by `check-windows`) |
| Windows exe dir / PATH (`exec_windows.rs:855`), `%WINDIR%`, `%HOMEDRIVE%%HOMEPATH%`, cwd, argv | exe dir kept as `OsString`; `home_windows` concatenates `OsStr`s; `CreateProcessW` gets `to_wide_nul` for path/cwd/args via new `windows_create_command_line_wide` | `wide_command_line_matches_str_version_and_keeps_lone_surrogates`, `home_windows_joins_drive_and_path` |
| OSC color request lists allocate globally (`color.rs:232`, `kitty_color.rs:57`, `capi/osc.rs:144`) | request lists are stored as fixed 8-byte records in a second parser storage (`Parser.aux`, same allocator as the payload); `Command::{ColorOperation,KittyColorProtocol}` borrow them (`color::List<'a>`, `kitty::color::List<'a>`, `into_owned` for the stream). C parser: 4 inline records (Zig `SegmentedList(Request, 4)`), longer lists grow through the caller's allocator, failure -> invalid command. X11 name lookup no longer allocates (build.rs generates a sorted lowercase table; previously a lowercased `String` + lazy global map) | Rust `end_makes_no_global_allocations` extended with OSC 4/5/10/11/104/112/21; `long_color_lists_grow_via_the_caller_allocator`; C `alloc_test.c` section 6 (frozen allocator, color commands) and new section 7 (growth + refusal) |
| macOS Mach handler hang (`mach_handler.rs:289,305`) | `SuspendGuard` (platform independent, `suspend.rs`) resumes every suspended thread, the faulting one included, before ports are restored and `KERN_FAILURE` is replied (Breakpad behaviour) | `suspend::tests::*` (mock threads, incl. panic path) |
| macOS SIGABRT minidump (`signal.rs:215`) | Breakpad design: SIGABRT handler sends an abort request (`exception::ABORT_MSG_ID`) to the exception-port thread and waits on a reply port (bounded); the thread dumps with `EXC_SOFTWARE/EXC_SOFT_SIGNAL(SIGABRT)` and replies. Falls back to the event-only report if the port is unavailable | `exception::tests::{abort_request_round_trips,sigabrt_is_described_as_a_soft_signal}`; compile via `just check-macos` (both targets). Not run on real macOS. |
| 1 Open config (`edit.rs`) | `open_path` returns `EncodedOs`; candidates and create-dir/file use decoded paths | `open_path_uses_decoded_non_utf8_xdg_dir` |
| 2 Background image (`generic.rs:1099`) | `load_background_image(&Path)` from `path().to_path()` | `background_image_loads_from_non_utf8_config_path` |
| 3 Crash dir (`dir.rs:22`) | `default_dir` uses `to_path()`; sentry `State.dir` stores `os_bytes` and decodes with `os_from_bytes` (was lossy on non-Unix) | `default_dir_decodes_non_utf8_xdg_state`, `non_utf8_report_name_is_kept` |
| 4 SSH cache (`disk_cache.rs:62`) | `to_path()` | `default_path_decodes_non_utf8_xdg_state` |
| 5 Exe dir / PATH (`exec.rs:1191`) | `OsString` end to end (`add_ghostty_path_for`) | `ghostty_path_keeps_non_utf8_exe_dir_bytes` |
| 6 Fontconfig filenames (`fontconfig.rs:226`) | `Pattern::get_os_string`/`add_string_bytes`; `Face::init_file(impl AsRef<Path>)` | `file_name_keeps_non_utf8_bytes` |
| 7 File links (`mouse.rs:405,1614`) | see open.rs row; `osc8_uri` returns raw bytes; `copy_url_to_clipboard` writes bytes | as above |
| 8 Kitty TMPDIR (`graphics_image.rs:591`) | `OsString` temp dir compared by bytes (`is_path_in_temp_dir_with`) | `temp_dir_check_keeps_non_utf8_tmpdir_bytes` |
| 9 `.hushlogin` (`exec.rs:1442`) | `passwd.home.to_path()` | `exec_command_darwin_hushlogin_with_non_utf8_home` |
| 10 Benchmarks (`benchmark.rs:65`, `args.rs:82`) | `Args` values are `OsString`, `--data` is a `PathBuf`; C args are split on raw bytes | `data_path_keeps_non_utf8_bytes`, `split_args_keeps_non_utf8_bytes` |

Other bugs found by the type change (not in RV5): GTK `gtk-custom-css` and
bell-audio paths used the escaped string as a path; `HISTFILE` for bash
integration was built from the escaped home; OSC 52 clipboard writes were
lossily converted to UTF-8 (now bytes); `ghostty_config_get` leaked escapes to
C; `ghostty_config_open_path` returned escaped bytes.

## Lossy-call audit (non-test code)

Grep: `to_string_lossy|from_utf8_lossy|to_str().unwrap|Path::new(|PathBuf::from(`
over `crates/*/src` (non-test modules). Result: all OS-derived values are typed;
what remains is justified below.

* **Display/log only** (never reaches a syscall): `os/open.rs` stderr of the
  opener, `termio/shell_integration.rs` and `termio/exec.rs` log messages,
  `terminal/kitty/graphics_image.rs` shm path log, `lib/embedded.rs` `wd_disp`,
  `apprt-gtk/debug_input.rs`, `cli/crash_report.rs` (listing), inspector widgets
  (`pwd`, hyperlink ids/uris), `core/surface/mouse.rs:1816` (link-preview
  status text), `gtk class/command_palette.rs`, `class/surface.rs` clipboard
  confirmation text (the clipboard itself gets the bytes).
* **Text, not OS data**: VT/OSC/tmux parsers (`ghostty-vt`, `terminal`), CLI
  config parsing (`config/command.rs` splitter works on already-valid encoded
  text), fonts' name/tag strings (`freetype.rs`, `feature.rs`,
  `fontconfig.rs::get_string` documents UTF-8-only properties), shadertoy
  source text, `terminfo/source.rs`, `build-data`, `datastruct`, `os/uri.rs`
  `to_raw` (documented: host names; paths use `to_raw_bytes`), `os/kernel_info`,
  `os/locale`, `os/hostname`, `os/desktop` (env values parsed as numbers/lists),
  `lib/embedded.rs` key text/preedit/IME strings (C API text), `lib/inspector.rs`,
  `lib/benchmark.rs:60` (action *name*, compared with the ASCII action table),
  `ssh_cache/disk_cache.rs:280` (cache file of host names), `bench/args.rs`
  messages and the action word.
* **Windows-only fallbacks**: `os/osbytes.rs` non-unix/non-windows branch
  (wasm: UTF-8 only), `os/env.rs:31` case-insensitive key compare on Windows
  (comparison only; lone-surrogate keys may collide, never rewritten),
  `os/windows.rs:75` computer name (ANSI API).
* **D-Bus**: `os/flatpak.rs:152` env `a{ss}` values are UTF-8 by D-Bus type
  (unchanged from Z5). cwd/argv use `ay` bytes.
* **Plain text paths**: `core/surface/mouse.rs:402,407` (`Path::new(text)` of
  the selected link text; the pwd side is `os_from_bytes`),
  `core/surface/mod.rs` screenshot helpers (developer aid, `var_os`),
  `os/path.rs`, `os/macos.rs` (`PathBuf::from` of an already-OS value),
  `os/resourcesdir.rs:152` (compile-time constant), `kitty/graphics_image.rs`
  (`Path::new(OsStr::from_bytes(path))`: bytes from the protocol).
* Sprite `Path::new()` hits are the vector `Path` type, not `std::path`.

Remaining known gaps: argv items of `config::Command`/termio `Command` and
`RepeatableStringMap` stay encoded `String`s (decoded at exec); Windows
`CreateProcessW` argv/path use WTF-16 via `EncodedOs::from_encoded` of those
items; macOS and Windows behaviour is compile-checked (`check-macos` for both
Apple targets, `check-windows`) plus platform independent unit tests, not run
natively.
