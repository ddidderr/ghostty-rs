# Codex final verification (RV18, 2026-10-12) — final audit round

At **HEAD `2040677`**, **all 6 RV17 findings are resolved**, with no verified regression attributable to Z20. **Full parity is not established: 13 remaining findings—2 P2 and 11 P3.** These comprise one incorrect registered claim and twelve unregistered differences, including corrections that should be preserved and documented.

No files were modified; the worktree remains clean.

RV17 verification:

| Finding | Result |
|---|---|
| FreeType-only backend on Apple | Resolved: backend selection honors the feature and disables CoreText/discovery appropriately. Native Apple execution remains unavailable. |
| CLI feature-unification test | Resolved: assertions follow the selected font backend. Targeted tests pass. |
| Inspector font | Resolved: 32 px with FreeType on Linux. Remaining platform rasterizer differences are registered. |
| Inspector DCS descriptions | Resolved against **Zig 0.15.2** formatting, including slice syntax and type prefix. |
| Inspector demo in release builds | Resolved: gated by the build profile instead of assertions. |
| Metal allocation correction | Resolved: preserved, tested through portable allocation arithmetic, and registered as entry 34. |

The remaining verified findings follow. “Source” means the differing branch and consequence were checked in both implementations, without an end-to-end runtime reproduction.

| # | Severity | Finding and evidence |
|---|---|---|
| 1 | **P2** | **Deviation entry 10 incorrectly claims Rust handles the listed terminal inputs without panicking.** A mode-2027/BOM sequence reaches `debug_assert!(!b)` in [terminal.rs:824](../../../crates/ghostty-terminal/src/terminal.rs:824). Rust debug exits 101; assertion-enabled Zig aborts. Both production variants succeed. The repository’s `release` profile also enables this assertion. This is an unresolved shared upstream defect, not a defined-result mismatch. |
| 2 | **P2** | **FreeBSD compilation selects an unavailable libc function.** [command.rs:549](../../../crates/ghostty-termio/src/command.rs:549) selects `libc::__errno_location()` for every non-Apple Unix target; the pinned libc exposes `__error()` on FreeBSD. Source-verified build blocker; no cross-build performed. |
| 3 | P3 | **FreeBSD resource discovery uses Linux conventions.** [resourcesdir.rs:52](../../../crates/ghostty-os/src/resourcesdir.rs:52) searches `terminfo` under `share`; Zig uses `site-terminfo` and `local/share`. A standard upstream FreeBSD installation is therefore missed without an explicit resource override. Source. |
| 4 | P3 | **Release resource precedence still treats assertions as Debug mode.** [resourcesdir.rs:74](../../../crates/ghostty-os/src/resourcesdir.rs:74) makes an installed resource tree override `GHOSTTY_RESOURCES_DIR` in this repository’s assertion-enabled release profile. Zig ReleaseSafe honors the environment first. Source. |
| 5 | P3 | **Theme-search word editing is incomplete.** [preview.rs:113](../../../crates/ghostty-cli/src/list_themes/preview.rs:113) lacks upstream word-navigation behavior. PTY reproduction: enter `blue red`, press Alt+B, then `X`; Zig produces `blue Xred`, Rust `blue redX`. |
| 6 | P3 | **Theme-search editing splits graphemes.** [preview.rs:100](../../../crates/ghostty-cli/src/list_themes/preview.rs:100) deletes one Unicode scalar. PTY reproduction: enter `e` plus U+0301, then Backspace; Zig removes the grapheme, Rust leaves `e`. |
| 7 | P3 | **Theme-search case handling differs for non-ASCII names.** [preview.rs:150](../../../crates/ghostty-cli/src/list_themes/preview.rs:150) Unicode-lowercases names but ASCII-lowercases queries. An exact query `Ä` fails against theme name `Ä`; upstream’s ASCII-only handling matches it. Source. |
| 8 | P3 | **Theme-preview initial colors differ.** [preview.rs:385](../../../crates/ghostty-cli/src/list_themes/preview.rs:385) consults `COLORFGBG`; upstream initializes light until receiving terminal color reports. PTY captures without replies confirmed changing `COLORFGBG` changes Rust’s colors while Zig’s output remains identical. |
| 9 | P3 | **Theme-preview load errors are swallowed.** [preview.rs:440](../../../crates/ghostty-cli/src/list_themes/preview.rs:440) ignores `load_file` errors and continues filtering with retained configuration. Zig propagates the error and exits the preview. This is separate from registered diagnostic-buffer flushing. Source. |
| 10 | P3 | **Subprocess cleanup silently gives up after approximately five seconds.** [exec.rs:1348](../../../crates/ghostty-termio/src/exec.rs:1348) caps the wait for the child’s new process group. If the child remains delayed before `setsid`, cleanup returns without sending SIGHUP; Zig keeps waiting. Source. |
| 11 | P3 | **The development resource fallback is unregistered.** [resourcesdir.rs:140](../../../crates/ghostty-os/src/resourcesdir.rs:140) discovers workspace resources where upstream returns none. This is a documentation/intentional-deviation finding. |
| 12 | P3 | **The PTY hangup output-loss correction is unregistered.** [exec.rs:742](../../../crates/ghostty-termio/src/exec.rs:742) drains remaining output after HUP; Zig exits immediately. A Linux PTY check confirmed `POLLIN | POLLHUP` can accompany unread final output. Preserve this correction and register it. |
| 13 | P3 | **The browser Canvas atlas correction is unregistered.** [web_canvas.rs:177](../../../crates/ghostty-font/src/face/web_canvas.rs:177) implements BGRA conversion, correcting upstream’s stale `.rgba` branch against an enum containing `.bgra`. Source-verified correction; preserve and register it. No browser-equivalence claim is made. |

The minimal reproduction for finding 1 is:

```python
b"\x1b[?2027la\xef\xbb\xbf\x1b[?2027h\xe4\xb8\xad"
```

Feed it to the differential drivers at geometry `20 × 8`, scrollback `1000`.

Verification coverage:

- **461/461 Zig files** inventoried and interface-screened across every source category, including **2,801 public declarations**, followed by targeted implementation comparisons. This establishes breadth of review, not exhaustive behavioral equivalence.
- **535 fresh CLI/config comparisons:** 524 identical; 11 differences covered by registered build-metadata and output-flushing deviations.
- **1,200 fresh terminal cases:** 1,187 identical successful results and 13 cases where both assertion-enabled implementations failed; **zero defined-result mismatches**. The documented 390k campaign was not rerun. Its exclusion of Zig-aborting cases explains why “zero divergences” does not establish the register’s no-panic claim.
- **2,451 successful selected unit tests**, plus targeted backend checks. Fixture-writing tests could not run under the read-only restriction.
- Existing **three visual scenarios and 36 sprite sheets** remain pixel-identical. No new GUI captures were taken.
- **14 GLSL files and 11 VT C headers** were byte-identical to upstream.

**Inherently unavailable for native verification on this Linux machine:** Apple Metal/IOSurface/CoreText, Apple input APIs, Mach handling and Apple packaging; Windows ConPTY/Win32 behavior; native FreeBSD execution. Swift was excluded as requested.

**Other verification limits:** existing build artifacts were used; no fresh build, live portal/D-Bus integration, real-browser rendering, or native io_uring execution was performed. These are coverage limits, not additional verified gaps.