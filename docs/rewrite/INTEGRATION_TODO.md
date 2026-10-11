# Cross-crate integration TODOs

Collected from agent reports; resolve during integration phases.

- [ ] vt `osc/parsers/semantic_prompt.rs`: local printf-q / percent decoders → share with `ghostty-os::string_encoding` (vt can't depend on os; maybe move into vt and have os re-export).
- [ ] os `locale::ensure_locale` no-op → call setlocale from apprt-gtk (unsafe allowed) at startup, as Ghostty's global.zig does.
- [ ] os `passwd` Flatpak branch needs termio Pty.
- [ ] os `open::OpenUrlKind` → replace with core apprt action type.
- [ ] font `sprite/face.rs`: local `Presentation`/`RenderOptions` stand-ins → real font types (Phase 3).
- [ ] font crate: re-add deps on config/unicode/terminal in Phase 3.
- [ ] terminal crate: re-add deps on unicode/datastruct once those land.
