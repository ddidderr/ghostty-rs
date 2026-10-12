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
| T1 | terminal: size/point/style/hyperlink/rcset/page/PageList | codex astra | ✅ | full parity; open API requests in notes/ghostty-terminal-pagelist-requests.md |
| F1 | font engine (freetype/fontconfig/harfbuzz, Collection, SharedGrid) | sonnet | ✅ | run.zig + SharedGridSet deferred (F2) | |
| IO1 | termio: pty, Command, shell integration, terminfo | sonnet | ✅ | |
| C1 | cli actions | sonnet | ✅ | list-fonts, show-face, new-window deferred | |
| T2 | terminal: Screen, ScreenSet, Tabstops, Selection | sonnet | ✅ | | |
| R1 | renderer: OpenGL base, shaders, shadertoy, size, cursor | sonnet | ✅ | |
| A0 | apprt contract types | sonnet | ✅ | |
| G1 | apprt-gtk (GTK4/libadwaita app) | sonnet | ✅ | against CoreApp/CoreSurface traits; untested interactively |
| T3 | terminal: Terminal + stream_readonly + StringMap | sonnet | ✅ | tmux viewer not wired | |
| F2 | font: run.zig, SharedGridSet, config integration, terminal shaper tests | sonnet | ✅ | |
| K1 | terminal: kitty graphics image/storage/unicode/render/exec | sonnet | ✅ | wired (TI) | |
| T4 | terminal: render state, highlight, formatter | sonnet | ✅ | |
| T5 | terminal: search + tmux viewer | sonnet | ✅ | tmux viewer not wired to termio yet | |
| IO2 | termio: Exec, Termio, Thread, stream_handler | sonnet | ✅ | |
| R2 | renderer: generic, cell, row, link, State, Overlay, image, Thread | sonnet | ✅ | kitty update_kitty stub | |
| TI | terminal integration: kitty wiring + PageList requests, remove workarounds | codex astra | ✅ | 1252 tests |
| CORE | core App/Surface, trait move, state unification, root main, run app | sonnet | ✅ | app runs; input untested at runtime |
| V1 | headless e2e tests (input/mouse/clipboard/actions) + GTK input replay, fix bugs | sonnet | ⚠️ | stalled twice; 81 e2e tests pass; handed to V1b |
| V2 | resources bundling (themes, terminfo), INTEGRATION_TODO cleanup | sonnet | ✅ | termio/gtk parts commit with V1 |
| RV1 | Codex read-only audit core Surface vs Zig | codex astra | ✅ | 9 findings (notes/review-core-codex.md) → handed to V1 |
| P1 | ghostty-bench port, Zig baseline, perf tuning | sonnet | ✅ | stream at Zig parity; screen-clone 2.6x |
| V1b | finish V1: clippy, audit fixes, GTK replay check | sonnet | 🟨 | |
| P2 | perf: screen-clone, osc-parser | sonnet | ✅ | clone at parity; render 2.4x |

## Log
- 2026-10-11: Phase 0 complete; Phase 1 launched.
- 2026-10-11: vt (W1+W2) and font sprite (W7) committed; S1 + T1 launched.
- 2026-10-11: Phase 1 done (vt, unicode, datastruct, os, input, config, font sprite). F1, IO1, C1 launched; T1 (Codex) still running.
- 2026-10-11: termio IO1 + font F1 committed. T1 partial (snapshot reflow, head-walking pins) → Codex round 2 in worktree. T2, R1 launched.
- 2026-10-11: cli, apprt contract, renderer base committed. G1 (GTK) launched.
- 2026-10-11: Screen committed. T3, F2 launched.
- 2026-10-11: F2 committed; K1 launched.
- 2026-10-11: Terminal committed (849 tests). T4, T5, IO2 launched.
- 2026-10-11: PageList parity merged (913 tests, 0 ignored). T4 committed; R2 launched.
- 2026-10-11: T5, K1 committed (1243 terminal tests). Codex TI launched.
- 2026-10-11: GTK runtime committed; terminal integration pass done (1252 tests, 0 ignored).
- 2026-10-11: IO2, R2 committed. CORE integration launched (Phase 6).
- 2026-10-12: CORE committed — `cargo run` opens a working terminal (2890 tests). Phase 7 verification: V1, V2, RV1 launched.
- 2026-10-12: P1 (perf parity on terminal-stream) and V2 cleanup committed.
- 2026-10-12: V1 stalled (large transcript); restarted fresh as V1b. P2 launched.
