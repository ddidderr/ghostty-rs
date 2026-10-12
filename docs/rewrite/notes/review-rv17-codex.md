# Codex final verification (RV17, 2026-10-12)

At **`b244567`**, full parity is **not yet established**: **6 P3 findings; no verified P1/P2 findings**.

RV16’s four register omissions are resolved. Its FreeType-only backend finding is resolved on Linux, but remains incomplete for Apple targets. No incorrect claim in the existing deviation register was verified.

| Severity | Verified finding | Location |
|---|---|---|
| P3 | **FreeType-only backend remains unavailable on Apple.** Rust explicitly ignores the feature there; Zig permits `.freetype`, including discovery-free fallback handling on Darwin. Unregistered. | [build.rs:50](../../../crates/ghostty-font/build.rs:50) |
| P3 | **Z19 introduces a feature-unification test defect.** Enabling `ghostty-font/font-backend-freetype` without the CLI forwarding feature selects no discovery, but the CLI test asserts discovery exists. The assertion reproduces compiler error `E0080`. Conversely, enabling the forwarding feature on Apple selects the wrong assertion. | [list_fonts.rs:140](../../../crates/ghostty-cli/src/list_fonts.rs:140) |
| P3 | **Inspector font rendering differs.** Rust uses `16 × display scale` and stb_truetype; Zig uses 32 pixels and FreeType. At scale 1, font size is halved. Neither difference is registered; VP’s scenarios do not exercise the Inspector. | [inspector.rs:32](../../../crates/ghostty-inspector/src/inspector.rs:32) |
| P3 | **Inspector DCS descriptions differ.** Rust formats fields and slices using its own debug syntax; Zig’s `Parser.Action.format` delegates to Zig 0.15.2’s formatting rules. The displayed/filterable description differs. Unregistered. | [termio.rs:269](../../../crates/ghostty-inspector/src/widgets/termio.rs:269) |
| P3 | **Inspector demo appears in release builds with assertions.** Rust gates it on `debug_assertions`, enabled by this repository’s release profile; Zig gates it strictly on `.Debug`, excluding `.ReleaseSafe`. Unregistered. | [surface.rs:137](../../../crates/ghostty-inspector/src/widgets/surface.rs:137) |
| P3 | **Metal buffer allocation correction is unregistered.** Rust allocates twice the required bytes; Zig multiplies that quantity by `sizeof(T)` again. This changes GPU-memory allocation. Preserve the correction and register it. | [buffer.rs:128](../../../crates/ghostty-renderer/src/metal/buffer.rs:128) |

Verification performed:

- **461 Zig files** inventoried and interface-screened; **2,801 public declarations** screened, with targeted implementation comparisons. This is a breadth audit, not exhaustive behavioral-equivalence proof.
- **1,980 config comparisons across 198 keys**, plus **166 CLI comparisons**: zero mismatches.
- **2,000 fresh terminal differential cases**: 1,970 identical results, 30 Zig aborts, zero Rust aborts or defined-result mismatches. The documented 390k campaign was not rerun.
- **2,517 successful unit-test executions**, including both font variants; seven ignored executions. Fixture-writing tests were excluded.
- Existing **three visual captures and 36 sprite sheets** remain pixel-identical. WASM Node smoke test passed.
- Linux FreeType-only CLI behavior, argument ordering, fallback gates, and the maximum-length `NOTIFY_SOCKET` regression were checked.

**Inherently unavailable for native verification on Linux:** Apple Metal/IOSurface/CoreText, Apple input APIs, Mach handling and packaging; Windows ConPTY/Win32 behavior; native FreeBSD behavior. Swift was excluded.

**Other verification limits:** existing binaries were used; no fresh full build, GUI captures, live portal/D-Bus integration, real-browser rendering, or native io_uring execution was performed. Platform findings above are source-verified.

No files were modified; the worktree remains clean.