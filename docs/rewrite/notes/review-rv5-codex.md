# Codex final targeted check (RV5, 2026-10-12, commit 72bfac1)

Audited **`72bfac1cc8b2`**, read-only. **7 of the 13 rows are resolved; 6 remain partial.** No files changed.

Locations below abbreviate `crates/ghostty-<crate>/src/` as `<crate>/`.

| Previous row | Status | Current-code finding |
|---|---|---|
| `Surface.zig` | **Still partial** | Flatpak cwd/argv and screen-file paste/open now preserve bytes. **Copy** still replaces invalid filename bytes with U+FFFD: [core/surface/binding.rs:635](../../../crates/ghostty-core/src/surface/binding.rs:635). **Regression:** the opener now decodes every URL, although regex/OSC8 callers supply ordinary, unencoded text; literal U+FDD0 followed by U+00FF becomes byte `FF`: [os/open.rs:33](../../../crates/ghostty-os/src/open.rs:33), [core/surface/mouse.rs:1663](../../../crates/ghostty-core/src/surface/mouse.rs:1663). |
| `renderer/shadertoy.zig` | **Resolved** | The reconstructed plain array supports whole-array copies and function arguments while retaining the padded uniform layout: [renderer/shadertoy.rs:265](../../../crates/ghostty-renderer/src/shadertoy.rs:265). Selected shader tests passed. |
| `cli/args.zig` | **Still partial** | Literal U+FDD0 escaping is fixed. Windows OS strings containing **unpaired UTF-16 surrogates** still lose data through `to_string_lossy`; decoding also replaces their WTF-8 representation. Zig’s Windows argument handling preserves WTF-8: [os/osbytes.rs:75](../../../crates/ghostty-os/src/osbytes.rs:75), [90](../../../crates/ghostty-os/src/osbytes.rs:90), [103](../../../crates/ghostty-os/src/osbytes.rs:103). |
| `config/Config.zig` | **Resolved** | Template directory/file creation decodes the pathname; template substitution uses original bytes: [config/config.rs:3944](../../../crates/ghostty-config/src/config.rs:3944), [3957](../../../crates/ghostty-config/src/config.rs:3957). |
| `apprt/gtk/class/surface.zig` | **Resolved** | Confirmation carries an explicit encoding flag and decodes only encoded byte pastes: [apprt-gtk/class/surface.rs:2668](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:2668), [2712](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:2712). |
| `os/resourcesdir.zig` | **Still partial** | Unix resource exports, shell integration and gettext are repaired. **Windows was missed:** `GHOSTTY_RESOURCES_DIR` and `TERMINFO` still consume encoded strings directly. A resource path containing literal U+FDD0 is exported with the doubled escape: [termio/exec_windows.rs:606](../../../crates/ghostty-termio/src/exec_windows.rs:606), [618](../../../crates/ghostty-termio/src/exec_windows.rs:618). |
| `config/path.zig` | **Resolved** | The reported custom-shader consumer now decodes canonicalized paths before loading: [renderer/shadertoy.rs:127](../../../crates/ghostty-renderer/src/shadertoy.rs:127). A separate background-image consumer remains broken; see sweep. |
| `cli/list_themes.zig` | **Resolved** | Preview hyperlinks percent-encode decoded pathname bytes: [cli/list_themes.rs:79](../../../crates/ghostty-cli/src/list_themes.rs:79). |
| `lib/allocator.zig` | **Still partial** | Inline caller-owned storage and borrowed payloads remove the reported `Box`/payload-copy allocations. Color-command request lists still allocate globally: [vt/osc/parsers/color.rs:232](../../../crates/ghostty-vt/src/osc/parsers/color.rs:232), [vt/osc/parsers/kitty_color.rs:57](../../../crates/ghostty-vt/src/osc/parsers/kitty_color.rs:57). Dropping these before returning does not restore caller allocator control. |
| `terminal/c/osc.zig` | **Still partial** | `ghostty_osc_end` still invokes those allocating color parsers: [vt-capi/osc.rs:144](../../../crates/ghostty-vt-capi/src/osc.rs:144). The zero-global-allocation test omits color commands: [vt-capi/osc.rs:310](../../../crates/ghostty-vt-capi/src/osc.rs:310). |
| `crash/sentry.zig` | **Still partial** | AArch64 context handling is repaired; macOS/Windows minidump paths exist. **macOS regression:** all other threads—including the faulting thread—are explicitly suspended, never resumed, and the watchdog is cancelled before returning `KERN_FAILURE`. This can leave the process hung instead of completing BSD signal termination: [crash/mach_handler.rs:289](../../../crates/ghostty-crash/src/mach_handler.rs:289), [305](../../../crates/ghostty-crash/src/mach_handler.rs:305). macOS SIGABRT still lacks a minidump; capture is Linux-only in the signal path: [crash/signal.rs:215](../../../crates/ghostty-crash/src/signal.rs:215). |
| `tripwire.zig` | **Resolved** | `CopyMap` failure injection and the cleanup/recovery test are present: [terminal/screen.rs:2619](../../../crates/ghostty-terminal/src/screen.rs:2619), [terminal/screen/tests/p5.rs:689](../../../crates/ghostty-terminal/src/screen/tests/p5.rs:689). |
| `build/GhosttyLibVt.zig` | **Resolved** | The linker sets the versioned SONAME; installation no longer depends on `patchelf`: [vt-capi/build.rs:23](../../../crates/ghostty-vt-capi/build.rs:23). Existing ELF verified as `libghostty-vt.so.0`. |

The broader sweep found these **10 additional functional gap groups**, excluding defects already listed above:

1. **GUI “Open Config” uses escaped pathnames as real paths.** Candidate selection and directory/file creation can miss the actual config or create a second file at the encoded pathname. [config/edit.rs:21](../../../crates/ghostty-config/src/edit.rs:21), [52](../../../crates/ghostty-config/src/edit.rs:52).

2. **Background-image loading does not decode config paths.** A non-UTF-8 filename, including a symlink’s canonical target, fails to load. [renderer/generic.rs:1099](../../../crates/ghostty-renderer/src/generic.rs:1099).

3. **The default crash directory retains the escaped XDG/HOME representation.** Reports are written/read under the wrong pathname. [crash/dir.rs:22](../../../crates/ghostty-crash/src/dir.rs:22).

4. **The SSH cache has the same XDG decoding omission.** [cli/ssh_cache/disk_cache.rs:62](../../../crates/ghostty-cli/src/ssh_cache/disk_cache.rs:62).

5. **Executable-directory discovery corrupts child environment paths.** Lossy conversion feeds both `GHOSTTY_BIN_DIR` and appended `PATH`, breaking discovery of Ghostty installed beneath a non-UTF-8 directory. [termio/exec.rs:1191](../../../crates/ghostty-termio/src/exec.rs:1191). Windows has the analogous surrogate-loss path at [termio/exec_windows.rs:855](../../../crates/ghostty-termio/src/exec_windows.rs:855).

6. **Fontconfig filenames are decoded lossily before opening.** Fonts discovered beneath non-UTF-8 paths cannot be loaded reliably. [font/fontconfig.rs:226](../../../crates/ghostty-font/src/fontconfig.rs:226), consumed at [font/deferred_face.rs:76](../../../crates/ghostty-font/src/deferred_face.rs:76).

7. **File-link opening still loses pathname information.** Relative links join against encoded terminal cwd without decoding; OSC8 links replace invalid bytes before opening. [core/surface/mouse.rs:405](../../../crates/ghostty-core/src/surface/mouse.rs:405), [1614](../../../crates/ghostty-core/src/surface/mouse.rs:1614).

8. **Kitty temporary-file validation uses lossy `TMPDIR`/`TMP`.** A valid temporary image under a non-UTF-8 temp directory outside the hardcoded prefixes is rejected. [terminal/kitty/graphics_image.rs:591](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:591), consumed at [569](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:569).

9. **macOS `.hushlogin` lookup uses encoded passwd-home directly.** It misses the file and fails to add login’s quiet option. [termio/exec.rs:1442](../../../crates/ghostty-termio/src/exec.rs:1442).

10. **Benchmark C arguments lose non-UTF-8 data-file paths.** Conversion happens before argument parsing and `File::open`. [lib/benchmark.rs:65](../../../crates/ghostty-lib/src/benchmark.rs:65), [bench/args.rs:82](../../../crates/ghostty-bench/src/args.rs:82).

The requested non-test marker sweep found **no additional demonstrated functional gaps**: integration TODOs are already wired or concern type consolidation; the GTK `unimplemented!` is an unknown-property fallback.

| Count | Result |
|---|---:|
| Targeted rows resolved | **7** |
| Targeted rows still partial | **6** |
| Targeted rows still missing | **0** |
| Additional byte/path sweep finding groups | **10** |
| Additional functional marker findings | **0** |
| Selected in-memory tests passed | **37** |

Tests used existing binaries; no rebuild or native macOS/Windows execution was performed. Working tree remains clean.