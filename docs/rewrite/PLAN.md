# Rewrite plan

Orchestrator: Opus (plans, designs core abstractions, reviews, integrates,
commits). Bulk porting: Sonnet subagents. Hard algorithmic cores: Codex
(gpt-6-astra) or Opus. Progress lives in `STATUS.md`.

## Phases

### Phase 0 — scaffolding (done)
Workspace, crate skeletons, docs, lints.

### Phase 1 — leaf crates (parallel)
- W1 `ghostty-vt` part A: Parser, parse_table, UTF8Decoder, osc + osc/parsers/*, osc/encoding, color, x11_color, kitty/color.
- W2 `ghostty-vt` part B: csi (incl. `SepList`), sgr, modes, charsets, ansi, device_status, mouse_shape, kitty/key, kitty/graphics_command, apc, dcs.
- W3 `ghostty-unicode` + `ghostty-datastruct`.
- W4 `ghostty-os`.
- W5 `ghostty-input`.
- W6 `ghostty-config` (+ cli/args, CommaSplitter, diagnostics).
- W7 `ghostty-font`: Metrics + sprite/*.

### Phase 2 — terminal core
- `ghostty-vt::stream` (stream.zig) once W1/W2 land.
- `ghostty-terminal`: style, hyperlink, size, point, ref_counted_set, page, PageList (Codex/Opus: hardest part), then Screen, Tabstops, Terminal, ScreenSet, Selection, stream_readonly, formatter, render, highlight, search/*, kitty graphics storage/image/exec/unicode, tmux.

### Phase 3 — font engine
discovery (fontconfig), freetype face, harfbuzz shaper (+run, feature, Cache), opentype tables, Collection, CodepointResolver, CodepointMap, DeferredFace, SharedGrid, SharedGridSet, Atlas, embedded fonts, nerd_font_attributes.

### Phase 4 — termio
pty, Command (fork/exec), Exec backend, Termio, Thread, mailbox, stream_handler, shell_integration (+ copy `ZIG/src/shell-integration`).

### Phase 5 — renderer
OpenGL backend (glow), generic renderer logic, cells, cursor, image, link, size, State, Thread, Overlay, shadertoy custom shaders.

### Phase 6 — core + GTK app
ghostty-core: App, Surface, surface_mouse, apprt action/structs, CLI actions. ghostty-apprt-gtk: application, windows, tabs, splits, surface widget (GLArea, input, IME, clipboard), config reload, dialogs, notifications, quick terminal, IPC (+new-window), CSS/UI.

### Phase 7 — integration and polish
End-to-end run, vttest-ish smoke tests, perf, inspector (maybe), docs, CHANGELOG.

## Review protocol
After each work package: orchestrator reads the agent report + notes, spot-checks
code (esp. tricky logic), runs quality gate on the workspace, commits. Larger
cores get an `/astrareview` or Codex review pass.

## Stopping criterion for review rounds (decided 2026-10-12)
Audit rounds RV1–RV17 converged to P3-only findings (no P1 in the last five
rounds). To avoid an endless review loop, RV18 is the final audit round. From
it, only P1/P2 findings, user-visible behaviour in normal use, memory-safety
issues and regressions are fixed; remaining P3 nitpicks are recorded in
docs/rewrite/KNOWN_DIFFERENCES.md instead of triggering further rounds.
