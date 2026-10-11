# Cross-crate integration TODOs

Collected from agent reports; resolve during integration phases.

- [ ] vt `osc/parsers/semantic_prompt.rs`: local printf-q / percent decoders → share with `ghostty-os::string_encoding` (vt can't depend on os; maybe move into vt and have os re-export).
- [ ] os `locale::ensure_locale` no-op → call setlocale from apprt-gtk (unsafe allowed) at startup, as Ghostty's global.zig does.
- [ ] os `passwd` Flatpak branch needs termio Pty.
- [ ] os `open::OpenUrlKind` → replace with core apprt action type.
- [x] terminal crate: re-add deps on unicode/datastruct once those land.
- [ ] input: key_encode::Options / paste::Options from terminal modes (Zig `fromTerminal`) — in terminal or core.
- [ ] input: `binding::set::EntryFormatter` must be implemented by config formatter.
- [ ] config: clipboard_codepoint_map::Entry and CursorStyle are local copies → unify with terminal/formatter (font types are unified: canonical in ghostty-config, re-exported by ghostty-font).
- [ ] config: url::REGEX needs onig (lookbehind) in renderer/core link matching.
- [ ] config: theme::open needs resources dir from ghostty_os.
- NOTE: Ghostty 1.3.1 is installed at /usr/bin/ghostty — use it as reference oracle (e.g. +show-config, +list-actions, terminfo).
- [ ] termio: Pty closes slave on drop (Zig didn't) — verify Exec semantics; cgroup/rlimit/D-Bus scope wiring; local option stand-ins → config types.
- [ ] cli: wire list-fonts/show-face (font discovery) and new-window (GTK IPC); call build_info::set from root/apprt.
- [ ] input: ActionKey::docs() has extra trailing blank line for 4 actions (cli works around locally).
- [ ] vt: x11_color lacks name listing with original case (cli re-parses rgb.txt).
- [ ] apprt: Scrollbar/RendererHealth stand-ins → terminal/renderer types; core implements AppMailbox<S>.
- [x] terminal (after T4/T5/K1): Codex round 3 on notes/ghostty-terminal-pagelist-requests.md items 1,2,3,4,5,7 + append_grapheme dirty + clone_partial_row_between; then remove Screen/Terminal workarounds. Items 6 and the two T3 test items are resolved by the parity pass.
- [ ] tmux viewer wiring into termio stream handler (see notes/ghostty-terminal-search-tmux.md); renderer sets search_viewport_dirty.
- [ ] apprt-gtk: stubs toggle_visibility/float_window/secure_input/close_all_windows; inspector no-op; cgroup wait loop duplicated with termio; GLArea framebuffer id must be queried by renderer; layer-shell lib load order.
- [ ] RendererState defined in termio::state; renderer R2 ports State.zig → unify (canonical in renderer, termio depends on renderer).
- [ ] termio: hook kitty graphics exec/replies (terminal now has it) and image_storage_limit; tmux viewer.
- [ ] termio: Flatpak stop path needs ghostty-os FlatpakHostCommand change; apprt-gtk create_scope; core restore_rlimit.
- [ ] renderer: generic::update_kitty → call image::State::kitty_update now that Screen has ImageStorage; apprt-gtk must set GL viewport before draw.
