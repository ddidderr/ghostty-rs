# Codex final verification (RV15, 2026-10-12)

**Parity is not yet complete at `433873e`.** Verified behavior findings: **0 P1, 1 P2, 4 P3**. Separately: **one P3 test defect and nine inaccurate deviation-register entries**. No files were modified; the worktree remains clean.

RV14: **five findings are resolved; diagnostic ordering is only partially resolved.**

| RV14 finding | Result |
|---|---|
| Oversized `show-face` codepoint | Fixed; boundary checks match the oracle without panicking. |
| Invalid UTF-8 string | Fixed; validation occurs after the codepoint lookup. |
| Diagnostic ordering | Original reproduction fixed; buffer-overflow cases still diverge. |
| Windows basename | Matches Zig **0.15.2**, including `λ:theme`. |
| Windows rooted paths | Matches Zig **0.15.2** absolute-path semantics. |
| io_uring availability | Extra requirements removed; fallback timeout implementation exists. Native io_uring execution remains unavailable here. |

**Concrete behavior findings**

1. **P2 — Malformed D-Bus arguments deliberately abort the process.** [new_window.rs:54](../../../crates/ghostty-apprt-gtk/src/ipc/new_window.rs:54) calls `process::abort()` for invalid UTF-8 or an interior NUL. The subprocess test confirms SIGABRT. This replicates an upstream input-triggered crash, contrary to the project rule; it is not covered by the register. Verification exercised the serializer, not a live D-Bus session.

2. **P3 — RV14 diagnostic ordering remains divergent at buffer boundaries.** [show_face.rs:153](../../../crates/ghostty-cli/src/show_face.rs:153) writes each diagnostic as one assembled chunk, unlike Zig’s fragmented formatting. With `+show-face --config-default-files=false --cp=65` followed by **23 copies of `--font-size=bad`**, Zig emits the lookup before the final diagnostic’s newline; Rust emits that newline first. **26 of 44 boundary probes differed.**

3. **P3 — URL openers lose inherited stdin.** [open.rs:37](../../../crates/ghostty-os/src/open.rs:37) supplies `/dev/null`. Upstream `os/open.zig` preserves `Child.init`’s inherited stdin. An opener reading stdin therefore receives EOF instead of the inherited input. Source-verified; unregistered.

4. **P3 — URL-opener environment filtering uses a different condition.** [open.rs:46](../../../crates/ghostty-os/src/open.rs:46) removes `LD_LIBRARY_PATH` whenever `SNAP` exists. Zig does so only for a Snap build. A non-Snap build launched with `SNAP` set consequently passes a different environment. Source-verified; unregistered.

5. **P3 — URL-opener output collection is unbounded.** [open.rs:115](../../../crates/ghostty-os/src/open.rs:115) collects all output with `wait_with_output()` and only subsequently truncates the logged stderr. Zig bounds collection to **50 KiB per stream**. The Rust path can consume arbitrarily more memory. Source-verified; unregistered.

**Register corrections**

These are documentation defects, not additional missing functionality. In [DEVIANT_FIXES.md](../../../docs/rewrite/DEVIANT_FIXES.md), nine entries contain incorrect claims:

| Line | Correction |
|---|---|
| 58 | `ZigWriter` writes at the inherited descriptor offset; it does **not always append at EOF**. |
| 84 | WASM is runnable here; the existing Node smoke test passed. |
| 88 | Invalid preedit UTF-8 now returns an error; it is not decoded lossily. |
| 92 | The cited OSC buffer-boundary differences have been addressed by NUL-capacity handling. |
| 95 | The stated `/etc/passwd/x` diagnostic now matches Zig’s `error.NotDir`. |
| 96 | Initial input uses byte-preserving `EncodedOs`; the claimed `\u{…}` conversion is absent. |
| 97 | The Rust compiler-version row is populated—`1.99.0` in the tested binary. |
| 98 | Unix `wcwidth` mode calls libc; it is no longer a no-op. |
| 99 | D-Bus arguments preserve plain string bytes, including U+FDD0; the claimed escape-encoding difference is obsolete. |

The separate **P3 test defect** is at [show_face.rs:307](../../../crates/ghostty-cli/src/show_face.rs:307): running the UTF-8 test by name with `--exact --nocapture` fails because `Config::load()` parses the test runner’s arguments. The equivalent CLI reproduction passes.

**Verification coverage**

- Breadth-screened **all 461 Zig files: 428 non-build and 33 build files**, including counterparts, public declarations, incomplete paths, and build outputs. No wholly missing subsystem was established. This is not exhaustive behavioral-equivalence proof.
- **975 substantive config comparisons matched** after aligning resource directories.
- **1,000 fresh terminal differential cases matched**, with no aborts. The documented 390k campaign was not rerun.
- Main test batch: **3,042 passed**, four ignored, 60 blocked by read-only fixture writes, and the one test defect above.
- Existing captures recomputed to **zero differing pixels** for three visual scenarios and **36 sprite sheets**.
- Checked all **138 C-header declarations** against exports/source counterparts; platform-gated declarations account for native export omissions.
- Used the newer existing debug binary after comparing its build-tree sources with HEAD. The `head-check` binary is stale.

**Inherently unverifiable natively on this Linux machine:** Apple Metal/IOSurface/CoreText, AppKit/UIKit/Carbon, Mach exception handling and Apple packaging; Windows ConPTY, process creation, native filesystem behavior and exception handling. Swift remains excluded.

**Other limits:** read-only access prevented fresh builds and fixture-writing tests; io_uring was unavailable. Fresh compositor/GPU captures, live portal/D-Bus integration, interactive GTK, and real-browser canvas rendering were not exercised.