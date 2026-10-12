# P3: KNOWN_DIFFERENCES rows 5-10

* 5/6 `ghostty-cli/src/list_themes/preview.rs` `TextInput` is now a port of
  `vaxis.widgets.TextInput`: byte cursor, grapheme-wise Backspace/Delete/Left/Right
  (`unicode-segmentation`), ASCII-space words for Alt+B/F (and Alt+Left/Right),
  Ctrl+W / Alt+Backspace, Alt+D, Ctrl+A/E/K/U/B/F/D. Modifiers must match exactly
  (vaxis `Key.matches`), e.g. Ctrl+Left is not Left.
* 7 `rank` lowers only ASCII and compares bytes (zf `std.ascii.toLower`).
* 8 `initial_scheme`/`COLORFGBG` removed: light until a color-scheme report arrives.
* 9 `Preview::new`, `update_filtered`, `update`, `handle_terminal_event` return
  `Result`; `load_file` errors propagate and end the preview (exit code 1, as Zig).
* 10 `ghostty-termio/src/exec.rs` `getpgid`: no 5 s cap, waits like Zig. Deviation
  (DEVIANT_FIXES.md #13): Zig spins forever if the child died before `setsid`
  and is an unreaped zombie (pgid stays ours); Rust stops when `waitid(WNOWAIT)`
  reports the child exited.

Tests: unit tests in `preview.rs` and `exec.rs`; PTY tests in
`tests/list_themes_pty.rs` (Rust binary vs `/usr/bin/ghostty`, oracle part skipped
when absent; uses `setsid --ctty`, needs util-linux).

Observed, not fixed (outside rows 5-10): a ZWJ emoji cluster (woman+ZWJ+rocket) in
the search box is drawn by Rust without the second emoji; Zig draws it.
