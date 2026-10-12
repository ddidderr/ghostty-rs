# macOS port (M1): os, input, config, termio, core, crash glue

Scope of this note: everything of M1 except the two big sub-ports, which have their own notes:
* `macos-M1-font.md` – CoreText face/shaper/discovery, backend selection (`ghostty-font`).
* `macos-M1-renderer.md` – Metal backend, `GraphicsApi` abstraction (`ghostty-renderer`).

Nothing here has ever run on macOS. It is compile-checked (`cargo check|clippy --target
aarch64-apple-darwin|x86_64-apple-darwin`) and every piece of platform-independent logic was
extracted so its macOS variant is unit-tested on Linux (via explicit `darwin: bool` parameters, or
`ghostty_input::platform::with_darwin` in test builds). Linux behaviour is unchanged.

## Cross-checking on Linux (no macOS SDK)
The C build scripts of the dependency tree (onig_sys, gettext-sys, dear-imgui-sys, oslog) cannot
cross-compile without the SDK. For the *check only* (no repo change) use `zig cc`/`zig c++`
wrappers (`-target aarch64-macos`/`x86_64-macos`):
```
export CARGO_BUILD_RUSTC_WRAPPER=            # kache breaks cc-rs
export CC_aarch64_apple_darwin=<zcc-arm> CXX_aarch64_apple_darwin=<zcxx-arm> AR_aarch64_apple_darwin=<zig ar>
export GETTEXT_DIR=/nonexistent              # makes gettext-sys skip its from-source build
export CFLAGS_<target>=-I<dir with a stub os/log.h>   # oslog's wrapper.c needs <os/log.h>
export ZIG_GLOBAL_CACHE_DIR/ZIG_LOCAL_CACHE_DIR=<writable dir>
cargo check --target aarch64-apple-darwin -p ghostty-vt -p ghostty-unicode -p ghostty-datastruct \
  -p ghostty-os -p ghostty-input -p ghostty-config -p ghostty-terminal -p ghostty-font \
  -p ghostty-renderer -p ghostty-termio -p ghostty-core
```
(the wrappers drop `-arch`, `-mmacosx-version-min` and `--target=` that cc-rs adds). Real macOS builds
need none of this: onig_sys compiles with the system clang, `gettext-rs` builds libintl from source
(like Zig's `pkg/libintl`), `oslog`/imgui use the SDK. Root crate (`ghostty`, GTK) and
`ghostty-apprt-gtk` are Linux-only and are not part of the macOS check (the macOS app links the embedded
`libghostty`, L1's job).

## ghostty-os
| Zig | Rust |
|---|---|
| `os/macos.zig` | `macos.rs`: `QosClass`, `set_qos_class`, `pthread_setname_np`, `is_at_least_version`, `app_support_dir`, `cache_dir` (NSFileManager via objc2-foundation), pure `join_dir`/`QosClass` values tested everywhere |
| `os/mach.zig` | `mach.rs`: `VmTag` (+`make`), `tagged::TaggedPageAllocator` (mmap with VM tag in the fd arg; Rust has no stable allocator API so it is an alloc/unmap/realloc struct) |
| `os/cf_release_thread.zig` | `cf_release_thread.rs`: bounded(64) mailbox + stop channel instead of xev; injectable release fn (tests count releases on Linux); `CFRelease` via `CFRetained` |
| `os/args.zig` IteratorMacOS | `args::iterator()` on macOS reads `NSProcessInfo.arguments` |
| `os/mouse.zig` | `click_interval()` = `NSEvent.doubleClickInterval` ceil ms (`click_interval_ms` tested) |
| `os/desktop.zig` | macOS: ppid == 1 (+ `GHOSTTY_MAC_LAUNCH_SOURCE=app` for the lib artifact: `launched_from_desktop_lib`), `DesktopEnvironment::Macos`, iOS true |
| `os/homedir.zig` | macOS `homeDirectoryForCurrentUser` after the `HOME` check |
| `os/locale.zig` | Darwin `ensure_locale`: LANG from NSLocale (`<lang>_<country>.UTF-8`) + `LANGUAGE` from preferred languages (`preferred_language_value`), setlocale fallback chain (module `darwin`, unsafe allowed there) |
| `os/open.zig` | `open [-t] url`; iOS `Unsupported`; Windows branch kept |
| `os/resourcesdir.zig` | `Contents/Resources` bundle lookup first, `terminfo/78/xterm-ghostty` sentinel |
| `os/pipe.zig` | Darwin: `pipe` + `FD_CLOEXEC` on both ends (no `pipe2`) |
| `os/TempDir.zig` | non-Linux random via `/dev/urandom` (no `getrandom`) |
| `os/systemd.zig`, `passwd.zig` flatpak | Linux/Android only; `launched_by_systemd` false and `notify` no-ops elsewhere |
| `os/i18n.zig` canonicalize | still the pure-Rust approximation on all platforms (the real `_libintl_locale_name_canonicalize` is only linked in Zig's own libintl build) |
`xdg::Error` gained `AppleApiFailed`. Deps (apple only): objc2, objc2-foundation, objc2-app-kit, objc2-core-foundation, libc.

## ghostty-input
* `platform.rs`: `is_darwin()`/`is_macos()` consts with a test-only per-thread override (`with_darwin`).
* Darwin branches enabled: `Mods::translation` (option-as-alt with sides), `ctrl_or_super` (Mods, Key, free fn
  + `ctrl_or_super_for(mods, darwin)`), keycode table column 4 (mac) on Apple (`native_for_code` for tests),
  key_encode: kitty `alt_prevents_text`, modifyOtherKeys alt handling, `legacy_alt_prefix` option-as-alt,
  "super+text encodes nothing" on macOS. The 7 macOS-only Zig tests + `translation macos-option-as-alt` are ported
  and run on Linux through the override.
* `keymap_darwin.rs` (`KeymapDarwin.zig`): Carbon `TISCopyCurrentKeyboardLayoutInputSource`/`UCKeyTranslate`
  FFI (macOS only); `CarbonMods`, `carbon_modifier_state`, `utf16_to_utf8` tested on Linux; `keymap_noop.rs`;
  `ghostty_input::{Keymap, KeymapState, KeymapTranslation}` pick by platform. Crate lint is now `unsafe_code = "deny"`.

## ghostty-config
macOS defaults: `font-size` 13, `alpha-blending` native, `copy-on-select` (true on Linux/macOS, false elsewhere),
`quick-terminal-autohide` true, `quit-after-last-window-closed` Linux-only, `linux-cgroup` never off Linux.
`Keybinds::init_for(darwin)` is now a faithful port of the whole Zig `init` (platform lists + the
Mac-specific block: cmd+q/k/a, undo/redo, mac windowing/splits/search/inspector, natural text editing);
`init()` selects with the target. Linux output verified identical to the previous dump; macOS tested.
`file_load`: Application Support paths (`default/legacy/preferred_app_support_path`, `preferred_default_file_path`
macOS order), `edit::config_path_candidates`, `load_default_files` loads app-support + XDG and writes the
template to app support; `probable_cli_environment` macOS desktop branch; `load_cli_args` reads
`ghostty_os::args::iterator()` (NSProcessInfo) and the Linux/BSD-only `xdg-terminal-exec` + no-args fast path.

## ghostty-termio / crash / core / terminal
* `pty.rs`: Darwin `openpty()` (libc) + CLOEXEC on the master; `command.rs`: dup2 path clears CLOEXEC on the
  source fd, hand-rolled `execvpe` (PATH search like Zig's `execvpeZ`, candidates precomputed pre-fork,
  tested), `__error` for errno; `wakeup.rs`: non-blocking pipe instead of eventfd; `exec.rs`: no pidfd (poll
  `waitpid`), `login(1)` wrapping (`exec_command_with(command, darwin, passwd)`, `-q` for `.hushlogin`, tests ported +
  extra), `XDG_DATA_DIRS`/`MANPATH` bundle env (`darwin_bundle_env`), killpg EPERM ignored on Darwin, terminfo-missing
  warnings; `shell_integration.rs`: `/bin/bash` is not integrated on Darwin (`detect_shell_for`).
* `ghostty-crash`: `getentropy` instead of `getrandom` on Apple.
* `ghostty-terminal` search thread: `pthread_setname_np` + `QosClass::Utility` on Apple.
* `ghostty-core`: Darwin child-exit handling (any quick exit is abnormal, no exit code line), macOS
  `encode_key_opts` (`macos-option-as-alt` from config or `RtApp::keyboard_layout().detect_option_as_alt()`),
  rectangle select = option only, `global::Logging.macos` + `os_log` logger (oslog crate, unified logging, levels
  debug/info/err/fault like Zig; `GHOSTTY_LOG=macos` parsed).
* `ghostty-apprt` (tiny): `RtApp::keyboard_layout()` default method (`Unknown`); the embedded runtime (L1) overrides it
  with `ghostty_input::Keymap::source_id` → `KeyboardLayout::map_apple_id`.

## Not done / for L1 and later
* apprt embedded.zig macOS bits (NSView/surface plumbing, CVDisplayLink hosting, `keyboardLayout`, app menu actions): L1.
* Render/IO thread names on Darwin rely on `std::thread::Builder::name` (std calls `pthread_setname_np` for the
  new thread on Darwin, which is what the Zig explicit call works around). QoS for the renderer thread: see renderer note.
* The font crate keeps a private CF release thread (`shaper/cf_release.rs`); `ghostty_os::cf_release_thread` is available
  for the renderer to own one like Zig's `renderer/Thread.zig`.
* Skipped Zig tests: `macos.zig cacheDir paths` runs only on macOS (ported as a pure + `cfg(apple)` test),
  KeymapDarwin's empty `test {}`, `Command.zig` `/private/tmp` cwd expectation (Linux test untouched).
