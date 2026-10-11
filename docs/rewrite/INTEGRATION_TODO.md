# Cross-crate integration TODOs

Collected from agent reports; resolve during integration phases.

- [ ] vt `osc/parsers/semantic_prompt.rs`: local printf-q / percent decoders → share with `ghostty-os::string_encoding` (vt can't depend on os; maybe move into vt and have os re-export).
- [x] os `locale::ensure_locale` no-op → call setlocale from apprt-gtk (unsafe allowed) at startup, as Ghostty's global.zig does. (resolved: apprt-gtk Application calls setlocale at startup; the core/root binary do not need unsafe)
- [ ] os `passwd` Flatpak branch needs termio Pty.
- [x] os `open::OpenUrlKind` → replace with core apprt action type.
- [x] terminal crate: re-add deps on unicode/datastruct once those land.
- [x] input: key_encode::Options / paste::Options from terminal modes (Zig `fromTerminal`) — in terminal or core. (resolved: `ghostty_core::surface::keyboard::encode_options_from_terminal`, `clipboard::paste_options_from_terminal`)
- [ ] input: `binding::set::EntryFormatter` must be implemented by config formatter.
- [ ] config: clipboard_codepoint_map::Entry and CursorStyle are local copies → unify with terminal/formatter (font types are unified: canonical in ghostty-config, re-exported by ghostty-font).
- [ ] config: url::REGEX needs onig (lookbehind) in renderer/core link matching.
- [ ] config: theme::open needs resources dir from ghostty_os.
- NOTE: Ghostty 1.3.1 is installed at /usr/bin/ghostty — use it as reference oracle (e.g. +show-config, +list-actions, terminfo).
- [ ] termio: Pty closes slave on drop (Zig didn't) — verify Exec semantics; cgroup/rlimit/D-Bus scope wiring; local option stand-ins → config types.
- [x] cli: wire list-fonts/show-face (font discovery) and new-window (`set_ipc_handler`, root must register it).
- [x] cli: call build_info::set from root (done in src/main.rs; build versions are the cargo feature levels 4.14/1.5).
- [ ] input: ActionKey::docs() has extra trailing blank line for 4 actions (cli works around locally).
- [ ] vt: x11_color lacks name listing with original case (cli re-parses rgb.txt).
- [x] apprt: Scrollbar/RendererHealth stand-ins → terminal/renderer types; core implements AppMailbox<S>. (resolved: apprt re-exports the real types; core implements AppMailbox<SurfaceId>)
- [x] terminal (after T4/T5/K1): Codex round 3 on notes/ghostty-terminal-pagelist-requests.md items 1,2,3,4,5,7 + append_grapheme dirty + clone_partial_row_between; then remove Screen/Terminal workarounds. Items 6 and the two T3 test items are resolved by the parity pass.
- [ ] tmux viewer wiring into termio stream handler (see notes/ghostty-terminal-search-tmux.md); renderer sets search_viewport_dirty.
- [ ] apprt-gtk: stubs toggle_visibility/float_window/secure_input/close_all_windows; inspector no-op; cgroup wait loop duplicated with termio; GLArea framebuffer id must be queried by renderer; layer-shell lib load order.
- [x] RendererState defined in termio::state; renderer R2 ports State.zig → unify (canonical in renderer, termio depends on renderer). (resolved: termio uses `ghostty_renderer::state::{State, SharedState}`)
- [x] termio: hook kitty graphics exec/replies (terminal now has it) and image_storage_limit; tmux viewer. (resolved: APC exec already in stream handler; image-storage-limit applied at init/config change, clear-screen deletes images; tmux viewer still open)
- [x] termio: Flatpak stop path needs ghostty-os FlatpakHostCommand change; apprt-gtk create_scope; core restore_rlimit. (partially: core supplies restore_rlimit; apprt-gtk supplies create_scope via RtSurface::create_scope_fn; Flatpak stop unchanged)
- [x] renderer: generic::update_kitty → call image::State::kitty_update now that Screen has ImageStorage; apprt-gtk must set GL viewport before draw. (resolved; apprt-gtk resolves GL via epoxy_* pointers; the renderer queries GL_VIEWPORT)
