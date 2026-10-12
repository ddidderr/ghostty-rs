# Verification V1 / V1b (core end-to-end + GTK runtime)

## E2E tests (`crates/ghostty-core/tests/`, 81 tests, all headless; real core Surface + termio + pty + headless EGL renderer, fake apprt)
- `common/mod.rs`: harness (FakeRt recording actions/clipboard, recorder child `cat`-like process, pump/wait/settle helpers).
- `e2e_bindings.rs` (23): default and configured binding actions, select-all/copy/paste, csi/esc/text, screen-file open, child env.
- `e2e_ime.rs` (4): preedit state/rendering, commit, composing key events.
- `e2e_keys.rs` (15): key events -> encoder -> pty bytes (legacy, kitty, modifiers, sequences).
- `e2e_mouse.rs` (19): selection, copy-on-select, multi-click, mouse reporting (x10/normal/sgr), wheel, alternate scroll.
- `e2e_term.rs` (20): resize/TIOCSWINSZ, focus, title, pwd, bell, OSC 52 read/write, notifications, progress, child exit.
- Gate: workspace build, `cargo test --workspace` (~480 tests), `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --all` all pass.

## Bugs found and fixed
- core `surface/mod.rs`: set_selection compared against the previous selection after replacing it (tracked-pin panic, e.g. select-all twice with copy-on-select).
- core `surface/mod.rs`: child env overlaid runtime env on the process env; now the runtime env wholesale (fallback only if empty).
- core `surface/keyboard.rs`: global/all `new_window` targeted the surface (parent inheritance); now `Target::App`.
- core `global.rs` + apprt-gtk `flatpak.rs`: runtime-specific resources-dir hook (host_path), wired from `src/main.rs`.
- core `surface/binding.rs`: failed screen-file open returned success; now an error (performable bindings fall through). CSI/ESC > 128 bytes rejected.
- core `surface/clipboard.rs`: OSC 52 base64 acceptance matches Zig (size check, ignore trailing-bit errors).
- core `global.rs`: `GHOSTTY_LOG` boolean aliases/empty value/invalid-field behaviour.
- apprt-gtk `class/application.rs` + `src/main.rs`: `init_locale` (ensureLocale) before GTK init and for CLI actions.
- renderer `headless.rs`, termio, apprt `runtime_api.rs`: fixes needed by the headless harness; apprt-gtk `debug_input.rs`: new input replay hook.
- tests: wheel e2e test raced under load (one discrete tick = 3 reports); fixed to wait for all 6.
- `dist/` (desktop file, metainfo, dbus/systemd units, icons) is legitimate install data; kept.

## Codex audit (review-core-codex.md), 9 items
1. selection-replacement panic: FIXED, test `select_all_twice_with_copy_on_select_does_not_panic`.
2. child env filtering: FIXED, test `child_environment_comes_from_the_runtime`.
3. new_window parent: FIXED for global/all (keyboard.rs). Surface-local `new_window` binding (binding.rs) still passes `Target::Surface` deliberately: Zig Surface.zig:5122 calls `app.newWindow` with the surface as parent there.
4. flatpak host path: FIXED (hook, no unit test: needs /.flatpak-info).
5. screen-file open failure: FIXED, test `write_screen_file_open_failure_is_an_error`.
6. ensureLocale: FIXED (unsafe setlocale/setenv in apprt-gtk, SAFETY comments); not unit-testable (process-global locale).
7. OSC 52 base64: FIXED, test `osc52_write_base64_acceptance_matches_zig`.
8. CSI/ESC length limit: FIXED, test `csi_binding_longer_than_the_zig_buffer_is_rejected`.
9. GHOSTTY_LOG parsing: FIXED, unit test `logging_parse`.

## GTK runtime check (real app, Wayland, `GHOSTTY_DEBUG_INPUT_SCRIPT` + screenshots)
- `echo hi` + Enter: output `hi` shown. PASS.
- ctrl+shift+t: new tab, typing works in it. PASS.
- ctrl+shift+o / ctrl+shift+e: right and down splits, each pane renders and quits cleanly. PASS.
- ctrl+shift+a, ctrl+shift+c, ctrl+shift+v: select all highlighted, copied text pasted back. PASS.
- ctrl+= x4: font size 12 -> 16, glyphs visibly larger. PASS.
- The "panic after the first screenshot" did not reproduce; the likely cause was the selection panic (item 1), now fixed. Observed instead: a screenshot taken <~1.6 s after opening a new tab/split can fail with "no frame has been presented yet" (new surface's first frame is slow in a debug build); use longer `wait`s.
- A `quit` with a child shell lacking shell integration (`sh`) shows the close-confirmation dialog, so the process stays alive (expected behaviour).

## Still open
- Not exercised in the GTK check: mouse, IME via real IM, clipboard confirmation dialogs.
- Runtime warnings unchanged: `unimplemented action=CellSize`, cgroup scope creation failing in this sandbox.
