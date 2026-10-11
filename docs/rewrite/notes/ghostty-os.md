# ghostty-os notes

Port of `ZIG/src/os/*` (Linux only) and `ZIG/src/file_type.zig`. `unsafe` is
forbidden; deps: rustix, nix (user), zbus, gettext-rs (`gettextrs`), thiserror,
log; dev: temp-env.

## File -> module
xdg, homedir, hostname, env, file, flatpak, i18n, i18n_locales, locale, open,
passwd, path, pipe, resourcesdir, shell, string_encoding, systemd,
TempDir (`temp_dir`), uri, desktop, kernel_info, cgroup, dbus, mouse, args,
main (re-exports in `lib.rs`), `file_type` (from `src/file_type.zig`).

## Skipped files
macos.zig, mach.zig, cf_release_thread.zig, windows.zig, wasm.zig, wasm/*
(macOS/Windows/WASM are out of scope). Windows/macOS branches inside ported
files are dropped. `main.zig` exports `OpenType` (stale, nonexistent in
open.zig) - not ported.

## API shape deviations (buffers/allocators -> owned values)
- Zig allocator/fixed-buffer APIs return `String`/`Option<String>`. Fixed
  buffer overflow semantics kept only where tested: `homedir::home_with_capacity`
  / `expand_home_with_capacity` (BufferTooSmall).
- `env::getenv` returns `Option<String>` (no `GetEnvResult`); `setenv`/`unsetenv`
  are NOT provided (unsafe in Rust 2024). Tests use `temp_env` (dev-dep).
- `ShellEscapeWriter<W: io::Write>`, `ShellCommandBuilder::to_owned_slice` -> `CString`.
- `string_encoding` take `impl io::Write`; error enum `{DecodeError, Io}`.
- `uri`: own `Uri`/`Component` types replicating `std.Uri.parse` (Zig std not
  available in Rust); `parse(text, ParseOptions)` as in Zig.
- `cgroup::current(pid)` reads at most 4096 bytes (caller buffer in Zig).
- `resourcesdir`: Zig `builtin.mode == .Debug` mapped to `cfg!(debug_assertions)`
  (note: workspace `release` profile has debug-assertions on; `production` off).
- `TempDir`: path-based (parent + name) rather than Dir handles; deletes on Drop;
  `deinit(self)` provided. Dir created mode 0o755.
- `file::fix_max_files` uses rustix rlimit; `None` == RLIM_INFINITY.
- `systemd::notify`: a leading `@` in NOTIFY_SOCKET is now treated as an abstract
  socket (Zig copied it literally, which can't work). Port otherwise 1:1.
- `i18n::canonicalize_locale`: gnulib's `_libintl_locale_name_canonicalize` is
  not available; pure-Rust approximation (zh fixes + `-` -> `_`). `tr()` ==
  `_()` (dgettext on bundle id domain). `static_locale` is a `const fn` that
  panics at compile time for unsupported locales. i18n always enabled.
- `open::open(kind, url)`: local `OpenUrlKind` enum
  (TODO(integration): apprt action kind). Snap handling keyed on `SNAP` env
  instead of build flag.

## Deferred / incomplete
- **locale::ensure_locale is a no-op.** `setlocale` and `setenv` are unsafe;
  no safe crate wraps them (`gettextrs::setlocale` is `unsafe fn`). gtk_init
  calls `setlocale(LC_ALL, "")` on Linux. TODO(integration): if needed, perform
  the libc call in an unsafe-allowed crate (steps documented in locale.rs).
- **passwd Flatpak branch**: Zig shells out to host `getent passwd` via
  FlatpakHostCommand + Pty. Needs ghostty-termio Pty -> TODO(integration);
  currently warns and uses sandbox entry.
- **FlatpakHostCommand**: implemented with `zbus` blocking API (HostCommand,
  HostCommandSignal, HostCommandExited signal, fd passing) in a helper thread;
  `waitXev` replaced by `wait_callback`. Compiles and the state machine/error
  paths are tested, but the D-Bus path is UNTESTED (no Flatpak portal here).
  Constructor `FlatpakHostCommand::new(argv, stdin, stdout, stderr)`; fds are
  `OwnedFd` and dup'd on spawn.
- `mouse::click_interval` is `None` on Linux (macOS only).

## Tests
Ported Linux-applicable Zig tests (all passing, 88 total incl. extras):
xdg(5), homedir(2: expandHomeUnix, home), hostname(4), env(4), path(3),
shell(1 builder + 7 escape), string_encoding(10 printf_q + singles_percent +
7 percent), uri(3), desktop(1), kernel_info(1), args(1), passwd(1), TempDir(1),
i18n canonicalize (darwin-only in Zig, run here against the approximation).
Skipped: none of the Linux tests. Extras: path_join, file_type, systemd notify
(real unix datagram socket), resourcesdir, pipe, uri/std parse, etc.
Zig tests that mutate env were rewritten with `temp_env::with_vars`; tests that
read PATH/HOME take the same lock via `testutil::with_env_lock`.
