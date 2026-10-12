# Codex final verification (RV7, 2026-10-12)

**Parity remains partial: 9 concrete gap groups remain.** Verified commit `350330838598e01a3090932afc7101df64511d7b`, read-only. No files changed.

“Resolved” below means the reported defect is fixed, not that the entire module has complete parity. Abbreviated Rust paths are relative to `crates/ghostty-<crate>/src/`.

| RV5 row carried into RV6 | Status | Current-code verification |
|---|---|---|
| `Surface.zig` | Resolved | Screen-file copy/paste retain path bytes; open targets decode explicitly. `core/surface/binding.rs:630`, `os/open.rs:33` |
| `renderer/shadertoy.zig` | Resolved | Array reconstruction preserves padded uniforms and whole-array reads. `renderer/shadertoy.rs:265` |
| `cli/args.zig` | Resolved | OS argument encoding preserves lone surrogates and literal U+FDD0. `os/osbytes.rs:74`, `os/args.rs:66` |
| `config/Config.zig` | Resolved | Template substitution uses decoded bytes; filesystem operations use decoded paths. `config/config.rs:3961` |
| `apprt/gtk/class/surface.zig` | Resolved | Confirmation decodes only explicitly encoded byte pastes. `apprt-gtk/class/surface.rs:2684` |
| `os/resourcesdir.zig` | Resolved | Windows resource exports and `TERMINFO` use native OS paths. `termio/exec_windows.rs:620` |
| `config/path.zig` | Resolved | Custom-shader paths decode before loading. `renderer/generic.rs:248` |
| `cli/list_themes.zig` | Resolved | File hyperlinks percent-encode decoded pathname bytes. `cli/list_themes.rs:74` |
| `lib/allocator.zig` | Resolved | OSC color requests use parser auxiliary storage; X11 lookup does not allocate. `vt/osc/parsers/color.rs:87`, `vt/color.rs:640` |
| `terminal/c/osc.zig` | Resolved | Auxiliary storage and growth use the caller allocator; refusal invalidates the command. `vt-capi/osc.rs:82` |
| `crash/sentry.zig` | Resolved | Suspension guard resumes threads before reply; receive failure returns false; SIGABRT subcode reaches the minidump. `crash/mach_handler.rs:352`, `:476`, `:513` |
| `tripwire.zig` | Resolved | `CopyMap` injection and owned-allocation cleanup are present. `terminal/screen.rs:2619` |
| `build/GhosttyLibVt.zig` | Resolved | Build script supplies versioned SONAME/install name. `vt-capi/build.rs:23` |

| Previous sweep finding | Status | Current boundary |
|---|---|---|
| GUI Open Config | Resolved | `config/edit.rs:24` |
| Background image | Resolved | `renderer/generic.rs:863` |
| Crash directory | Resolved | `crash/dir.rs:24` |
| SSH cache directory | Resolved | `cli/ssh_cache/disk_cache.rs:62` |
| Executable directory / child PATH | Resolved | `termio/exec.rs:1193`, `termio/exec_windows.rs:871` |
| Fontconfig filenames | Resolved | `font/deferred_face.rs:75` |
| Relative links / OSC8 | Resolved | `core/surface/mouse.rs:406`, `:1620` |
| Kitty temporary directory | Resolved | `terminal/kitty/graphics_image.rs:568` |
| macOS `.hushlogin` | Resolved | `termio/exec.rs:1443` |
| Benchmark C arguments | Resolved | `lib/benchmark.rs:65` |

| Six additional RV6 findings | Status | Current-code result |
|---|---|---|
| Default `SHELL` | Resolved | `var_os` plus explicit encoding. `config/config.rs:4628` |
| macOS executable-search `PATH` | Resolved | Candidates are constructed from raw bytes. `termio/command.rs:301` |
| Windows config environment overrides | Resolved | Both keys and values decode before insertion. `termio/exec_windows.rs:588` |
| Windows environment-key comparison | **Partial** | Surrogate collisions fixed; Windows casing semantics remain incorrect. Gap 1 below. |
| `+show-config` round-trip | Resolved | Formatted output decodes before stdout. `cli/show_config.rs:85` |
| Non-Apple C API locale initialization | **Partial** | Unix fallback chain implemented; Windows remains a no-op. Gap 2 below. |

The remaining verified gaps are:

1. **Windows environment-key comparison still differs from Windows/Zig.** `char::to_uppercase()` followed by rejecting multi-character mappings is not Windows’ per-unit uppercase mapping. For example, U+1F80 and U+1F88 compare equal under Windows/Zig’s table but remain different here. Conversely, modern Unicode mappings can merge keys Windows keeps distinct. This affects lookup and overrides. [env.rs:18](../../../crates/ghostty-os/src/env.rs:18), [Windows/Zig mapping:133](/usr/lib/zig/std/os/windows/nls.zig:133).

2. **Windows C API locale initialization remains unimplemented.** `ghostty_init → init_lib → ensure_locale` reaches the `cfg(not(unix))` function returning `None`. Zig calls its locale initialization and fallback chain, including Windows environment-update implementations. [locale.rs:24](../../../crates/ghostty-os/src/locale.rs:24), [Zig global.zig:164](../../../../ghostty/src/global.zig:164), [Zig locale.zig:32](../../../../ghostty/src/os/locale.zig:32).

3. **Bash `--rcfile` / `--init-file` paths lose their decoded identity.** The intercepted argument remains config-encoded when inserted into `GHOSTTY_BASH_RCFILE`. A pathname containing byte `FF` therefore becomes literal escape characters in the environment, so integration sources the wrong pathname. [shell_integration.rs:266](../../../crates/ghostty-termio/src/shell_integration.rs:266). Zig inserts the original bytes at [shell_integration.zig:387](../../../../ghostty/src/termio/shell_integration.zig:387).

4. **Configured `TERM` is exported without decoding on Unix and Windows.** With resources available, both implementations insert `cfg.term` directly. Non-UTF-8 config bytes and literal U+FDD0 consequently reach the child incorrectly. [exec.rs:804](../../../crates/ghostty-termio/src/exec.rs:804), [exec_windows.rs:627](../../../crates/ghostty-termio/src/exec_windows.rs:627). Zig preserves the value’s bytes at [Exec.zig:639](../../../../ghostty/src/termio/Exec.zig:639).

5. **GTK discards non-UTF-8 inherited `LANG`.** `std::env::var("LANG").ok()` treats it as absent; surface environment construction then removes `LANG`. This remains observable when, for example, `LC_ALL=C` lets locale initialization succeed despite a non-UTF-8 `LANG`. [application.rs:401](../../../crates/ghostty-apprt-gtk/src/class/application.rs:401), [surface.rs:1444](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:1444). Zig saves the original bytes at [application.zig:270](../../../../ghostty/src/apprt/gtk/class/application.zig:270).

6. **Windows Kitty file paths reject valid WTF-8 lone surrogates.** The non-Unix `OsStrExt::from_bytes` substitutes an empty pathname whenever `from_utf8` fails. Both canonicalization and subsequent file opening use it. [graphics_image.rs:24](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:24), [graphics_image.rs:140](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:140). Zig passes the original pathname to `realpath` at [graphics_image.zig:89](../../../../ghostty/src/terminal/kitty/graphics_image.zig:89).

7. **Malformed Windows byte sequences are silently replaced instead of rejected.** `wtf8_to_wide` maps malformed input such as byte `FF` to U+FFFD, and process creation uses that result. Zig’s process-launch conversions are fallible: malformed input fails instead of selecting a replacement-character executable/path or changing arguments. [osbytes.rs:163](../../../crates/ghostty-os/src/osbytes.rs:163), [command_windows.rs:134](../../../crates/ghostty-termio/src/command_windows.rs:134), [Zig Command.zig:261](../../../../ghostty/src/Command.zig:261).

8. **Embedded C API clipboard and binding-action input remain lossy.** Clipboard completion and `ghostty_surface_binding_action` replace invalid UTF-8. For example, clipboard byte `FF`, or action bytes `text:` followed by `FF`, become U+FFFD instead of the original byte. [embedded.rs:1938](../../../crates/ghostty-lib/src/embedded.rs:1938), [embedded.rs:1979](../../../crates/ghostty-lib/src/embedded.rs:1979). Zig passes byte slices at [embedded.zig:1973](../../../../ghostty/src/apprt/embedded.zig:1973) and [embedded.zig:1995](../../../../ghostty/src/apprt/embedded.zig:1995).

9. **Config-loaded text/CSI/ESC bindings leak the reversible encoding into PTY output.** The file reader encodes raw bytes, but these action handlers write the resulting strings without decoding. A literal U+FDD0 in a `text:` binding is doubled; byte `FF` becomes the UTF-8 escape representation. [cli_args.rs:811](../../../crates/ghostty-config/src/cli_args.rs:811), [binding.rs:725](../../../crates/ghostty-core/src/surface/binding.rs:725), [binding.rs:739](../../../crates/ghostty-core/src/surface/binding.rs:739). Zig’s parser copies unescaped bytes directly at [string.zig:17](../../../../ghostty/src/config/string.zig:17).

Other inspected stub markers matched upstream limitations, stale comments, or implementations elsewhere. The literal export-name comparison found no missing active Zig C/WASM exports; `app_new` and `app_free` are commented out upstream.

| Verdict count | Result |
|---|---:|
| Original rows resolved / partial | **13 / 0** |
| Previous sweep findings resolved / partial | **10 / 0** |
| Six additional RV6 findings resolved / partial | **4 / 2** |
| Newly verified gap groups | **7** |
| Total remaining verified gap groups | **9** |
| Selected existing-binary tests passed / failed | **58 / 0** |

No rebuild or native macOS/Windows/WASM execution was performed. The platform findings are source-verified; passing existing-binary tests do not establish exhaustive parity.