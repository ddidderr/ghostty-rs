# Codex final verification (RV11, 2026-10-12)

**Parity remains incomplete.** At HEAD `38c6ad670a60e400293ef27a6ce5699b66ed4e3c`, RV10 has **7 fully resolved findings and 1 partially resolved finding**. I verified **3 remaining port-gap groups: 0 P1, 2 P2, 1 P3**, plus one benign output divergence.

The worktree remains clean. No files were modified and no build was performed.

| RV10 finding | Verification |
|---|---|
| 1. Counted C-action buffers | **Resolved.** Full bytes, including interior NULs, are allocated; advertised lengths match. Existing allocation tests pass. |
| 2. Benchmark argument parsing | **Partial.** Original numeric-reset, duplicate-number, empty-data and quoted-backslash cases match. Duplicate enum values still differ; reproduction below. |
| 3. OSC7-derived titles | **Resolved.** Raw decoded path bytes reach the title buffer; regression test passes. |
| 4. Notification/hover-link bytes | **Resolved.** Raw bytes survive core/runtime/C conversion. Notification and counted-buffer tests pass. |
| 5. Diagnostic byte encoding | **Resolved.** Live C-library comparison returns raw `FF` identically. |
| 6. Escaped-surrogate diagnostic | **Resolved.** Both return `Utf8CannotEncodeSurrogateHalf`. |
| 7. NaN formatting | **Resolved.** Current WASM artifact prints `wght=nan` and `nan%`. |
| 8. Embedded GTK finalization | **Resolved.** Both embedded libraries retain `gtk-single-instance=detect` after finalization. |

No regression of the original reproductions was established.

The remaining verified gaps are:

1. **P2 — Benchmark duplicate enum arguments bypass earlier validation.**  
   [`args.rs:116`](../../../crates/ghostty-bench/src/args.rs:116) retains the last string; [`benches.rs:262`](../../../crates/ghostty-bench/src/benches.rs:262) validates only that result.  
   Reproduction: `+codepoint-width --mode=bad --mode=noop`. Zig exits **1** with `InvalidValue`; Rust’s C benchmark entry returns **true**. Also reproduced for `grapheme-break` and `is-symbol`, and with a trailing empty reset. This is the residual RV10 finding 2.

2. **P2 — Theme loading still rejects interior-NUL paths instead of opening their prefix.**  
   [`theme.rs:244`](../../../crates/ghostty-config/src/theme.rs:244), also the named-theme path at [`theme.rs:200`](../../../crates/ghostty-config/src/theme.rs:200).  
   Live reproduction: an absolute path to the existing `0x96f` theme followed by `<NUL>tail`. The ReleaseFast Zig oracle loads it, yielding background **`#262427`** without diagnostics. Rust rejects it and retains **`#282c34`**. Without the suffix, both load identically.

3. **P3 — Filesystem diagnostics expose Rust error descriptions.**  
   [`path.rs:167`](../../../crates/ghostty-config/src/path.rs:167), [`theme.rs:257`](../../../crates/ghostty-config/src/theme.rs:257).  
   Live reproduction: `theme=/etc/passwd/x`. Zig reports **`error.NotDir`**; Rust reports **`Not a directory (os error 20)`**. Relative-path expansion produces the same discrepancy. Both reject the path.

There is also a **benign observable divergence**: piped `+list-colors` produces **15,434 bytes** in Rust versus **12,297 bytes** in the installed oracle. Zig omits its final buffer flush and stops midway through `PeachPuff3`; Rust explicitly flushes at [`list_colors.rs:68`](../../../crates/ghostty-cli/src/list_colors.rs:68). This improves output completeness; I have separated it from the three port defects.

The fresh sweep covered the **461-file source inventory**, implementation mappings, public-function/export checks and stub searches across terminal, GTK, fonts/rendering, configuration/input, OS/termio, C/WASM, crash, CLI and build/distribution code. **159 active literal ABI names had counterparts**; presence does not establish behavioral equivalence. No wholly absent feature was established.

Validation included:

- **14,746 differential configuration cases across all 202 fields**, followed by targeted reproductions.
- **2,495 distinct existing-binary tests passed**; four ignored, 17 excluded, and five artifact-writing tests blocked by the read-only filesystem.
- Current WASM Node smoke test passed.
- CLI actions, default keybindings, plain theme listing and font listing matched. Default configuration differed only by expected debug-build `gtk-opengl-debug`.
- The documented **390,000-case terminal campaign** remains supporting evidence; I did not rerun it. Its genuine upstream memory bugs/UB were excluded from gap counts.

**Inherently unverifiable natively on this Linux machine:** macOS/iOS Metal, IOSurface, CoreText, AppKit/UIKit/Carbon integration, Mach crash handling and Apple framework packaging; Windows ConPTY, `CreateProcessW`, native filesystem/environment behavior and exception handling.

Linux GPU/compositor, portal/D-Bus and interactive GTK integration were **not exercised in this read-only pass**. Those are outstanding runtime checks, rather than inherently untestable Linux functionality. The sweep does not certify exhaustive behavioral equivalence across all 461 files.