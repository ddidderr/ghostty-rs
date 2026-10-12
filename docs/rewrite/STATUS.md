# Status

**Overall:** Linux/GTK port complete and verified (3038 tests). **Phase 8 in progress:** libghostty C API, libghostty-vt C API, WASM, macOS Zig paths, Windows, crash handling, GTK compat fallbacks, remaining verification + perf.

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
| V1b | finish V1: clippy, audit fixes, GTK replay check | sonnet | ✅ | GTK typing/tabs/splits/copy-paste/font-size verified |
| P2 | perf: screen-clone, osc-parser | sonnet | ✅ | clone at parity; render 2.4x |
| I1 | inspector (ImGui) crate + GTK widgets + core hooks | sonnet | ✅ | verified in real app |
| RV2 | Codex completeness audit (every Zig file → Rust) | codex astra | ✅ | 319 full / 28 partial / 7 missing → X1, X2 |
| X1 | distribution: completions, vim/bat, man pages, translations, install modes, build metadata | sonnet | ✅ | byte-identical to installed Ghostty |
| X2 | correctness gaps from RV2 (flatpak, env bytes, case folding, ...) | sonnet | ✅ | all 10 items |
| V3 | real-app verification: nvim/htop/less/tmux, mouse, unicode, kitty images, paste dialog | sonnet | ✅ | no product bugs found |
| DOC | README, CHANGELOG, ARCHITECTURE decomposition | opus | ✅ | |

## Phase 8 — full scope
| WP | Scope | Owner | State | Notes |
|---|---|---|---|---|
| L2 | libghostty-vt C API (lib_vt.zig, terminal/c/*, vt.h) + run example/c-vt* + wasm examples | sonnet | ✅ | ABI-checked, examples run |
| W1 | WASM: main_wasm, config/Wasm, os/wasm, font web_canvas face+shaper | sonnet | ✅ | W1b aligning zig-js ABI |
| CR | crash/* (sentry envelope, local crash reports, thread context hooks) | sonnet | ✅ | |
| PF | perf: RenderState render 2.4x, parser SGR 1.5x | sonnet | ✅ | all benches ≤ Zig |
| VR | verify IME, paste dialog confirm/deny, quick terminal (niri layer-shell), Flatpak via mock D-Bus | sonnet | ✅ | fixed quick-terminal link-order bug |
| L1 | libghostty C API (apprt/embedded.zig, main_c.zig, config CApi/c_get, ghostty.h ABI) | sonnet | ✅ | ABI-checked; Swift app not built |
| M1 | macOS: Metal renderer, CoreText face/shaper/discovery, KeymapDarwin, os macos/mach | sonnet | ✅ | compile-checked apple-darwin |
| WIN | Windows: ConPTY pty, os/windows, Command | sonnet | ✅ | compile-checked windows-gnu |
| W1b | wasm: align JS imports with zig-js ABI | sonnet | ✅ | |
| GC | GTK/adw compat fallbacks, bench wcwidth/uucode modes, helpgen generators, cli windows, ssh-cache lock | sonnet | ✅ | |
| RV3 | final Codex completeness re-audit (full scope) | codex astra | ✅ | 397 full / 30 partial / 1 missing + 8 artifact gaps |
| Z1 | final gaps: os/config/cli byte fidelity, rlimit fallbacks, i18n map, build-info source | sonnet | ✅ | |
| Z2 | final gaps: core Windows, embedded init, lib/core/gtk bytes, version wiring | sonnet | ✅ | |
| Z3 | final gaps: C allocator, shadertoy overloads + MSL stride, tripwire, crash minidump | sonnet | ✅ | |
| Z4 | final gaps: webdata mdx, xcframework, lib install + pc, app id, i18n extract, dist, win resources | sonnet | ✅ | |

| XC | cross-check scripts: macOS (2 targets), Windows, wasm — all clippy -D warnings clean | opus | ✅ | just check-all |
| RV4 | targeted Codex re-audit of RV3 rows | codex astra | ✅ | 26/39 resolved, 13 tail items → Z5 |
| Z5 | close RV4 tail items (encoded paths at OS boundaries, osbytes non-unix, shader arrays, C alloc, crash aarch64/Darwin, tripwire test, vt soname) | sonnet | ✅ | 3342 tests |
| Z6 | Breakpad-equivalent minidumps on macOS (Mach exception port) + Windows; screen-file open path | sonnet | ✅ | |
| RV5 | final targeted Codex check | codex astra | ✅ | 7/13 resolved, 6 partial + 10 sweep groups, 2 regressions → Z7 |
| Z7 | type-safe encoded OS strings + all RV5 findings (incl. Mach handler hang, open.rs regression, color OSC alloc) | sonnet | ✅ | 3376 tests, all cross-checks green |
| RV6 | final Codex verification + sweep + verdict | codex astra | ✅ | 22/23 resolved; 7 small gap groups → Z8 |
| Z8 | RV6 gaps: crash abort handoff, SHELL/PATH bytes, Windows env, show-config round-trip, lib locale | sonnet | ✅ | 3390 tests |
| RV7 | final Codex verification of RV6 rows + sweep + verdict | codex astra | ✅ | all prior rows resolved; 9 narrow byte/Windows gaps → Z9 |
| Z9 | RV7 gaps + systematic config-string consumer audit | sonnet | ✅ | 3405 tests; flaky TMPDIR test race fixed |
| Z10 | byte-typed C API key/preedit text | sonnet | ✅ | 3407 tests; preedit error semantics aligned with Zig by orchestrator |
| RV8 | final Codex verification (RV7 gaps, Z9/Z10) + sweep + verdict | codex astra | ✅ | 9 edge-case groups; root cause = escape-encoded config strings |
| BX | structural: byte-faithful config value parsing (fields parse &[u8] like Zig) + RV8 findings | codex astra | ✅ | 3428 tests; CLI outputs identical to Ghostty 1.3.1 |
| RV9 | final Codex verification + full-scope sweep | codex astra | ✅ | RV8 9/9 resolved; 6 differential gaps (NUL panic, numeric grammar, ...) → Z11 |
| Z11 | RV9 gaps: NUL font names, Zig parseInt/parseFloat, C NUL, ASCII trim, ArgIteratorGeneral, diagnostics | sonnet | ✅ | 3456 tests; 202-field differential vs installed Ghostty: 0 mismatches |
| Z12 | Zig numeric parsing in VT/terminal/protocol parsers | sonnet | ✅ | 3468 tests |
| DF | differential fuzzing: Zig terminal core vs Rust terminal core (Zig 0.15.2 driver) | sonnet | 🟨 | |
| RV10 | final Codex verification (RV9 rows + full sweep) | codex astra | 🟨 | |

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
- 2026-10-12: V1b + P2 committed (2983 tests). I1 inspector + RV2 completeness audit launched.
- 2026-10-12: I1, X1, X2 committed (3037 tests). Docs written. V3 real-app verification running.
- 2026-10-12: V3 verification passed, final polish (nightly fmt) done. Rewrite complete.
- 2026-10-12: Stop-hook review: 'full end-to-end' not met → Phase 8 (full scope) started.
- 2026-10-12: Phase 8 batch A + M1/WIN/W1b committed (3224 tests). L1, GC launched.
- 2026-10-12: L1 (libghostty) + GC committed (3278 tests). Final completeness re-audit RV3 running.
- 2026-10-12: RV3 re-audit: 397/428 full. Z1–Z4 launched for all remaining rows.
- 2026-10-12: Z1–Z4 committed (3327 tests); cross-check scripts added and passing; RV4 running.
- 2026-10-12: Z5 committed (3342 tests, all cross-checks green). Z6 (macOS/Windows minidumps) launched.
- 2026-10-12: Z6 committed (3348 tests). RV5 found remaining byte-path leaks + 2 regressions → Z7 (structural fix).
- 2026-10-12: Z7 committed (EncodedOs newtype; 3376 tests). RV6 final verification running.
- 2026-10-12: Z8 committed (3390 tests, cross-checks green). RV7 running.
- 2026-10-12: Z9 committed (audit table in notes/final-Z9.md). Z10 launched; RV8 after.
- 2026-10-12: Z10 committed (3407 tests, all cross-checks green). RV8 running.
- 2026-10-12: RV8: gap count stalled (escape design). Codex BX structural refactor launched.
- 2026-10-12: BX committed (3428 tests, all cross-checks green, oracle diffs clean). RV9 running.
- 2026-10-12: RV9: all RV8 rows resolved; 6 new differential gaps → Z11.
- 2026-10-12: Z11 committed (3456 tests). Z12 launched; RV10 after.
- 2026-10-12: Z12 committed (3468 tests). DF (Zig-vs-Rust differential fuzzing) + RV10 launched.
