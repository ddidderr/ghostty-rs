# Codex final verification (RV8, 2026-10-12)

**Parity remains partial: 9 verified gap groups remain.** Audited `e47009e21e8cba1f931e18a9b14efa037202f519` read-only; no files changed.

“Resolved” means the reported defect is fixed, not that the entire module has parity.

| Review item | Status | Current-code verification |
|---|---|---|
| RV7 #1: Windows environment-key casing | Resolved | Uses Zig’s per-unit mapping. All 2,544 table entries match. [env.rs:18](../../../crates/ghostty-os/src/env.rs:18) |
| #2: Windows locale initialization | Resolved | Windows now runs the locale fallback chain, including environment updates. [locale.rs:69](../../../crates/ghostty-os/src/locale.rs:69) |
| #3: Bash rcfile bytes | Resolved | Decodes before inserting `GHOSTTY_BASH_RCFILE`. [shell_integration.rs:268](../../../crates/ghostty-termio/src/shell_integration.rs:268) |
| #4: Configured `TERM` | Resolved | Both platforms decode before exporting. [exec.rs:805](../../../crates/ghostty-termio/src/exec.rs:805), [exec_windows.rs:639](../../../crates/ghostty-termio/src/exec_windows.rs:639) |
| #5: GTK inherited `LANG` | Resolved | Saved as `OsString` using `var_os`, then restored without UTF-8 conversion. [application.rs:402](../../../crates/ghostty-apprt-gtk/src/class/application.rs:402) |
| #6: Windows Kitty paths | **Partial** | Lone surrogates work; the new decoder rejects another representation Zig accepts. Finding 3 below. |
| #7: Windows malformed-byte handling | **Partial** | Launch rejects byte `FF`, but conversion contracts still differ. Findings 1–2. |
| #8: C clipboard completion / binding actions | Resolved | Clipboard remains bytes; action input enters the reversible encoding. [embedded.rs:1946](../../../crates/ghostty-lib/src/embedded.rs:1946), [embedded.rs:1990](../../../crates/ghostty-lib/src/embedded.rs:1990) |
| #9: Text/CSI/ESC binding output | Resolved | Decodes payload bytes; CSI/ESC length limit applies after decoding. [binding.rs:723](../../../crates/ghostty-core/src/surface/binding.rs:723) |
| Z9: C key-event text limitation | Resolved | Byte slices now reach the encoder, including through VT C API. [key_event.rs:35](../../../crates/ghostty-vt-capi/src/key_event.rs:35) |
| Z9/Z10: Invalid preedit limitation | Resolved | Clears prior preedit, marks dirty, then returns an error, matching Zig. [surface/mod.rs:1694](../../../crates/ghostty-core/src/surface/mod.rs:1694) |
| Z9: Invalid clipboard replacement strings | **Partial** | Replacement conversion masks missing input validation. Finding 6. |
| Z9: Flatpak / new-window D-Bus limits | **Partial** | UTF-8 transport limitations are shared; Rust additionally changes conversion/wire behavior. Finding 9. |

The sweep found these concrete remaining gaps:

1. **Windows filesystem paths still replace malformed bytes.** `EncodedOs::to_path()` reaches the lossy Windows decoder. A config pathname containing `FF` can therefore open the corresponding U+FFFD pathname instead of failing. Config loading and renderer assets use this route. [osbytes.rs:95](../../../crates/ghostty-os/src/osbytes.rs:95), [file_load.rs:162](../../../crates/ghostty-config/src/file_load.rs:162). Zig’s Windows path conversion returns `BadPathName`. [Threaded.zig:6652](/usr/lib/zig/std/Io/Threaded.zig:6652)

2. **Windows process launch accepts surrogate bytes Zig rejects.** Rust accepts `ED A0 80` through `try_to_wide`; inherited environment values also pass directly through UTF-16. Zig’s `Command` uses strict **UTF-8**, not WTF-8, for application, cwd, command line and environment. [command_windows.rs:138](../../../crates/ghostty-termio/src/command_windows.rs:138), [windows_args.rs:20](../../../crates/ghostty-termio/src/windows_args.rs:20), [Command.zig:261](../../../../ghostty/src/Command.zig:261), [Command.zig:487](../../../../ghostty/src/Command.zig:487)

3. **The new strict decoder over-rejects Windows Kitty path input.** `ED A0 BD ED B8 80` decodes to a surrogate pair, then re-encodes as `F0 9F 98 80`; the equality check rejects it. Zig explicitly permits paired surrogate encodings in its WTF-8 validation. This is a Z9 decoder regression relative to Zig’s accepted inputs. [osbytes.rs:192](../../../crates/ghostty-os/src/osbytes.rs:192), [graphics_image.rs:38](../../../crates/ghostty-terminal/src/kitty/graphics_image.rs:38), [unicode.zig:1630](/usr/lib/zig/std/unicode.zig:1630)

4. **A literal U+FDD0 Unicode key trigger is rejected.** Config input doubles that character, but trigger parsing counts encoded characters directly and sees two. Zig accepts the original single codepoint. [binding/mod.rs:617](../../../crates/ghostty-input/src/binding/mod.rs:617), [Binding.zig:1730](../../../../ghostty/src/input/Binding.zig:1730)

5. **`selection-word-chars` parses encoding artifacts as delimiters.** Raw byte `FF` becomes delimiter codepoints U+FDD0 and U+00FF instead of producing `InvalidValue`. [types_a.rs:639](../../../crates/ghostty-config/src/types_a.rs:639), [string.rs:147](../../../crates/ghostty-config/src/string.rs:147), [Config.zig:6143](../../../../ghostty/src/config/Config.zig:6143)

6. **`clipboard-codepoint-map` accepts invalid UTF-8 replacements.** The parser incorrectly assumes its encoded `&str` proves the original bytes valid. A replacement containing `FF` is accepted and later becomes U+FFFD; Zig rejects the configuration value. [types_a.rs:922](../../../crates/ghostty-config/src/types_a.rs:922), [clipboard.rs:421](../../../crates/ghostty-core/src/surface/clipboard.rs:421), [Config.zig:8212](../../../../ghostty/src/config/Config.zig:8212)

7. **Font styles and codepoint-map family names remain encoded during lookup.** Unlike `font-family`, these consumers never decode. A literal U+FDD0 therefore becomes two characters in the Fontconfig/CoreText lookup name. [shared_grid_set.rs:505](../../../crates/ghostty-font/src/shared_grid_set.rs:505), [types_a.rs:875](../../../crates/ghostty-config/src/types_a.rs:875), [discovery.rs:40](../../../crates/ghostty-font/src/discovery.rs:40). Zig retains the original names. [SharedGridSet.zig:604](../../../../ghostty/src/font/SharedGridSet.zig:604), [Config.zig:7940](../../../../ghostty/src/config/Config.zig:7940)

8. **C title actions still lose bytes.** `set_surface_title:` or `set_tab_title:` followed by `FF` becomes U+FFFD before the embedder callback. Configured titles have the same conversion. Zig forwards the original bytes. [binding.rs:938](../../../crates/ghostty-core/src/surface/binding.rs:938), [surface/mod.rs:610](../../../crates/ghostty-core/src/surface/mod.rs:610), [Surface.zig:5486](../../../../ghostty/src/Surface.zig:5486)

9. **D-Bus string handling differs beyond the shared UTF-8 restriction.** New-window sends and expects the internal encoding. An ordinary GTK action argument containing literal U+FDD0 followed by `é` can consequently decode into different bytes; Zig treats it as ordinary text. [new_window.rs:38](../../../crates/ghostty-apprt-gtk/src/ipc/new_window.rs:38), [application.rs:1141](../../../crates/ghostty-apprt-gtk/src/class/application.rs:1141), [application.zig:1717](../../../../ghostty/src/apprt/gtk/class/application.zig:1717). Flatpak additionally replaces invalid bytes itself; a local GLib probe of Zig’s construction produced `[Invalid UTF-8]` for the whole invalid value instead. [flatpak.rs:152](../../../crates/ghostty-os/src/flatpak.rs:152), [flatpak.zig:349](../../../../ghostty/src/os/flatpak.zig:349)

The inspected non-test stub markers matched upstream limitations or implementations elsewhere. The literal C/WASM export-name sweep found no unmatched names; that does not establish behavioral parity.

| Verdict count | Result |
|---|---:|
| Nine RV7 rows resolved / partial | **7 / 2** |
| Z9 key-text / preedit limitations resolved | **2 / 2** |
| Remaining verified gap groups | **9** |
| Additional wholly unimplemented features verified | **0** |
| Literal C/WASM export names checked / unmatched | **159 / 0** |
| Selected existing-binary tests passed / failed | **121 / 0** |

**Native verification remains separate:** no macOS/Windows hardware execution, WASM host execution, or fresh rebuild was performed. The platform gaps above are source-verified; passing existing binaries cannot certify exhaustive parity across all of `ghostty/src`.