# Known differences (left by the stopping criterion)

These are known minor differences between the Rust port and Ghostty 1.3.1
found in the final audit (RV18, `notes/review-rv18-codex.md`). They were
recorded, not fixed, because the stopping criterion in `PLAN.md` does not
require chasing P3 items. All are severity P3. Each was verified from source
and, where noted, by PTY reproduction.

| # | Area | Rust | Zig | Observable effect | Sev |
|---|------|------|-----|-------------------|-----|
| 5 | Theme search: word navigation | `ghostty-cli/src/list_themes/preview.rs:113` has no word-navigation editing | upstream list-themes search has word navigation | Enter `blue red`, Alt+B, `X`: Zig gives `blue Xred`, Rust `blue redX` | P3 |
| 6 | Theme search: grapheme backspace | `preview.rs:100` deletes one Unicode scalar | deletes one grapheme | `e` + U+0301 then Backspace: Zig removes both, Rust leaves `e` | P3 |
| 7 | Theme search: non-ASCII case | `preview.rs:150` Unicode-lowercases theme names but ASCII-lowercases the query | ASCII-only on both sides | Query `Ä` does not match theme `Ä` in Rust, matches in Zig | P3 |
| 8 | Theme preview: initial colors | `preview.rs:385` consults `COLORFGBG` | starts light until terminal color reports arrive | Without terminal replies, changing `COLORFGBG` changes Rust's colors only | P3 |
| 9 | Theme preview: load errors | `preview.rs:440` ignores `load_file` errors and keeps filtering with the retained config | the error propagates and the preview exits | A config load error is silently swallowed in Rust | P3 |
| 10 | Subprocess cleanup cap | `ghostty-termio/src/exec.rs:1348` waits at most about 5 s for the child's new process group | keeps waiting | If the child is delayed before `setsid` for longer, Rust returns without sending SIGHUP | P3 |

## Not verifiable on this machine

- Native Apple execution: Metal, IOSurface, CoreText, Apple input APIs, Mach handling and Apple packaging (compile-checked only, `just check-macos`).
- Windows ConPTY and Win32 behavior (compile-checked only, `just check-windows`).
- Native FreeBSD execution (cross `cargo check` only, if the target could be added).
- Live portal / D-Bus integration.
- Real-browser rendering (wasm is checked with Node only).
- Native io_uring execution; Swift code was excluded.
