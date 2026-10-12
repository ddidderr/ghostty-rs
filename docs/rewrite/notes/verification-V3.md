# Verification V3 (real GTK app, real programs, mouse input)

## Replay hook extensions (`crates/ghostty-apprt-gtk/src/debug_input.rs`, `class/surface.rs`)
New steps: `mouse-move x y [mods]`, `mouse-press|mouse-release|click <btn> [mods]`, `drag x1 y1 x2 y2 [mods]`,
`scroll dx dy`, `resize w h`, `clipboard-set`, `clipboard-dump` (also logs context-menu visibility),
`windows-dump` (toplevels + widget tree, shows AdwDialogs), `window-screenshot` (GTK render of the whole
window incl. headerbar/dialogs; the GL area content may differ from `screenshot`).
The GTK handlers `mouse_down/mouse_up/mouse_motion/scroll_vertical` were split into `*_impl` methods
(taking button/modifier state instead of a `GdkEvent`) so the replay runs the exact same code as the
gesture controllers. The replay passes `assume_focus=true` for presses: the replayed window is not
compositor-focused, and the "first click only transfers focus" suppression would otherwise eat every click.
Parser unit test `parse_mouse_steps` added.

## Scenarios (GTK, Wayland, release profile)
| Scenario | Result |
|---|---|
| nvim (-e nvim file.rs): syntax colors, ligatures, status line, `:q` | PASS. Insert mode bar cursor OK. |
| nvim from shell, `:q`: alternate screen restore | PASS (prompt + prior output intact). |
| htop: meters, colors, box/bars, F10 | PASS. Mouse click on MEM% header sorts (mouse reporting; clicking twice toggles asc/desc). F10 quits. |
| less on 300 lines: wheel in alt screen, `q` | PASS. 5 wheel ticks -> 15 lines (3 arrow keys per tick, DECSET 1007), `q` restores. |
| tmux: start, ctrl+b %, status bar, exit | PASS. |
| shell drag selection | PASS (highlight; text lands in PRIMARY via copy-on-select). |
| double-click word / triple-click line | PASS ("beta"; whole line). |
| ctrl+shift+c | PASS (CLIPBOARD gets the selection). |
| right-click | PASS: context popover becomes visible (`context menu visible = true`). |
| wheel scrollback (10 ticks = 30 lines) | PASS. |
| long wrapped line + window resize (shrink, grow) | PASS: reflow correct both ways, viewport preserved. |
| unicode: CJK, emoji, ZWJ family, combining, fullwidth, powerline/nerd glyphs, box drawing | PASS: widths/alignment correct. |
| kitty graphics (PNG, a=T, c=8,r=4) | PASS: image rendered at the right cell size, text flows after it. |
| multi-line paste into `cat` (no bracketed paste) | PASS: core rejects as unsafe, `GhosttyClipboardConfirmationDialog` is created and mapped (widget tree dump). The dialog isn't visible in the render-texture screenshot, only verified structurally. |
| Raw mouse bytes (SGR 1006, button 1/3, wheel) | PASS: `ESC[<0;10;5M/m`, `ESC[<2;..`, `ESC[<64;..` x3 per tick. |
| Kitty keyboard protocol (flags 1/31) from `key` steps | PASS (CSI u with text/release events). |

## Bugs
No product bugs found in this pass. Observations:
- Release profile keeps `debug-assertions = true` (workspace Cargo.toml), so the "debug build" banner shows; use `--profile production` to get the real release look.
- `screenshot` returns the renderer's last presented frame; right after input it can be one frame stale. Use >= 1 s waits.
- Replay limits: `key <printable>` bypasses the IM; `text` commits bypass key events (plain bytes even under kitty flags "report all keys"); window is not compositor-focused so cursor is hollow and clicks need `assume_focus`.
- nvim leaves swap files when the replay is `quit` mid-edit; a later nvim of the same file prompts (E325) and swallows keystrokes. This looked like a bug at first but is nvim behaviour (use `nvim -n`).

## Remaining
- Confirm/deny of the paste dialog by interacting with it not exercised; IME via real input method not exercised.
- Horizontal scroll (tab switching) not replayed.
