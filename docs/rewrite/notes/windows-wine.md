# Windows code under Wine (agent WINE)

`just test-windows-wine` (= `scripts/windows-wine-test.sh [cargo test args]`)
runs the `x86_64-pc-windows-gnu` test binaries under Wine 11.19 (staging) with
the mingw-w64 toolchain. Dedicated `WINEPREFIX=target/wine-prefix`,
`WINEDEBUG=-all`, no display (console tests only). `RUSTC_WRAPPER=` (kache
breaks cc-rs), `RUSTFLAGS=--cfg wine` (lets tests mark Wine limitations as
ignored; `cfg(wine)` is declared in the workspace lints and ghostty-termio),
`WINEPATH` for the mingw runtime DLLs (libstdc++ from the bundled HarfBuzz).

## Native dependencies

Linking needs real libraries. The script builds them once into
`target/wine-deps/prefix` (like Ghostty, from source): expat 2.6.4 (tarball is
downloaded, or `EXPAT_TARBALL=`), FreeType (the sources vendored in
`freetype-sys`) and fontconfig 2.14.2 (zig package cache copy, `FONTCONFIG_SRC=`,
or a download) with cmake/meson cross builds. HarfBuzz/FreeType/oniguruma for
the `-sys` crates come from their bundled features as before.

## What ran (final run, exit 0)

| crate | passed | ignored |
|---|---|---|
| ghostty-vt | 486 | 1 (pre-existing) |
| ghostty-unicode | 20 | 0 |
| ghostty-datastruct | 88 | 0 |
| ghostty-input | 237 | 0 |
| ghostty-os | 97 | 0 |
| ghostty-config | 283 + 7 + 4 (integration) | 0 |
| ghostty-terminal | 1275 | 0 |
| ghostty-crash | 33 | 0 |
| ghostty-termio | 107 lib + 2 golden + 10 `exec_windows` | 1 (Wine) |
| ghostty-cli | 108 | 4 (Wine) |

Before the fixes: cli 7, config 9, os 2, terminal 1, termio 15 failures and a
link failure (below).

`ghostty-termio/tests/exec_windows.rs` is the Windows counterpart of
`exec_integration`: real `cmd.exe` children on a real ConPTY through
`Termio` + `Thread` (`WaitForMultipleObjects` loop) + the pipe read thread
against a real `Terminal`: `echo` output and exit status, `exit 7`, `Shell`
command via `%WINDIR%\System32\cmd.exe /C`, interactive typing and exit code,
120 KiB of input through the overlapped writer, resize messages
(terminal + `ResizePseudoConsole`), `stop` terminating a running child,
messages after exit, start failure shown in the terminal.

## Bugs found and fixed

* Link: `harfbuzz-sys` `bundled` is built without `HAVE_FREETYPE`, so
  `hb_ft_font_create_referenced` was missing on Windows. `ghostty-font/build.rs`
  now compiles `hb-ft.cc` against the bundled FreeType headers (Zig builds
  HarfBuzz with FreeType). `cc` added as a build-dependency.
* `std::fs::canonicalize` returns `\\?\C:\...` on Windows, Zig's `realpath`
  returns `C:\...`; the verbatim form rejects `/` and is not usable as a
  `cmd.exe` working directory. New `ghostty_os::path::realpath` strips it; used
  by config path expansion, `+new-window`, `+validate-config`, kitty graphics
  file loading, screen-file writes (faithfulness fix, not a Zig bug).
* DEVIANT_FIXES 38: absolute Windows theme paths were rejected (`C:` looks like
  a `light:`/`dark:` pair). DEVIANT_FIXES 39: `CreateProcessW` is called with a
  literal `lpApplicationName`, so a bare `cmd.exe` argv[0] failed with
  FileNotFound; PATH is searched first.
* Tests with Unix assumptions fixed (platform-aware expectations, no product
  change): shell-integration helpers (verbatim path, `;` PATH delimiter),
  `xev::prefer(Iocp)` (Iocp is a candidate on Windows), i18n `init` (no-op on
  Windows), kitty `/proc/` guard (POSIX only), config/theme/file-load
  directory-open errors (Windows gives `AccessDenied` for a directory, matching
  Zig 0.15 `OpenFile`; so `NotAFile` is `FileOpenFailed` there), path
  separators, `light:`/`dark:` pairs need quoted backslashes (as in Zig),
  `list_themes` file URIs (`file://C:%5C...`, as Zig's `toUri`).

## Wine limitations (ignored with `wine:` reasons)

* `ResizePseudoConsole` is a stub in Wine 11.19 (`fixme:console:
  ResizePseudoConsole (100,40) stub, faking success`): the child never sees the
  new size. `exec_windows::resize_is_applied_to_the_pseudo_console` is ignored;
  the terminal-side resize and the API call are still tested.
* `NtSetInformationFile(FileRenameInformationEx, REPLACE_IF_EXISTS|
  POSIX_SEMANTICS)` is unsupported (`fixme:file:NtSetInformationFile
  unsupported flags: 0x3`), so `std::fs::rename` over an existing file fails
  with `ERROR_ACCESS_DENIED`. The four `ssh_cache` disk-cache tests (atomic
  temp-file + rename) are ignored; real Windows supports it.
* Wine's `cmd.exe` has no `mode con`, and its `findstr` ignores `/n`; tests use
  a re-run of the test binary as console-size child and `%OS%` expansion.
* Wine maps `/` to `Z:\`; tests on POSIX-looking paths account for it.

## Not covered

fontconfig/font discovery and the renderer/core/GTK crates are not run (no
Windows apprt); ghostty-font links but its tests are not part of the run.
