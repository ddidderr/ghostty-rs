# Cross-crate integration TODOs

Collected from agent reports; resolve during integration phases.

- [ ] vt `osc/parsers/semantic_prompt.rs`: local printf-q / percent decoders → share with `ghostty-os::string_encoding` (vt can't depend on os; maybe move into vt and have os re-export).
- [ ] os `locale::ensure_locale` no-op → call setlocale from apprt-gtk (unsafe allowed) at startup, as Ghostty's global.zig does.
- [ ] os `passwd` Flatpak branch needs termio Pty.
- [ ] os `open::OpenUrlKind` → replace with core apprt action type.
- [ ] terminal crate: re-add deps on unicode/datastruct once those land.
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
