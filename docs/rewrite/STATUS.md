# Status

Legend: ⬜ todo · 🟨 in progress · ✅ done (gate passed, committed) · ⚠️ done with gaps

| WP | Scope | Owner | State | Notes |
|---|---|---|---|---|
| P0 | scaffolding | opus | ✅ | |
| W1 | vt: parser/osc/color | sonnet | ✅ | |
| W2 | vt: csi/sgr/modes/kitty/apc/dcs | sonnet | ✅ | |
| W3 | unicode + datastruct | sonnet | ✅ | |
| W4 | os | sonnet | ⚠️ | ensure_locale no-op, flatpak untested | |
| W5 | input | sonnet | ✅ | |
| W6 | config | sonnet | ✅ | |
| W7 | font: metrics + sprite | sonnet | ✅ | |
| S1 | vt: stream.zig + simd | sonnet | ✅ | |
| T1 | terminal: size/point/style/hyperlink/rcset/page/PageList | codex astra | ⚠️ | partial committed; round 2 (full parity) running in worktree ../ghostty-rs-t1 branch t1-pagelist |
| F1 | font engine (freetype/fontconfig/harfbuzz, Collection, SharedGrid) | sonnet | ✅ | run.zig + SharedGridSet deferred (F2) | |
| IO1 | termio: pty, Command, shell integration, terminfo | sonnet | ✅ | |
| C1 | cli actions | sonnet | ✅ | list-fonts, show-face, new-window deferred | |
| T2 | terminal: Screen, ScreenSet, Tabstops, Selection | sonnet | ✅ | 3 tests ignored pending pagelist | |
| R1 | renderer: OpenGL base, shaders, shadertoy, size, cursor | sonnet | ✅ | |
| A0 | apprt contract types | sonnet | ✅ | |
| G1 | apprt-gtk (GTK4/libadwaita app) | sonnet | 🟨 | against CoreApp/CoreSurface traits |
| T3 | terminal: Terminal + stream_readonly + StringMap | sonnet | 🟨 | |
| F2 | font: run.zig, SharedGridSet, config integration, terminal shaper tests | sonnet | ✅ | |
| K1 | terminal: kitty graphics image/storage/unicode/render/exec | sonnet | 🟨 | |

## Log
- 2026-10-11: Phase 0 complete; Phase 1 launched.
- 2026-10-11: vt (W1+W2) and font sprite (W7) committed; S1 + T1 launched.
- 2026-10-11: Phase 1 done (vt, unicode, datastruct, os, input, config, font sprite). F1, IO1, C1 launched; T1 (Codex) still running.
- 2026-10-11: termio IO1 + font F1 committed. T1 partial (snapshot reflow, head-walking pins) → Codex round 2 in worktree. T2, R1 launched.
- 2026-10-11: cli, apprt contract, renderer base committed. G1 (GTK) launched.
- 2026-10-11: Screen committed. T3, F2 launched.
- 2026-10-11: F2 committed; K1 launched.
