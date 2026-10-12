# Codex final verification (RV10, 2026-10-12)

**Parity remains incomplete: 8 verified gap groups remain—1 P1, 3 P2, and 4 P3.** All nine original RV8 defects and all six original RV9 reproductions are resolved, but Z11 introduced a counted-buffer regression.

Read-only review of HEAD `44f39fb1d3a34354797e22d8b21a9c0790a4fd63`. I modified no files.

| RV8 finding | Status | Current-code evidence |
|---|---|---|
| 1. Windows malformed filesystem bytes | Resolved | Fallible WTF-8 conversion: [osbytes.rs:422](../../../crates/ghostty-os/src/osbytes.rs:422). |
| 2. Windows process inputs accept surrogates | Resolved | Strict UTF-8 validation: [command_windows.rs:142](../../../crates/ghostty-termio/src/command_windows.rs:142). |
| 3. Kitty paths reject separately encoded surrogate pairs | Resolved | Conversion accepts these sequences: [osbytes.rs:186](../../../crates/ghostty-os/src/osbytes.rs:186). |
| 4. Literal U+FDD0 key trigger | Resolved | Original UTF-8 parsed directly: [binding/mod.rs:552](../../../crates/ghostty-input/src/binding/mod.rs:552). |
| 5. Invalid selection delimiters | Resolved | Original-byte validation: [types_a.rs:661](../../../crates/ghostty-config/src/types_a.rs:661). |
| 6. Invalid clipboard replacements | Resolved | UTF-8 validation: [types_a.rs:953](../../../crates/ghostty-config/src/types_a.rs:953). |
| 7. Encoded font styles/family names | Resolved | Decoded bytes reach discovery: [discovery.rs:37](../../../crates/ghostty-font/src/discovery.rs:37). |
| 8. C title actions lose invalid bytes | Resolved | Raw bytes retained: [action_c.rs:129](../../../crates/ghostty-lib/src/action_c.rs:129). |
| 9. D-Bus encoding/substitution | Resolved | Plain argument strings and whole-value substitution: [new_window.rs:50](../../../crates/ghostty-apprt-gtk/src/ipc/new_window.rs:50), [flatpak.rs:2](../../../crates/ghostty-os/src/flatpak.rs:2). |

| RV9 finding | Original reproduction |
|---|---|
| 1. Fontconfig interior NUL | Resolved: prefix retained. |
| 2. Numeric parsing/formatting | Reported cases resolved; two additional formatting gaps below. |
| 3. C getters/title interior NUL | Resolved for sentinel strings; counted-buffer regression below. |
| 4. GTK Unicode whitespace trimming | Resolved: explicit ASCII trimming. |
| 5. Config command tokenization | Resolved: Zig-compatible iterator. Separate benchmark tokenizer differs. |
| 6. Config diagnostic error kinds | Reported cases resolved. Additional surrogate error differs below. |

The sweep covered the **461 Zig files** through source inventory, export checks, implementation mapping, and targeted behavioral comparisons. All **161 discovered literal export names** have counterparts; that establishes presence, not behavioral equivalence.

The following gaps are concrete:

1. **P1 — C key-table action can advertise more bytes than its allocation contains.**  
   [action_c.rs:43](../../../crates/ghostty-lib/src/action_c.rs:43) truncates at the first NUL, while [action_c.rs:160](../../../crates/ghostty-lib/src/action_c.rs:160) reports the original length. Both implementations accept a table named `a<NUL>b` and its activation binding. Rust then supplies a two-byte terminated buffer with `len = 3`; a callback reading the advertised slice reads beyond it. Zig passes the complete backing slice in [action.zig:818](../../../../ghostty/src/apprt/action.zig:818). Configuration acceptance was tested live; the allocation/callback mismatch was verified from source.

2. **P2 — Benchmark CLI parsing and C tokenization differ.**  
   [args.rs:81](../../../crates/ghostty-bench/src/args.rs:81) discards earlier duplicate arguments before validation; [args.rs:95](../../../crates/ghostty-bench/src/args.rs:95) rejects empty numeric values instead of resetting defaults. Live examples: `--terminal-rows=` succeeds in Zig and fails in Rust; `--terminal-rows=bad --terminal-rows=10` fails in Zig and succeeds in Rust. `--data=` also differs. Additionally, [benchmark.rs:23](../../../crates/ghostty-lib/src/benchmark.rs:23) strips arbitrary quoted backslashes: C benchmark arguments `--terminal-rows="1\0"`—literal backslash followed by zero—return false in Zig and true in Rust.

3. **P2 — OSC7-derived window titles lose invalid UTF-8 bytes.**  
   [stream_handler.rs:1019](../../../crates/ghostty-termio/src/stream_handler.rs:1019) converts the decoded path lossily before setting the title. `file://localhost/tmp/%FF` therefore produces `/tmp/` plus U+FFFD instead of Zig’s raw `FF` byte. Expansion can also push a previously acceptable title beyond the byte-length limit. Zig forwards the raw path at [stream_handler.zig:1199](../../../../ghostty/src/termio/stream_handler.zig:1199).

4. **P2 — Notification and hovered-link callbacks still replace invalid bytes.**  
   Notification title/body conversion at [surface/mod.rs:963](../../../crates/ghostty-core/src/surface/mod.rs:963) and OSC8 URI conversion at [mouse.rs:1819](../../../crates/ghostty-core/src/surface/mouse.rs:1819) replace `FF` with UTF-8 `EF BF BD` before C actions receive the payload. Zig preserves the original bytes. These are source-verified callback differences, regardless of how a UI subsequently renders the text.

5. **P3 — C diagnostic getter exposes internal byte encoding.**  
   [config_capi.rs:577](../../../crates/ghostty-lib/src/config_capi.rs:577) exports formatted diagnostic text without decoding its internal representation. Live input `font-size=<FF>` yields raw `FF` inside Zig’s diagnostic, but `EF B7 90 C3 BF` in Rust’s diagnostic.

6. **P3 — Escaped-surrogate diagnostic has the wrong error kind.**  
   [string.rs:91](../../../crates/ghostty-config/src/string.rs:91) maps an unencodable surrogate to `InvalidString`. Live input `input=\u{d800}` reports `Utf8CannotEncodeSurrogateHalf` in Zig and `InvalidString` in Rust. Both reject the value.

7. **P3 — Two custom config formatters emit `NaN` instead of `nan`.**  
   [types_a.rs:738](../../../crates/ghostty-config/src/types_a.rs:738) and [types_b.rs:571](../../../crates/ghostty-config/src/types_b.rs:571) use Rust float formatting. The shared formatter, exercised through WASM, accepts and outputs `font-variation = wght=NaN` and `quick-terminal-size = NaN%`; Zig formats both with lowercase `nan`. These paths also serve `+show-config`.

8. **P3 — Embedded configuration applies GTK-only finalization.**  
   [config.rs:4547](../../../crates/ghostty-config/src/config.rs:4547) unconditionally resolves `gtk-single-instance=detect`. Zig restricts this to the GTK runtime at [Config.zig:4610](../../../../ghostty/src/config/Config.zig:4610). In live embedded-library tests, the finalized getter returned `detect` in Zig and `false` in Rust.

Validation included **48,076 config field/value cases across 202 fields**, **82,810 OSC command-type/title comparisons**, WASM Node smoke checks, and **2,206 passing existing-binary tests**, with one ignored and 17 filtered tests. No fresh build was performed. Concurrent edits appeared in the workspace, including VT decoding and page-list code; those edits are not certified by the existing-binary results.

**Inherently unverifiable natively on this Linux machine:** macOS Metal/IOSurface, CoreText, AppKit/Carbon integration and Mach crash handling; Windows ConPTY, `CreateProcessW`, native environment/filesystem behavior and crash handling. Portable tests and source inspection cannot replace execution on those systems.

**Verdict counts:** RV8 **9 resolved / 0 unresolved**; RV9 original reproductions **6 resolved / 0 unresolved**; remaining verified gaps **8: 1 P1, 3 P2, 4 P3**. No wholly absent feature was established. The sweep does not establish exhaustive behavioral parity.