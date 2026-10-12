# Codex final verification (RV13, 2026-10-12)

**At `f68b11d`: the three original RV12 reproductions are resolved, but full parity still has four verified P3 gaps.** Counts: **0 P1, 0 P2, 4 P3**. No files were modified; the worktree remains clean.

| RV12 finding | Result |
|---|---|
| Platform errno numbering | Fixed: platform constants replace Linux-only numbers. A separate Windows error-mapping defect remains below. |
| `theme=.` / `theme=..` | Fixed. Basename handling and directory diagnostics now match; the existing `..` fixture matched the installed oracle. |
| Sprite rasterization | Fixed in the saved evidence: all **36 sprite sheets** and all **three VP scenarios** recompute to **0 differing pixels**. |

The fresh sweep covered the **461-file inventory: 428 non-build files and 33 build files**, checking counterparts, exports, incomplete paths, configuration consumers, and build outputs, with targeted behavioral checks. No wholly absent source counterpart was established. This is breadth coverage, not proof of exhaustive equivalence.

Concrete remaining gaps:

1. **P3 — Wrong default update channel.** [config.rs:4659](../../../crates/ghostty-config/src/config.rs:4659) hardcodes `Stable`. Both tested binaries report `channel: tip`, but `+show-config` prints `stable` for Rust and `tip` for Zig. Zig defaults this field from the build channel.

2. **P3 — `async-backend` is ineffective.** [application.rs:332](../../../crates/ghostty-apprt-gtk/src/class/application.rs:332) omits Zig’s backend-selection startup step. The parsed field has no runtime consumer; [thread.rs:234](../../../crates/ghostty-termio/src/thread.rs:234) uses polling regardless of `epoll` or `io_uring`. Verified statically.

3. **P3 — Windows file-open diagnostics use the wrong error mapping.** [zig_error.rs:101](../../../crates/ghostty-os/src/zig_error.rs:101) maps sharing violations to `FileBusy`; Zig’s Windows `OpenFile` maps them to `AccessDenied`. Theme loading uses this helper. Verified statically against Zig 0.15.2; native execution unavailable.

4. **P3 — Windows basename handling restricts drive prefixes incorrectly.** [theme.rs:162](../../../crates/ghostty-config/src/theme.rs:162) requires an alphabetic first byte. For `1:theme`, Rust returns the whole name while Zig returns `theme`, so Rust bypasses Zig’s rejection diagnostic. Verified statically; native execution unavailable.

Fresh validation produced:

- **2,770 tests passed**, four ignored; file-writing tests excluded.
- **2,000 terminal differential cases matched**, with no aborts.
- **617 CLI/config comparisons**: 606 exact matches; all 11 mismatches concern the update-channel default.
- All **138 C/WASM API declarations** have Rust counterparts; WASM smoke test passed.

Existing binaries were reused; no fresh build or visual capture was made. The documented **390,000-case campaign** remains supporting evidence, not a rerun. Genuine Zig memory bugs/UB were excluded.

**Inherently unavailable natively here:** Apple Metal/IOSurface/CoreText, Carbon/AppKit/UIKit, Mach crash handling and Apple packaging; Windows ConPTY/process creation, native filesystem/environment behavior and exception handling.

Fresh Linux compositor/GPU, portal/D-Bus, and interactive GTK checks were not rerun. Those are coverage limitations, not inherently unverifiable Linux functionality.