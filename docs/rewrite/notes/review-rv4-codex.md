# Codex targeted re-audit (RV4, 2026-10-12)

Audited **`86d0d3830f9a`**, read-only. Statuses include concrete regressions in the repaired paths. “Resolved” means the previous gap is closed with no remaining defect identified in this targeted check.

Rust locations below abbreviate `crates/ghostty-<crate>/src/` as `<crate>/`.

| Previous Zig row | Status now | What remains |
|---|---|---|
| `Surface.zig` | still partial | Flatpak cwd still reaches D-Bus as encoded text rather than original bytes: [os/flatpak.rs:363](../../../crates/ghostty-os/src/flatpak.rs:363). Screen-file creation now preserves temporary-directory bytes, but its returned filename remains lossy: [core/surface/binding.rs:444](../../../crates/ghostty-core/src/surface/binding.rs:444). Windows field/import fixes are present. |
| `global.zig` | resolved | — |
| `os/file.zig` | resolved | — |
| `renderer/shadertoy.zig` | still partial | Overload selection and directly indexed uniform reads are fixed. Whole-array loads/copies remain unadapted: the patch changes the uniform member’s wrapper type but only rewrites `.member.inner[index]` accesses: [renderer/shadertoy.rs:449](../../../crates/ghostty-renderer/src/shadertoy.rs:449), [461](../../../crates/ghostty-renderer/src/shadertoy.rs:461). |
| `main_ghostty.zig` | resolved | — |
| `cli/args.zig` | still partial | Unix byte handling is fixed. The non-Unix OS-string encoder does not escape literal U+FDD0, although decoding interprets it as an escape. For example, U+FDD0 followed by U+00FF does not round-trip through the Windows config-argument path: [os/osbytes.rs:103](../../../crates/ghostty-os/src/osbytes.rs:103), [os/args.rs:66](../../../crates/ghostty-os/src/args.rs:66). |
| `config/Config.zig` | still partial | CLI loading/path expansion is fixed, but default-template creation passes encoded paths directly to `create_dir_all` and `write`. A non-UTF-8 HOME/XDG path therefore creates the template at the wrong pathname: [config/config.rs:3945](../../../crates/ghostty-config/src/config.rs:3945). |
| `main_c.zig` | resolved | — |
| `config/CApi.zig` | resolved | — |
| `apprt/embedded.zig` | resolved | — |
| `apprt/gtk/class/surface.zig` | still partial | File-drop bytes are preserved, but confirmation now decodes **every** paste as encoded text, including ordinary UTF-8 passed unencoded by `paste()`. Confirming text containing U+FDD0 followed by U+00FF changes it to byte `FF`: [apprt-gtk/class/surface.rs:2625](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:2625), [2692](../../../crates/ghostty-apprt-gtk/src/class/surface.rs:2692). |
| `os/env.zig` | resolved | — |
| `os/homedir.zig` | resolved | — |
| `os/path.zig` | resolved | — |
| `os/passwd.zig` | resolved | — |
| `os/resourcesdir.zig` | still partial | Discovery preserves bytes, but consumers still use encoded resource strings directly: child `GHOSTTY_RESOURCES_DIR`, terminfo lookup/export, and gettext directory binding: [termio/exec.rs:794](../../../crates/ghostty-termio/src/exec.rs:794), [814](../../../crates/ghostty-termio/src/exec.rs:814), [os/i18n.rs:45](../../../crates/ghostty-os/src/i18n.rs:45). |
| `config/path.zig` | still partial | Canonicalization preserves bytes, but custom-shader loading still constructs `Path::new` from the encoded string. A symlink resolving to a non-UTF-8 shader filename still fails: [renderer/shadertoy.rs:127](../../../crates/ghostty-renderer/src/shadertoy.rs:127). |
| `cli/validate_config.zig` | resolved | — |
| `cli/list_themes.zig` | still partial | Listing/loading preserves bytes, but preview hyperlinks percent-encode the escape representation instead of the original pathname: [cli/list_themes.rs:79](../../../crates/ghostty-cli/src/list_themes.rs:79). |
| `lib/allocator.zig` | still partial | The OSC storage object itself remains globally allocated through `Box::new`, and parser payload copies still allocate global `Vec`s. Caller allocator control is incomplete: [vt-capi/osc.rs:75](../../../crates/ghostty-vt-capi/src/osc.rs:75), [vt/osc/mod.rs:566](../../../crates/ghostty-vt/src/osc/mod.rs:566). |
| `terminal/c/osc.zig` | still partial | Payload storage and returned title use the C allocator, but `ghostty_osc_end` first constructs globally allocated command payload copies. Caller limits/failure behavior do not govern those allocations: [vt-capi/osc.rs:134](../../../crates/ghostty-vt-capi/src/osc.rs:134), [vt/osc/mod.rs:566](../../../crates/ghostty-vt/src/osc/mod.rs:566). |
| `os/i18n.zig` | resolved | — |
| `crash/sentry.zig` | still partial | Darwin still discards signal/machine context and writes an event without native capture: [crash/signal.rs:217](../../../crates/ghostty-crash/src/signal.rs:217). New Linux code also accesses nonexistent AArch64 `mcontext_t.fpregs`, a compilation regression: [crash/minidump.rs:97](../../../crates/ghostty-crash/src/minidump.rs:97). |
| `build_config.zig` | resolved | — |
| `termio/Exec.zig` | resolved | — |
| `termio/stream_handler.zig` | resolved | — |
| `apprt/gtk/class/window.zig` | resolved | — |
| `os/cgroup.zig` | resolved | — |
| `benchmark/Benchmark.zig` | resolved | — |
| `terminal/kitty/graphics_image.zig` | resolved | — |
| `tripwire.zig` | still partial | Named injection, hit counts, and trip verification now exist. The associated selection-string map allocation-failure injection/test remains absent; its explicit skip remains: [terminal/screen/tests/p5.rs:688](../../../crates/ghostty-terminal/src/screen/tests/p5.rs:688). |

| Previous artifact/distribution row | Status now | What remains |
|---|---|---|
| `build/GhosttyWebdata.zig` — three MDX outputs | resolved | — |
| `build/GhosttyXCFramework.zig` / `build/GhosttyLib.zig` — universal library/XCFramework | resolved | — |
| `build/GhosttyLib.zig` — native library/header installation | resolved | — |
| `build/GhosttyLibVt.zig` — native VT distribution | still partial | Installation permits an unversioned SONAME when `patchelf` is absent, despite installing versioned filenames. The build still emits `libghostty-vt.so`: [vt-capi/build.rs:6](../../../crates/ghostty-vt-capi/build.rs:6); optional correction: [scripts/install-lib.sh:48](../../../scripts/install-lib.sh:48). |
| `build/GhosttyResources.zig` — Linux desktop/D-Bus identity | resolved | — |
| `build/GhosttyI18n.zig` — extraction/POT/PO merging | resolved | — |
| `build/GhosttyDist.zig` — versioned source distribution | resolved | — |
| `build/GhosttyExe.zig` — Windows executable resources | resolved | — |

| Scope | Resolved | Still partial | Still missing | Total |
|---|---:|---:|---:|---:|
| Source rows | 19 | 12 | 0 | 31 |
| Artifact/distribution rows | 7 | 1 | 0 | 8 |
| **Overall** | **26** | **13** | **0** | **39** |

The byte codec itself round-trips bytes and leaves valid UTF-8 without U+FDD0 unchanged. **30 selected tests passed using existing binaries**: 6 byte-codec, 19 in-memory shader, and 5 tripwire tests. Packaging was inspected statically; scripts passed syntax checks, and all three Windows resource files match Zig’s copies. No files were modified; the working tree remains clean.