# Z1 final pass: os / config / cli / root main (byte fidelity, i18n, build info)

Owner: Z1. Crates: `ghostty-os`, `ghostty-config`, `ghostty-cli`, new `ghostty-build-info`, root `src/main.rs`.

## Byte fidelity design

Zig paths/args/env are byte slices. The config system is `&str`/`String` based
throughout, so rewriting it to a byte type was not feasible without changing
hundreds of `ConfigField` impls. Instead:

* `ghostty_os::osbytes` is the single conversion module:
  * `bytes_to_config_string` / `config_str_to_bytes`, `os_to_config_string` /
    `config_str_to_os`, `path_to_config_string` / `config_str_to_path`,
    `os_bytes`, `os_from_bytes`.
  * The encoding is a true bijection on byte strings. Escape char = noncharacter
    `U+FDD0`; an invalid byte `b` becomes `U+FDD0` + char `U+00bb` (`0xFF` ->
    `U+FDD0 U+00FF`); a literal `U+FDD0` in valid input becomes `U+FDD0 U+FDD0`.
    Valid UTF-8 without `U+FDD0` is encoded unchanged (Nerd Font icons, incl.
    `U+F500..U+FD46`, are safe), and `decode(encode(x)) == x` for all bytes.
    Property tests (random bytes and strings, icon ranges, escape char) cover this.
* Every os function that returned `String` now has a byte-exact OS-typed twin
  (`*_os`), and the `String` function is a thin wrapper using the reversible
  encoding (a present non-UTF-8 value is never "absent" and never U+FFFD).
  Existing callers keep compiling.
* The config/cli layers decode back to OS strings at every filesystem / exec /
  stdout boundary they own.

## Audit rows fixed

| Row | Change |
|---|---|
| main_ghostty argv | `src/main.rs` collects `Vec<OsString>` (`os::args::collect_os`), dispatches with `ghostty_cli::ghostty::{detect_os, run_from_os_args}` (reversible encoding; no lossy). `collect_lossy` removed; `collect_bytes`, `collect_config_strings` added. |
| cli/args.zig | `ArgsIterator` via `args::collect_config_strings`; `LineIterator` no longer `from_utf8_lossy`; `parse_zig_string_literal_bytes` returns raw bytes (`\xFF` stays `0xFF`), `parse_zig_string_literal` returns the reversible string. |
| Config.zig | xdg-terminal-exec args and `cwd` keep bytes; `load_file`, config-file open, `load_reader` parent dir decode/encode paths. |
| os/env | `getenv_os`, `getenv_not_empty_os`; `getenv*` return `Some` for non-UTF-8 values. |
| os/homedir | `home_os`, `home_os_with_capacity`, `expand_home_os[_with_capacity]`; HOME, passwd and `cd && pwd` bytes kept (shell output trimmed as bytes). |
| os/path | `path::expand_os` ($PATH split and lookup as bytes). |
| os/passwd | `EntryOs`, `get_os` (native `shell`/`home`, flatpak `getent` output parsed as bytes). `Entry`/`get` unchanged shape (encoded strings). |
| os/resourcesdir | `GHOSTTY_RESOURCES_DIR` read via `var_os`; exe-relative probing uses `Path`s (`maybe_dir_os`); `ResourcesDir::{app_os, host_os}`. |
| os/file | `alloc_tmp_dir_os` (TMPDIR/TMP bytes, used by `TempDir`); `Rlimit` (unit struct), `fix_max_files` (-> `None`), `restore_max_files` (no-op) now exist on non-Unix targets like Zig's fallbacks. |
| os/cgroup | `current_os`; `scope_matches` compares bytes. |
| config/path | canonicalization, `resolve_lexical`, `theme.rs` locations use decoded paths and keep canonical (symlink-resolved) non-UTF-8 targets. |
| cli/validate_config | canonicalizes the decoded option path and passes the reversible string to `load_file`. |
| cli/list_themes | directory entries keep raw bytes (names/paths), plain listing writes raw bytes, `read_dir` on decoded dir; preview auto-theme file/clipboard copy decode bytes. |
| cli edit_config / new_window | exec editor / canonical working directory keep bytes. |
| os/i18n | `canonicalize_locale` is now an exact port of gnulib `gl_locale_name_canonicalize` (macOS flavour used by `_libintl_locale_name_canonicalize`): legacy table (131 names), langtag table (16), script table (4), `strlen==7` guard, dash->underscore; tables in `i18n_canon_table.rs` generated from gettext 1.0's `localename-unsafe.c`. `pa-Guru` -> `pa_IN`, `sr-Latn` -> `sr@latin`, `Brazilian Portuguese` -> `pt_BR`. |

Not touched (other owners): `ghostty-lib` `ghostty_init`/`config_capi`/`embedded` lossy
conversions, GTK file drop, `Surface::pwd()` -- Z2. Z2 may use the `osbytes`
helpers and `*_os` functions; `Entry`/`ResourcesDir` string fields carry the
reversible encoding, so use `config_str_to_os` before using them as paths.

## Build info (single source of truth) -- Z1 owns, nobody else edits

New crate `crates/ghostty-build-info` (no deps, `build.rs` derived; moved out of
`ghostty-cli/build.rs`, which now only keeps the action help table and
`GHOSTTY_RUSTC_VERSION`). Added to `[workspace.dependencies]`.

```rust
ghostty_build_info::VERSION_STRING   // &str  "1.3.1" or "1.3.1-<branch>+<hash>"; env GHOSTTY_VERSION_STRING overrides
ghostty_build_info::COMMIT           // Option<&str>
ghostty_build_info::RELEASE_CHANNEL  // "stable" | "tip"
ghostty_build_info::BUILD_MODE       // ".Debug" | ".ReleaseSafe" | ".ReleaseFast"
ghostty_build_info::{version(), parse(), SemanticVersion}
```

`ghostty_cli::build_info::Default` and `ghostty_cli::build_info::derived`
re-export/use it. For Z2: add `ghostty-build-info.workspace = true` to
termio/apprt-gtk/crash and use `VERSION_STRING` for XTVERSION
(`stream_handler.rs`), `TERM_PROGRAM_VERSION` (`exec*.rs`), the About dialog
(`window.rs`), and crash `release` (`sentry.rs`; `RELEASE_CHANNEL`/`COMMIT`
for tags).

## Tests added

`osbytes` (3), env non-UTF-8, homedir non-UTF-8, path non-UTF-8 dir lookup, passwd getent bytes,
resourcesdir non-UTF-8 env + exe dir, file tmpdir bytes, cgroup, i18n tables,
cli_args (`\xFF` raw byte, LineIterator), config path symlink to non-UTF-8 target,
validate_config symlink, list_themes non-UTF-8 entry, ghostty `os_args`, build-info (3).
