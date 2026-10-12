# Codex final verification (RV16, 2026-10-12)

At **`9094054`**, all RV15 findings are resolved: **5 behavior findings, 1 test defect, and 9 register corrections**. No regression in those fixes was found.

The fresh sweep found **1 P3 functionality gap and 4 P3 deviation-register omissions; 0 P1/P2 findings**. Full parity is therefore not yet established.

| Severity | Verified finding | Location |
|---|---|---|
| P3 | **FreeType-only backend cannot be selected.** Zig supports `-Dfont-backend=freetype`, disabling font discovery. Rust unconditionally enables Fontconfig on non-Apple native targets and provides no corresponding feature. Unregistered. | [build.rs:52](../../../crates/ghostty-font/build.rs:52), [backend.rs:93](../../../crates/ghostty-font/src/backend.rs:93) |
| P3 register omission | **Benchmark `--time` extension.** Rust accepts it and prints timing information; upstream’s benchmark options do not include it. Rust execution verified. | [main.rs:33](../../../crates/ghostty-bench/src/main.rs:33) |
| P3 register omission | **Benchmark `--handler=full` extension.** Rust accepts this alternate terminal handler; upstream only implements the simplified handler. Rust execution verified. | [benches.rs:107](../../../crates/ghostty-bench/src/benches.rs:107) |
| P3 register omission | **Synthetic `+sgr` extension.** Rust generates output for this action; upstream’s action enum contains only `ascii`, `osc`, and `utf8`. Rust execution verified. | [ghostty-synthetic.rs:37](../../../crates/ghostty-bench/src/bin/ghostty-synthetic.rs:37) |
| P3 register omission | **Maximum-length `NOTIFY_SOCKET` safety fix.** With a 108-byte pathname, Zig passes its length check and writes `path[108]`, outside its 108-byte array. Rust’s socket-address constructor safely handles that length. Register row 13 describes the abstract-address fix, but omits this separate bounds fix. Source-verified. | [systemd.rs:128](../../../crates/ghostty-os/src/systemd.rs:128); [Zig:116](../../../../ghostty/src/os/systemd.zig:116) |

The extensions and safety fix need documentation, not removal. No additional incorrect register claim was verified.

Verification covered:

- **All 461 Zig files** at inventory/interface level, with targeted implementation review; **2,801 public function declarations** screened. This is a breadth sweep, not exhaustive behavioral-equivalence proof.
- **386 `+show-face` differential comparisons:** zero differences, including CLI and file-diagnostic buffer boundaries.
- **2,587 config comparisons across 200 keys:** zero differences.
- **2,000 fresh terminal differential cases:** zero differences or aborts. The documented 390k campaign was not rerun.
- **2,668 distinct unit tests passed**, four ignored. One fixture-writing test was blocked by read-only access; other identified fixture-writing tests were excluded.
- **138 C-header declarations** have source counterparts; native export omissions correspond to Apple/WASM gates.
- Existing visual artifacts: **three scenarios and 36 sprite sheets remain pixel-identical**. WASM Node smoke test passed.

**Inherently unavailable for native verification here:** Apple Metal/IOSurface/CoreText, AppKit/UIKit/Carbon, Mach handling and Apple packaging; Windows ConPTY, native process/filesystem/exception behavior; native FreeBSD behavior. Swift was excluded.

**Other execution limits:** no fresh builds or captures, live portal/D-Bus integration, interactive GTK, real-browser canvas rendering, or native io_uring execution. Existing binaries were used after source-tree comparison with HEAD.

No files were modified; the worktree remains clean.