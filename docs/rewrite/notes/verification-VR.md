# Verification VR (IME, dialogs, quick terminal, flatpak host command, hscroll)

## Replay hook additions (`crates/ghostty-apprt-gtk/src/debug_input.rs`, `class/surface.rs`)
- IME: `ime-preedit <text>`, `ime-preedit-end`, `ime-commit <text>`, `ime-state`,
  `ime-key <preedit|commit|commit-simple|handled-commit|pass> <key> [text]`.
  The steps call the surface's real template callbacks (`im_preedit_start/changed/end`, `im_commit`, now `pub(super)`
  and taking `&gtk::IMContext`) with a `mock_im::MockIm` (a real `GtkIMContext` subclass whose `preedit_string()`
  returns text + underline attrs + cursor) as emitter. `ime-key` runs the real `in_keyevent`/`im_composing`
  bookkeeping: the IM half of `Surface::key_event` was extracted into `Surface::im_filter(filter)` (behaviour unchanged,
  `key_event` passes `filter_keypress`). Orders covered: ibus (commit then preedit-end), simple (preedit-end then commit),
  dead key (handled while composing -> not encoded), plain translation commit (handled, encoded with the key), pass.
  Zig has no `retrieve-surrounding`/`delete-surrounding` handlers (checked surface.zig), so there is nothing to port.
- Dialogs: `dialog-info`, `dialog-remember`, `dialog-click <label>` (clicks the real GtkButton in the AdwAlertDialog,
  so the AdwAlertDialog response -> `confirm`/`deny` signals run). `animations off|on`.
- Quick terminal: `windows-dump` now logs per GhosttyWindow: quick_terminal, visible, mapped, active, size and the
  layer-shell state (layer, keyboard mode, namespace, anchors, exclusive zone); `quick-focus-out` emits
  `notify::is-active` on the quick window (the window is never compositor-focused here) to run the autohide handler.
- `hscroll <dx>` (the handler body was moved to `Surface::scroll_horizontal_impl`; the controller keeps the unit
  check), `tabs-dump`. Replay target window: if GTK's active window is hidden (autohidden quick terminal) the first
  visible Ghostty window is used.

## Results
| Area | Result |
|---|---|
| IME preedit | PASS. `ime-preedit かn` renders the underlined preedit at the cursor (screenshot); core preedit cleared on commit/end. |
| IME commit / dead keys | PASS. pty receives `こんáxyé` (`cat -v` also shows the exact UTF-8 bytes); preedit text never reaches the pty; composing/consumed state sequence matches Zig's logic. |
| Unsafe paste dialog | PASS. Dialog (heading, body, Deny/Allow, contents) verified structurally; Deny writes nothing, Allow writes the text once. |
| OSC 52 write=ask | PASS. Deny keeps the clipboard, Allow writes it, Allow+remember (switch) makes later writes silent (`clipboard_write` -> allow). |
| OSC 52 read=ask | PASS. Allow: pty gets `ESC]52;c;c2VjcmV0...`; Deny: nothing sent. |
| Quick terminal | PASS after the link-order fix below: layer surface (visible in `niri msg layers`: Top/Overlay, namespace `ghostty-quick-terminal`, keyboard interactivity), anchors/size for top/bottom/left/right/center match Zig's `quick-terminal-size` and anchor logic, toggle hides and re-shows, autohide on focus loss (default false on Linux, as Zig) hides, `autohide=false` stays. |
| hscroll tabs | PASS (threshold 120, direction, no wrap at the ends, 500 ms accumulator reset). |
| Flatpak host command | PASS against a mock service (see below). |

Not verifiable here: real keyboard focus on the layer surface (`quick-terminal-keyboard-interactivity=exclusive` is
applied, `niri msg layers` shows it, but `GtkWindow::is_active` stays false because the replayed windows are never
compositor-focused; the autohide path was driven with `quick-focus-out`); KDE slide animation (needs the KDE protocol,
niri has none); rendering of AdwDialog in screenshots (`window-screenshot` does not capture the dialog layer; use
`dialog-info`); a real input method (ibus/fcitx are not available).

## Bug found and fixed: gtk4-layer-shell load order
`libgtk4-layer-shell` must be loaded before `libwayland-client`. The Rust binary's DT_NEEDED order depends on how
rustc orders native libs, which flipped between builds (adding an unrelated `gtk4_layer_shell::` path in a file was
enough); with the wrong order `is_supported()` is false and the quick terminal is silently disabled ("compositor does
not support wlr-layer-shell"). Fix: `wayland-sys` with features `client,dlopen` is a direct dependency of apprt-gtk,
so `libwayland-client` is no longer a direct DT_NEEDED of the binary (it is only pulled in transitively by GTK, which
is loaded after all direct libraries, same effect as Zig's explicit link order). Check: `readelf -d target/*/ghostty |
grep NEEDED` has no `libwayland-client`. Regression: `quick_terminal_layer_surface_toggle_and_autohide`.

## Flatpak host command
- Seam: `ghostty_os::flatpak::set_flatpak_info_path(path)` (OnceLock, production code, unused by the app) makes
  `is_flatpak()` check that path instead of `/.flatpak-info`.
- Mock service: `crates/ghostty-os/tests/common/flatpak_mock.rs` (zbus, claims `org.freedesktop.Flatpak`, implements
  `Development.HostCommand`/`HostCommandSignal`, really runs the command locally with the received stdio fds in its
  own process group, emits `HostCommandExited` with the raw wait status, records spawns and signals). Tests re-exec
  themselves under `dbus-run-session` (skipped if it is missing).
- `crates/ghostty-os/tests/flatpak_mock.rs`: spawn returns host pid, cwd/env/fds forwarded, stdout via passed fd,
  exit status propagated (7), `wait` state reset, `wait_callback`, host spawn failure -> `FlatpakSpawnFail`,
  `signal_handle().signal(1, true)` after the command moved into the watcher -> recorded `(pid, 1, true)` and the
  host process exits.
- `crates/ghostty-termio/tests/flatpak_exec.rs` (dev-deps: zbus, rustix): termio `Exec` flatpak branch end to end
  (output on the pty, exit code 5 propagated as ChildExited, input echoed through the pty, `stop()` sends SIGHUP to
  the host process group and the host process dies). One test per binary: the override is process-global.

## Regression tests
- `tests/gtk_replay.rs` (root package, runs the real binary on the current Wayland/X11 session; skips without a
  display or with `GHOSTTY_SKIP_GTK_REPLAY`): IME flow, paste deny/confirm, OSC 52 write/read ask (+remember),
  quick terminal layer surface/toggle/autohide/no-autohide, horizontal scroll tab switching. ~45 s total, serialized
  (they share the session clipboard). The harness sets `RUST_LOG=debug` and
  `GHOSTTY_LOG=stderr` for the app so replay assertions and Wayland layer-shell
  detection work independently of the default log level and inherited logging settings.
- Parser unit tests for the new steps in `debug_input.rs`.

## Notes
- `screenshot` can be one frame stale right after input (wait >= 1 s); `text` after `key escape` in bash is
  meta-prefixed by readline (not a bug).
- Child programs reading the tty from `sh -c` need `timeout --foreground` (job-control SIGTTIN otherwise).
- `quick-terminal-autohide` defaults to false on Linux (as Zig), so the quick terminal does not hide on focus loss by default.
