# Windows port (agent WIN)

Windows-specific Zig code ported behind `cfg(windows)`; Linux code paths are
unchanged. Windows is compile-checked only (`cargo check --target
x86_64-pc-windows-gnu`), nothing can be run here.

## Verifying

```
RUSTC_WRAPPER= PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=<dir with stub fontconfig.pc> \
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
cargo check --target x86_64-pc-windows-gnu --tests \
  -p ghostty-vt -p ghostty-unicode -p ghostty-datastruct -p ghostty-input \
  -p ghostty-os -p ghostty-config -p ghostty-terminal -p ghostty-termio
```
* `RUSTC_WRAPPER=` is needed because the `kache` rustc wrapper from
  `~/.cargo/config.toml` is picked up by `cc-rs` as a compiler launcher and
  breaks the mingw C builds (onig_sys, freetype-sys, harfbuzz-sys).
* mingw (`x86_64-w64-mingw32-gcc/g++`, cmake) exists, so the C crates build:
  `onig_sys` natively, `freetype-sys` / `harfbuzz-sys` via their `bundled`
  feature (enabled only in the `cfg(windows)` dependency table of
  ghostty-font; Zig also builds them from source).
* `yeslogic-fontconfig-sys` still wants `pkg-config` for fontconfig (Zig's
  default backend there is `fontconfig_freetype`, built from source). For
  `cargo check` a stub `fontconfig.pc` + `PKG_CONFIG_ALLOW_CROSS=1` is enough
  (no linking happens). A real Windows link needs fontconfig from vcpkg/MSYS2.
* Windows clippy: `cargo clippy --target x86_64-pc-windows-gnu --no-deps
  -p ghostty-os -p ghostty-termio -p ghostty-crash -p ghostty-config
  -p ghostty-terminal -p ghostty-input --all-targets -- -D warnings` is clean.

## What was ported

| Zig | Rust |
|-----|------|
| `os/windows.zig` | `ghostty-os/src/windows.rs` (`windows-sys` re-export, `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, `to_wide_nul`, `computer_name`) |
| `os/pipe.zig` (CreatePipe) | `ghostty-os/src/pipe.rs` (returns `OwnedHandle`s) |
| `os/homedir.zig` | `homedir.rs`: `HOMEDRIVE`+`HOMEPATH` (`home_windows`, tested), `expand_home` is identity |
| `os/env.zig` | `;` delimiter; `EnvMap` is ASCII case-insensitive on Windows (Zig's EnvMap hash context); `filter_snap_paths` no-op |
| `os/file.zig` | `alloc_tmp_dir` = `%TMP%` only (None if unset); rlimit helpers are unix only |
| `os/path.zig` | `;` delimiter, `\` separator, every file is executable |
| `os/hostname.zig` | `GetComputerNameA` |
| `os/open.zig` | `rundll32 url.dll,FileProtocolHandler` (merged with M1's `open_command`) |
| `os/xdg.zig` | `LOCALAPPDATA` fallback for the XDG env var |
| `os/resourcesdir.zig` | sentinel `terminfo/ghostty.terminfo` |
| `os/desktop.zig` | `DesktopEnvironment::Windows`, `launched_from_desktop` uses `std::process::id` |
| `os/i18n.zig` | `init` is a no-op, `tr` returns the msgid (gettext is not built on Windows) |
| `os/temp_dir` | `BCryptGenRandom` for the name, no unix mode |
| `os/passwd.zig` (`@compileError` on Windows) | `ghostty_os::passwd` stub returning an empty `Entry`; `flatpak::is_flatpak()` is `false` |
| `pty.zig` `WindowsPty` | `ghostty-termio/src/pty_windows.rs` (named pipe for input, anonymous pipe for output, `CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole`, non-inheritable handles, drop order as `deinit`) |
| `Command.zig` Windows branch | `command_windows.rs` (`CreateProcessW` + `STARTUPINFOEXW` + proc-thread attribute list for the pseudo console, NUL stdio, cwd, env block, `wait` = `WaitForSingleObject`+`GetExitCodeProcess`, `terminate`) and the pure `windows_args.rs` (`windows_create_command_line`, `create_windows_env_block`, `windows_shell_args`) |
| `termio/Exec.zig` Windows branches | `exec_windows.rs` (read thread = blocking `ReadFile` on the output pipe, quit pipe polled with `PeekNamedPipe`, interrupted with `CancelIoEx`; process exit via the process handle; `cmd.exe` default shell and `%WINDIR%\System32\cmd.exe /C` for shell commands; `TerminateProcess` on stop) |
| `Thread.zig` loop (libxev IOCP) | `thread_windows.rs`: `WaitForMultipleObjects` over mailbox event, stop event, process handle + deadline timeout |
| `xev.Async` | `wakeup_windows.rs`: auto-reset event |
| `config/Config.zig` Windows branches | `load_cli_args` has no argc fast path, `probable_cli_environment` is false, default shell `cmd.exe` + `%HOMEDRIVE%%HOMEPATH%` working dir, `~/` not expanded in `path.rs` |
| `input/keycodes.zig` | native column 3 (win) on Windows |
| `terminal/kitty/graphics_image.zig` | shared-memory medium returns `UnsupportedMedium`, tmp dir = `%TMP%` |
| `crash/sentry.zig` | `init` is a no-op on Windows (as Zig), `signal_windows.rs` provides only `random_bytes` |

Module selection in termio is `#[cfg_attr(windows, path = "..._windows.rs")]`
in `lib.rs` (pty, command, exec, wakeup) so the Linux files are untouched;
`thread.rs` only gets `#[cfg(unix)]` on its poll loop and a `#[cfg(windows)]
mod windows_loop` child module.

## Dependency gating

* ghostty-os: `gettext-rs`, `nix`, `rustix`, `zbus` -> `cfg(not(any(wasm32, windows)))`;
  `windows-sys` -> `cfg(windows)`. Unix-only modules (`flatpak`, `passwd`,
  `systemd`) are `cfg(unix)`; `flatpak` / `passwd` have tiny Windows stand-ins.
* ghostty-termio: `libc`, `nix`, `rustix` (+ dev `zbus`, `rustix`) -> `cfg(unix)`;
  `windows-sys` -> `cfg(windows)`. Linux integration tests got `#![cfg(unix)]`.
* ghostty-terminal: `rustix` (shm) -> not wasm/windows.
* ghostty-crash: `libc` -> `cfg(unix)`.
* ghostty-font: bundled freetype/harfbuzz on Windows (see above).

## Tests

Platform independent (run natively): `windows_args` (10 tests: command-line
quoting rules incl. a CommandLineToArgvW round-trip reference splitter, env
block layout/UTF-16/invalid unicode, `%WINDIR%` shell args), `path_join_with`
Windows flavour, `home_windows`. Windows-only (compiled, not runnable here):
ConPTY size, pipe round-trip, `cmd.exe /c exit 7`, wakeup event, wait timeout
rounding, `exec_command` Windows variants, `hostname.exe` PATH lookup.
Unix-specific existing tests are `cfg(unix)`-gated.

## Deviations / gaps

* No termios timer on Windows (Zig doesn't run it either); `focus_gained` is a
  no-op (Zig would `@panic` there).
* Input writes go through a writer thread doing blocking overlapped
  `WriteFile`s (no libxev). Output uses plain blocking `ReadFile`.
* Windows `exec::Config` omits `rt_pre_exec_info`, `rt_post_fork_info`,
  `create_scope`, `restore_rlimit` (cgroup/rlimit are Linux-only);
  ghostty-core's surface setup will need `cfg` around them when it gets a
  Windows apprt.
* `Command::wait(block)` ignores `block` (blocks, like Zig). The thread
  handle from `CreateProcessW` is closed (Zig leaks it).
* Fontconfig discovery is kept on Windows (needs a real fontconfig to link).
* Not touched: `cli/ssh-cache/DiskCache` file locking, `cli/boo|list_colors|
  list_keybinds|edit_config` Windows branches (ghostty-cli), `benchmark/
  CodepointWidth`, `config/Config.zig` `probableCliEnvironment` env-based
  extras. ghostty-core/apprt-gtk are not Windows-checkable (GTK).
* Unrelated breakage seen while verifying (other agents): `ghostty-config`
  `config.rs` uses `ghostty_os::launched_from_desktop` under `cfg!(macos)`
  which fails on wasm32; `ghostty-os/tests/common/flatpak_mock.rs` fails
  `clippy::needless_pass_by_value`; `ghostty-font` coretext_math clippy.
