# Codex final verification (RV14, 2026-10-12, audited f9094c8)

**Parity remains incomplete at `f9094c8`: 0 P1, 1 P2, 5 P3 verified gaps.** No files were modified; the worktree remains clean.

All four original RV13 reproductions are addressed, but two fixes retain broader differences:

| RV13 finding | Verification |
|---|---|
| Default update channel | Fixed. Both binaries default to `tip`; explicit-value tests pass. |
| Ineffective `async-backend` | Selection now reaches the I/O reactor. Epoll works; availability differs in the case below. |
| Windows sharing-violation diagnostic | Fixed: `AccessDenied`, matching Zig **0.15.2**. Source and platform-independent tests checked. |
| Windows `1:theme` basename | Fixed, but the replacement introduces newer-Zig semantics described below. |

Concrete remaining gaps:

1. **P2 — `+show-face` accepts oversized codepoints and can panic.** [show_face.rs:41](../../../crates/ghostty-cli/src/show_face.rs:41) uses `u32` instead of Zig’s `u21`. With `--config-default-files=false --cp=4294967295`, Rust panics in the Unicode lookup and exits **101**; Ghostty reports an invalid value and exits **1**. Differentially verified.

2. **P3 — Invalid UTF-8 in `+show-face --string` changes validation behavior.** [show_face.rs:44](../../../crates/ghostty-cli/src/show_face.rs:44) rejects it during option parsing. With `--cp=65` and a raw `0xFF` string, Ghostty prints the codepoint result, then `error.InvalidUtf8`; Rust prints only an option diagnostic. Differentially verified.

3. **P3 — `+show-face` diagnostics appear in a different order.** [show_face.rs:85](../../../crates/ghostty-cli/src/show_face.rs:85) writes both streams directly to stdout. For `--cp=65 --font-size=bad`, Rust prints the diagnostic first; Zig’s separate buffers put the lookup first. Differentially verified.

4. **P3 — Windows basename fix follows the wrong Zig version.** [theme.rs:249](../../../crates/ghostty-config/src/theme.rs:249) treats multibyte characters followed by `:` as drive prefixes. Rust returns `theme` for `λ:theme`; Zig **0.15.2** returns `λ:theme`, recognizing a colon only at byte index 1. This changes theme-name rejection, including names supplied through `light:λ:theme,dark:λ:theme`. Statically verified.

5. **P3 — Windows rooted theme paths are rejected.** [theme.rs:288](../../../crates/ghostty-config/src/theme.rs:288) uses Rust’s `Path::is_absolute`, which requires a Windows prefix. Zig accepts `\theme` and `/theme` as absolute; Rust instead rejects their separators. Verified against both standard-library implementations.

6. **P3 — io_uring availability is narrower than upstream.** [xev.rs:120](../../../crates/ghostty-os/src/xev.rs:120) additionally requires `EXT_ARG` and successful opcode probing. Upstream libxev accepts a successful Zig `IoUring.init(256, 0)`. A supported ring without `EXT_ARG` therefore selects io_uring upstream but falls back to epoll in Rust. Statically verified; that environment was not reproduced here.

The fresh breadth sweep screened **461 Zig files: 428 non-build and 33 build files**, including counterparts, public functions, config consumers, incomplete paths, and build outputs. No wholly missing counterpart was established; this does not prove exhaustive behavioral equivalence.

Fresh validation:

- **2,799 distinct tests passed**, four ignored; three fixture tests were blocked by the read-only filesystem.
- **1,000 terminal differential cases matched**, without aborts. The documented 390k campaign was not rerun.
- **567 broad CLI/config comparisons:** 555 exact matches; three build-mode differences, two known `list-colors` differences, seven diagnostic-order differences.
- **594 additional config probes** matched after normalizing the identified output ordering.
- All **138 C/WASM declarations** have source counterparts; WASM smoke test passed.
- Saved captures recomputed to **zero differing pixels** for all three visual scenarios and all **36 sprite sheets**. No fresh captures were made.

The previously documented benign `+list-colors` difference remains separate: Rust flushes all **15,434 bytes**; Zig omits its final flush and emits **12,297**. Genuine Zig memory bugs/UB were excluded.

**Inherently unavailable natively here:** Apple Metal/IOSurface/CoreText, Carbon/AppKit/UIKit, Mach exception handling, and Apple packaging; Windows ConPTY, process creation, native filesystem behavior, and exception handling. Swift remains excluded.

**Other coverage limits:** io_uring was denied in this sandbox; 20 integration cases skipped. Fresh compositor/GPU, portal/D-Bus, interactive GTK, builds, and cross-compilation were not run. Existing binaries were reused after their build-tree Rust/Cargo sources were compared with HEAD.