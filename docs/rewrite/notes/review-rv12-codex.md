# Codex final verification (RV12, 2026-10-12)

**All three RV11 reproductions are fixed at `4bbe0d7`. Exact parity remains incomplete: 2 newly verified diagnostic gap groups and 1 known visual gap—0 P1, 0 P2, 3 P3.** No files were modified; the worktree remains clean.

| RV11 finding | Verification |
|---|---|
| Duplicate benchmark enum arguments | Resolved. Earlier invalid values are rejected, including before empty resets. Sixteen differential checks matched Zig. |
| Interior-NUL theme paths | Resolved. Absolute and named paths load their prefix, producing `#262427` without diagnostics in both implementations. |
| Filesystem diagnostic descriptions | Original Linux cases resolved, including absolute and relative `NotDir` paths. The replacement helper has the portability defect below. |

Concrete remaining gaps:

1. **P3 — Platform error codes interpreted as Linux errno values.** [zig_error.rs:37](../../../crates/ghostty-os/src/zig_error.rs:37) unconditionally applies the Linux table. Windows access-denied code `5` becomes `error.Unexpected`; Darwin `ELOOP=62` and `ENAMETOOLONG=63` also become `Unexpected`. Verified statically against platform definitions and call sites; native execution remains untested.

2. **P3 — Incorrect diagnostics for dot theme names.** [theme.rs:187](../../../crates/ghostty-config/src/theme.rs:187) uses Rust’s `file_name()`. Live differential checks show `theme=.` and `theme=..` report “cannot include path separators,” whereas Zig reports “it is a directory.” Both reject the theme.

3. **P3 — Known sprite stroke rasterization differences.** [canvas.rs:489](../../../crates/ghostty-font/src/sprite/canvas.rs:489) retains tiny-skia stroke geometry. Recomputing the saved VP captures confirmed **196 differing pixels per page scenario**, maximum channel delta **86**; tabs/splits and chrome are identical. This is the documented curly-underline/diagonal-glyph difference.

The fresh sweep covered the **461-file inventory: 428 non-build files and 33 build files**, with counterpart, public-function/export, stub and targeted behavioral checks across all subsystems. No wholly absent feature was established. This is breadth coverage, not proof of exhaustive behavioral equivalence.

Validation produced:

- **2,759 existing-binary tests passed**, four ignored; four encountered read-only filesystem restrictions.
- **6,300 configuration diagnostic comparisons**, covering all 202 fields and compatibility handlers: only the dot-theme differences above.
- **2,700 configuration value/diagnostic comparisons**: only expected debug-default differences.
- Matching actions, default keybindings, theme/font listings, zsh/fish completions; WASM smoke test passed.
- The documented **390,000-case terminal campaign** remains supporting evidence; it was not rerun. Genuine Zig memory bugs/UB were excluded.

The benign `+list-colors` difference also persists: Rust flushes all **15,434 bytes**, while Zig stops at **12,297**. It is excluded from defect counts.

**Inherently unverifiable natively here:** Apple Metal/IOSurface/CoreText, AppKit/UIKit/Carbon integration, Mach crash handling and Apple framework packaging; Windows ConPTY/CreateProcessW, native filesystem/environment behavior and exception handling.

Fresh Linux GPU/compositor, portal/D-Bus and interactive GTK runs were not performed. Those are runtime coverage limitations, not inherently untestable Linux functionality.