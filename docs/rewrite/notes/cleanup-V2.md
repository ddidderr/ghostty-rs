# Cleanup V2

## Resources (self-contained)
- `resources/` mirrors Ghostty's `share/ghostty` plus its sibling `share/terminfo`:
  `shell-integration/`, `themes/` (463 files copied from the installed Ghostty 1.3.1,
  `LICENSE-iTerm2-Color-Schemes` = MIT attribution, written from the upstream license;
  no upstream copy existed on this machine, check the copyright line if it matters) and
  `terminfo/` (compiled `xterm-ghostty`, `g/ghostty`).
- `scripts/gen-terminfo.sh` (`just terminfo`): `cargo run -p ghostty-termio --example terminfo`
  prints the source from `ghostty_termio::terminfo::ghostty`, `tic -x -o resources/terminfo`, then
  compares with the installed entry via `infocmp -x` (identical).
- `ghostty_os::resourcesdir`: Ghostty's logic unchanged (exe-relative `share/terminfo/...` sentinel,
  `GHOSTTY_RESOURCES_DIR` first in release / fallback in debug). New last-resort (debug only): if the
  exe lives inside the workspace this crate was compiled in (`env!("CARGO_MANIFEST_DIR")/../..`) and
  `<workspace>/resources` has the same sentinel (`terminfo/x/xterm-ghostty`), use it. `cargo run`
  now works without the env var.
- Child `TERMINFO`: Zig sets `<resources>/../terminfo`. Rust prefers `<resources>/terminfo` when
  it holds the sentinel (dev layout), else the Zig sibling path.
- `config::theme::open` already took `ghostty_os::resources_dir()`; the dev fallback makes it work in dev.
- `just install [PREFIX]` (default /usr/local, honours DESTDIR): bin, share/ghostty/{shell-integration,themes},
  share/terminfo, desktop file, D-Bus + systemd units, metainfo, kio/nautilus extensions, icons
  (templates vendored in `dist/linux`, `dist/icons`, copied from ZIG). Not covered: shell completions,
  vim/bat syntax (no `+fish`/`+vim-syntax` generators in the Rust cli). Install tested with DESTDIR on a fake binary.

## Integration items
- vt `string_encoding` is canonical; `ghostty-os` now depends on `ghostty-vt` and re-exports it
  (semantic_prompt's `DecodeError` = `string_encoding::Error`).
- `binding::set::EntryFormatter` was already implemented by an adapter in `ghostty-config/src/keybinds.rs`.
  `+list-keybinds` (plain/--default/--docs) and `+list-actions [--docs]` are byte-identical to /usr/bin/ghostty.
  `+show-config --default [--docs]` differs only in `gtk-opengl-debug` (debug build, same as Zig).
- config now depends on `ghostty-terminal` (terminal dev-depends on config; cargo allows it). `CursorStyle` is
  `ghostty_terminal::cursor::Style` (ConfigField impl in config/types_a.rs); termio's `cursor_style_from_config`
  was removed. `clipboard_codepoint_map::{Entry, Replacement}` re-export terminal formatter types; core no longer converts.
- `ActionKey::docs()`: four docs had an extra trailing blank line (data fixed); cli `generate_plaintext` now calls
  `helpgen_actions::generate_string`.
- `x11_color::names()` (original case, deduped, file order); cli depends on ghostty-vt for it.
  Note: /usr/bin/ghostty `+list-colors` output is truncated at 12297 bytes when redirected (oracle bug); the prefix is identical.
- Pty/slave: Zig closes the slave in exactly two places (parent after successful spawn, error path) and `Pty.deinit`
  only runs after; Rust `Exec` calls `close_slave()` after success and drops the Pty on error, so lifetimes match. Documented in pty.rs.
- Cgroup wait loop: single `ghostty_os::cgroup::{scope_matches, wait_for_scope}` used by termio `Command` hooks and
  apprt-gtk `pre_exec`/`post_fork`.
- apprt-gtk stubs: in Zig 1.3.1 `application.zig` lists secure_input, close_all_windows, float_window, toggle_visibility
  under "Unimplemented" (log warn, return false) and the command palette hides them; the Rust stubs are therefore faithful and
  left as is. `toggle_visibility` on Window exists only for the quick terminal (as Zig). secure_input is macOS-only.
- tmux: `HandlerState.tmux_viewer: Option<Box<Viewer>>`; DCS Enter creates, Exit drops, other notifications go to
  `viewer.next`, `Command` -> `write_req`, `Exit` ignored, `Windows` TODO (same TODO in Zig). Test added. Renderer already sets `search_viewport_dirty`.
- passwd Flatpak: implemented in `ghostty-os` (termio depends on os, so no termio Pty) with rustix openpt + `FlatpakHostCommand`
  running `/bin/sh -l -c "getent passwd <name>"`; parse of shell/home is unit tested, the host call itself is untestable here.
- `+version`: `crates/ghostty-cli/build.rs` captures `rustc --version` (number only) into `GHOSTTY_RUSTC_VERSION`.

## Gate
fmt, clippy `-D warnings` clean for the workspace except `ghostty-bench` (another agent's new crate, 3 lints) and
`ghostty-core` integration tests under construction by V1 (lib/bin clean; fixed one `is_multiple_of` lint in core clipboard.rs).
Tests: all crates except core integration tests and bench pass (`ghostty-core --lib` 20). One run saw
`ghostty-termio/tests/exec_integration large_write_with_backpressure_is_delivered_in_order` fail (10s timeout) and pass on rerun: flaky/V1 area.
