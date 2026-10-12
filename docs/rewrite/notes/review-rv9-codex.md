# Codex final verification (RV9, 2026-10-12)

**All nine specific RV8 defects are resolved. Parity remains incomplete: six additional verified gap groups remain.**

Reviewed `0ed59764e19a2d9ad705f01ed362d1bb949b7ab6`. No files changed.

| RV8 finding | Status | Current-code evidence |
|---|---|---|
| 1. Windows malformed filesystem bytes | Resolved | Checked WTF-8 conversion in [osbytes.rs:417](../../../crates/ghostty-os/src/osbytes.rs:417); config loading propagates failure. |
| 2. Windows process inputs accept surrogates | Resolved | Application, cwd and argv require UTF-8 in [command_windows.rs:138](../../../crates/ghostty-termio/src/command_windows.rs:138); environment conversion also validates UTF-8. |
| 3. Kitty paths reject separately encoded surrogate pairs | Resolved | [osbytes.rs:186](../../../crates/ghostty-os/src/osbytes.rs:186) accepts these pairs without canonical-reencoding comparison. |
| 4. Literal U+FDD0 key trigger | Resolved | Triggers decode original UTF-8 in [binding/mod.rs:179](../../../crates/ghostty-input/src/binding/mod.rs:179). |
| 5. Invalid selection delimiters | Resolved | Original bytes undergo codepoint validation in [types_a.rs:638](../../../crates/ghostty-config/src/types_a.rs:638). |
| 6. Invalid clipboard replacements | Resolved | UTF-8 validation precedes storage in [types_a.rs:930](../../../crates/ghostty-config/src/types_a.rs:930). |
| 7. Encoded font styles/family names | Resolved | Original bytes reach Fontconfig in [discovery.rs:40](../../../crates/ghostty-font/src/discovery.rs:40); CoreText validates decoded bytes. |
| 8. C title actions lose invalid bytes | Resolved | Raw title bytes reach callbacks in [action_c.rs:130](../../../crates/ghostty-lib/src/action_c.rs:130). Interior NUL handling remains a separate gap below. |
| 9. D-Bus internal encoding/substitution | Resolved | Plain argument strings in [new_window.rs:55](../../../crates/ghostty-apprt-gtk/src/ipc/new_window.rs:55); whole-value GLib substitution in [flatpak.rs:2](../../../crates/ghostty-os/src/flatpak.rs:2). |

The fresh inventory, public-function, stub-marker and export sweep covered **461 Zig files**, including platform, C/WASM and build code. Behavioral checks included invalid-byte probes for **all 202 config fields**, 58 additional grammar cases, and targeted consumer checks. This is not exhaustive behavioral-equivalence certification.

Verified sweep findings:

1. **P2 — Interior NULs in font names panic.**  
   `font-family=monospace<NUL>suffix`, `font-style=Regular<NUL>suffix`, and a codepoint-map family containing NUL all make Rust exit **101**; Zig exits **0** and uses the prefix before NUL. `CString::new` rejects the bytes, then discovery asserts success. [fontconfig.rs:194](../../../crates/ghostty-font/src/fontconfig.rs:194), [discovery.rs:40](../../../crates/ghostty-font/src/discovery.rs:40); [Zig discovery.zig:115](../../../../ghostty/src/font/discovery.zig:115).

2. **P2 — Numeric grammars remain narrower than Zig’s.**  
   Differential probes confirm Rust rejects values Zig accepts: `window-height=0X10`, `window-height=1__0`, `font-size=1_4`, `font-size=0x1.cp+3`, `window-padding-x=1_0`, and numeric binding parameters containing underscores. Variations, metric adjustments and quick-terminal sizes also exhibit this mismatch. [cli_args.rs:526](../../../crates/ghostty-config/src/cli_args.rs:526), [cli_args.rs:620](../../../crates/ghostty-config/src/cli_args.rs:620), [types_b.rs:1204](../../../crates/ghostty-config/src/types_b.rs:1204), [action.rs:311](../../../crates/ghostty-input/src/binding/action.rs:311); [Zig args.zig:429](../../../../ghostty/src/cli/args.zig:429).

3. **P2 — C getters and actions delete interior NULs.**  
   A live C API probe loading `title=a<NUL>b` returns `"ab"`. Zig preserves the backing bytes, so a C-string consumer sees `"a"`. Title callbacks use the same deletion policy. [config_capi.rs:111](../../../crates/ghostty-lib/src/config_capi.rs:111), [action_c.rs:45](../../../crates/ghostty-lib/src/action_c.rs:45); [Zig c_get.zig:32](../../../../ghostty/src/config/c_get.zig:32).

4. **P2 — Incoming GTK new-window arguments lose Unicode whitespace.**  
   Rust’s `trim()` removes trailing U+00A0 from `--working-directory=/tmp/name<U+00A0>` and from titles. Zig trims only ASCII whitespace, preserving that pathname/title. Source-verified. [application.rs:1142](../../../crates/ghostty-apprt-gtk/src/class/application.rs:1142); [Zig application.zig:1749](../../../../ghostty/src/apprt/gtk/class/application.zig:1749).

5. **P2 — Shell-command tokenization omits CR/LF separators.**  
   Rust splits only spaces/tabs; Zig’s `ArgIteratorGeneral` also splits CR/LF. Thus `bash<LF>-i` yields one argument in Rust and two in Zig, changing shell detection and integration. Source-verified. [command.rs:74](../../../crates/ghostty-config/src/command.rs:74), [shell_integration.rs:128](../../../crates/ghostty-termio/src/shell_integration.rs:128); [Zig Args.zig:895](/usr/lib/zig/std/process/Args.zig:895).

6. **P3 — Parser diagnostics differ, including a refactor regression.**  
   For raw `FF`, metric adjustments now report `InvalidValue` instead of Zig’s `InvalidFormat`; quick-terminal size reports `InvalidValue` instead of `MissingUnit`; `link` reports `InvalidValue` instead of `NotImplemented`. These were reproduced against both binaries. Early UTF-8 validation changes the field-specific error contract. [metric_modifier.rs:103](../../../crates/ghostty-config/src/metric_modifier.rs:103), [types_b.rs:633](../../../crates/ghostty-config/src/types_b.rs:633), [types_a.rs:1057](../../../crates/ghostty-config/src/types_a.rs:1057).

| Verdict count | Result |
|---|---:|
| RV8 resolved / partial / unresolved | **9 / 0 / 0** |
| Additional verified gap groups | **6: five P2, one P3** |
| Wholly unimplemented features verified | **0** |
| Literal C/WASM export names checked / unmatched | **159 / 0** |
| Existing-binary tests passed / environment-blocked | **464 / 3** |
| WASM Node smoke test | **Passed** |

`+show-config --default --docs` matches except for the expected debug-build `gtk-opengl-debug` value. Three test wrappers were blocked by temporary-file/build requirements; no fresh build was performed.

Native macOS execution—Metal, CoreText, AppKit/Carbon integration and platform callbacks—and native Windows execution—ConPTY, CreateProcessW, filesystem and locale behavior—remain unverifiable on this Linux machine. Source checks and portable conversion tests do not replace those runs.