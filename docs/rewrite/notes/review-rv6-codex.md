# Codex final verification (RV6, 2026-10-12)

Verified **`847a16f65c88`**, read-only. **12 of 13 RV5 rows are resolved; crash handling remains partial. All 10 previous sweep findings are resolved. Six additional gap groups remain.**

“Resolved” below refers to the reported defect, not complete parity of the entire module. Paths abbreviate `crates/ghostty-<crate>/src/` as `<crate>/`.

| RV5 row | Status | Current-code verification |
|---|---|---|
| `Surface.zig` | Resolved | Screen-file copy/paste preserve pathname bytes. `os::open` accepts `&OsStr`; regex links encode plain text, while OSC8 links retain raw bytes. [binding.rs:630](../../../crates/ghostty-core/src/surface/binding.rs:630), [open.rs:33](../../../crates/ghostty-os/src/open.rs:33) |
| `renderer/shadertoy.zig` | Resolved | Plain-array reconstruction preserves padded uniform layout and whole-array use. [shadertoy.rs:265](../../../crates/ghostty-renderer/src/shadertoy.rs:265) |
| `cli/args.zig` | Resolved | Windows conversions use WTF-8/WTF-16 and preserve lone surrogates; literal escape characters round-trip. [osbytes.rs:74](../../../crates/ghostty-os/src/osbytes.rs:74) |
| `config/Config.zig` | Resolved | Template creation decodes paths; substitution uses original bytes. [config.rs:3953](../../../crates/ghostty-config/src/config.rs:3953) |
| `apprt/gtk/class/surface.zig` | Resolved | Confirmation decodes only explicitly encoded byte pastes. [surface.rs:2680](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:2680) |
| `os/resourcesdir.zig` | Resolved | Windows resource exports decode through `to_os()`; `TERMINFO` uses an OS pathname. [exec_windows.rs:604](../../../crates/ghostty-termio/src/exec_windows.rs:604) |
| `config/path.zig` | Resolved | Custom-shader paths become decoded `PathBuf`s before loading. [generic.rs:248](../../../crates/ghostty-renderer/src/generic.rs:248) |
| `cli/list_themes.zig` | Resolved | Hyperlinks percent-encode decoded pathname bytes. [list_themes.rs:74](../../../crates/ghostty-cli/src/list_themes.rs:74) |
| `lib/allocator.zig` | Resolved | Color records use parser auxiliary storage backed by the caller’s allocator. X11 lookup no longer allocates. [color.rs:85](../../../crates/ghostty-vt/src/osc/parsers/color.rs:85) |
| `terminal/c/osc.zig` | Resolved | C parser supplies caller-allocated auxiliary storage; growth and allocation refusal are handled there. [osc.rs:82](../../../crates/ghostty-vt-capi/src/osc.rs:82) |
| `crash/sentry.zig` | **Partial** | Suspension is now balanced before returning; SIGABRT handoff exists. Remaining defects detailed below. |
| `tripwire.zig` | Resolved | `CopyMap` failure injection and cleanup/recovery are implemented. [screen.rs:2619](../../../crates/ghostty-terminal/src/screen.rs:2619) |
| `build/GhosttyLibVt.zig` | Resolved | Linker emits the versioned SONAME. Existing ELF reports `libghostty-vt.so.0`. [build.rs:23](../../../crates/ghostty-vt-capi/build.rs:23) |

The **remaining crash gap group** has two concrete defects:

- **SIGABRT receive failures are treated as success.** `request_abort_dump` ignores the timed `mach_msg` receive result and unconditionally returns `true`, suppressing the signal-path fallback even on timeout/error. [mach_handler.rs:464](../../../crates/ghostty-crash/src/mach_handler.rs:464), [signal.rs:257](../../../crates/ghostty-crash/src/signal.rs:257).
- **SIGABRT’s signal-number subcode is discarded from the minidump context.** `from_signal` supplies it, but `ExceptionInfo.subcode` retains subcodes only for `EXC_BAD_ACCESS`, excluding `EXC_SOFTWARE/EXC_SOFT_SIGNAL`. The event retains the signal; the minidump exception information does not. [mach_handler.rs:511](../../../crates/ghostty-crash/src/mach_handler.rs:511).

| Previous sweep finding | Status | Verified boundary |
|---|---|---|
| 1. GUI Open Config | Resolved | Decoded candidates and file creation: [config/edit.rs:24](../../../crates/ghostty-config/src/edit.rs:24) |
| 2. Background image | Resolved | Decode before loading: [renderer/generic.rs:863](../../../crates/ghostty-renderer/src/generic.rs:863) |
| 3. Crash directory | Resolved | XDG path decoded: [crash/dir.rs:24](../../../crates/ghostty-crash/src/dir.rs:24) |
| 4. SSH cache directory | Resolved | XDG path decoded: [cli/ssh_cache/disk_cache.rs:62](../../../crates/ghostty-cli/src/ssh_cache/disk_cache.rs:62) |
| 5. Executable directory / child PATH | Resolved | Native OS strings on Unix and Windows: [termio/exec.rs:1193](../../../crates/ghostty-termio/src/exec.rs:1193), [exec_windows.rs:866](../../../crates/ghostty-termio/src/exec_windows.rs:866) |
| 6. Fontconfig filenames | Resolved | Raw filename passed to face loader: [font/deferred_face.rs:75](../../../crates/ghostty-font/src/deferred_face.rs:75) |
| 7. Relative links / OSC8 | Resolved | Cwd decoded from bytes; OSC8 URI stays bytes: [core/surface/mouse.rs:406](../../../crates/ghostty-core/src/surface/mouse.rs:406), [mouse.rs:1617](../../../crates/ghostty-core/src/surface/mouse.rs:1617) |
| 8. Kitty temporary directory | Resolved | Raw `OsString` and byte comparisons: [terminal/kitty/graphics_image.rs:568](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:568) |
| 9. macOS `.hushlogin` | Resolved | Passwd home decoded before lookup: [termio/exec.rs:1445](../../../crates/ghostty-termio/src/exec.rs:1445) |
| 10. Benchmark C arguments | Resolved | Raw argument splitting and `PathBuf` data paths: [lib/benchmark.rs:65](../../../crates/ghostty-lib/src/benchmark.rs:65) |

The **new sweep found six additional concrete gap groups**:

1. **Default `SHELL` loses OS-string fidelity.** `std::env::var("SHELL")` rejects non-UTF-8 paths/lone surrogates. Valid text containing literal U+FDD0 is stored without encoding and subsequently decoded as encoded command text, potentially changing executable bytes. Zig retains the environment bytes. [config/config.rs:4621](../../../crates/ghostty-config/src/config.rs:4621).

2. **macOS executable search mishandles `PATH`.** A non-UTF-8 `PATH` is discarded in favor of the default search path. A literal U+FDD0 sequence in valid UTF-8 `PATH` can also be wrongly decoded when constructing candidates. [termio/command.rs:298](../../../crates/ghostty-termio/src/command.rs:298).

3. **Windows config environment overrides are not decoded.** `RepeatableStringMap` contains encoded strings, but Windows inserts keys and values directly into `EnvMap`. Literal U+FDD0 and encoded surrogate values therefore reach the child incorrectly. The Unix implementation decodes both. [termio/exec_windows.rs:690](../../../crates/ghostty-termio/src/exec_windows.rs:690).

4. **Windows environment-key comparison can overwrite a different variable.** Lossy conversion makes distinct lone-surrogate keys compare equal; `insert` then replaces the existing key’s value. ASCII-only uppercasing also misses Windows’ non-ASCII case equivalences. This affects storage, not merely display. [os/env.rs:31](../../../crates/ghostty-os/src/env.rs:31), [env.rs:44](../../../crates/ghostty-os/src/env.rs:44).

5. **`+show-config` does not round-trip escaped OS data.** Path formatting emits `as_encoded()`, and stdout writes that UTF-8 unchanged. Reloading encodes the escape characters again. For example, an original pathname byte `FF` becomes literal U+FDD0/U+00FF after export and reload. [config/path.rs:213](../../../crates/ghostty-config/src/path.rs:213), [cli/show_config.rs:64](../../../crates/ghostty-cli/src/show_config.rs:64), [config/cli_args.rs:811](../../../crates/ghostty-config/src/cli_args.rs:811).

6. **Non-Apple C API initialization still lacks locale setup.** `ghostty_init → global::init_lib` reaches the no-op `ensure_locale`; it never performs Zig’s `setlocale` and fallback behavior. The standalone GTK executable compensates separately, but the C API does not. [os/locale.rs:34](../../../crates/ghostty-os/src/locale.rs:34), [lib/main_c.rs:127](../../../crates/ghostty-lib/src/main_c.rs:127).

Other inspected stub markers were implemented elsewhere, unsupported cases shared with Zig, or upstream TODOs—including the unimplemented dynamic/special-color operations.

**Verdict: in-scope parity remains partial.**

| Count | Result |
|---|---:|
| RV5 rows resolved / partial | **12 / 1** |
| Previous sweep groups resolved / partial | **10 / 0** |
| New OS-data/encoding gap groups | **5** |
| New functional-stub gap groups | **1** |
| Total remaining verified gap groups, including crash | **7** |
| Selected existing-binary tests passed / failed | **47 / 0** |

No rebuild or native macOS/Windows/WASM execution was performed. Findings are source-verified; passing tests cover selected behavior, not exhaustive parity. No files changed; working tree remains clean.