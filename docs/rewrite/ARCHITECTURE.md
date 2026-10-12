# Ghostty-rs Architecture

Rust rewrite of Ghostty 1.3.1. Reference source: `../ghostty/` (Zig), referred
to below as `ZIG/` (= `../../../ghostty`).

Primary platform: **Linux (GTK4 + libadwaita, OpenGL)** — built, tested and
verified at runtime here. Phase 8 extends the port to the rest of Ghostty's
Zig code: the embedded `libghostty` C API (identical `include/ghostty.h` ABI,
so Ghostty's unchanged Swift macOS app can link the Rust library), the
`libghostty-vt` C API (`include/ghostty/vt*.h`), WASM (wasm32), macOS
(Metal renderer, CoreText fonts, Darwin keymap/os) and Windows (ConPTY).
Non-Linux targets are compile-checked (`cargo check --target ...`) on this
machine; they cannot be run here. The Swift macOS app itself is not Zig and
stays as-is (it is a client of the C ABI).

## Top-down decomposition

```
ghostty (bin, src/main.rs)            CLI `+action` dispatch or GTK app startup
├── ghostty-cli                       `ghostty +action` commands
├── ghostty-apprt-gtk                 GTK4/libadwaita runtime: windows, tabs, splits,
│   │                                 dialogs, GLArea surfaces, IPC, inspector windows
│   └── ghostty-apprt                 core<->runtime contract (actions, messages, traits)
└── ghostty-core                      App + Surface: input routing, bindings, mouse,
    │                                 clipboard, config reload, per-surface threads
    ├── ghostty-termio                IO thread, pty, child process, stream handler
    │   └── ghostty-terminal          terminal state machine
    │       ├── Terminal / Screen     cursor, modes, scrolling, alt screen, selection
    │       ├── PageList / Page       scrollback memory model (slab nodes, tracked pins)
    │       ├── render / formatter    per-frame RenderState, text/VT/HTML export
    │       ├── kitty / search / tmux kitty graphics, search thread, tmux control mode
    │       └── ghostty-vt            byte stream -> Parser -> Stream -> Handler actions
    ├── ghostty-renderer              OpenGL renderer (glow), renderer thread, shaders
    │   └── ghostty-font              fontconfig/freetype/harfbuzz, sprite font, atlas
    ├── ghostty-inspector             Dear ImGui terminal inspector
    └── ghostty-config                config file/CLI parsing, themes, keybinds
        ├── ghostty-input             keys, key encoding, binding sets, mouse
        └── ghostty-os                XDG, env, shell, locale, resources dir, flatpak
Support: ghostty-unicode (width/grapheme/case-fold tables), ghostty-datastruct
(split tree, LRU, queues), ghostty-onig (Oniguruma retry-limit FFI),
ghostty-bench (benchmarks), ghostty-build-data (completions, man pages, syntax).
```

## Crate map (workspace `crates/`)

| Crate | Ports (ZIG/src/...) | Internal deps |
|---|---|---|
| `ghostty-vt` | terminal/Parser, parse_table, UTF8Decoder, osc.zig + osc/*, csi, dcs, apc, sgr, color, x11_color, charsets, modes, ansi, device_status, mouse_shape, kitty/{key,color,graphics_command}, tmux/control, stream.zig, simd (scalar) | – |
| `ghostty-unicode` | unicode/* (width, grapheme break, props, symbols) + case folding, generated from vendored UCD 17.0.0 | – |
| `ghostty-datastruct` | datastruct/* (split_tree, lru, circ_buf, blocking_queue, cache_table, ...) | – |
| `ghostty-onig` | (Oniguruma whole-search retry limit used by StringMap.zig) | – |
| `ghostty-os` | os/* (xdg, homedir, shell, locale, passwd, env, open, desktop, resourcesdir, hostname, TempDir, uri, flatpak, systemd, cgroup, ...) | vt |
| `ghostty-input` | input/* (key, key_mods, keycodes, function_keys, key_encode, Binding, command, mouse, paste, kitty, Link) | vt, unicode |
| `ghostty-terminal` | terminal/ page, PageList, Screen, ScreenSet, Terminal, style, hyperlink, Tabstops, size, point, Selection, formatter, render, highlight, search/*, kitty/graphics_*, tmux/{layout,output,viewer}, stream_readonly, StringMap, ref_counted_set | vt, unicode, datastruct, onig |
| `ghostty-config` | config/*, cli/args.zig, cli/CommaSplitter.zig, cli/diagnostics.zig | input, os, terminal, vt |
| `ghostty-font` | font/* (Metrics, sprite/*, Atlas, discovery (fontconfig), face (freetype), shaper (harfbuzz) + run, Collection, CodepointResolver, SharedGrid(Set), opentype/*, nerd_font_attributes) | config, terminal, unicode, datastruct |
| `ghostty-renderer` | renderer/ generic, OpenGL + opengl/*, cell, cursor, image, link, size, State, Thread, Overlay, shadertoy, row, shaders/glsl | config, font, terminal, input, unicode, vt, datastruct |
| `ghostty-inspector` | inspector/* (Dear ImGui terminal inspector) | font, input, renderer, terminal, vt |
| `ghostty-apprt` | apprt/action.zig, structs.zig, surface.zig, ipc.zig, runtime traits | config, input, inspector, renderer, terminal, vt, datastruct |
| `ghostty-termio` | termio/*, pty.zig, Command.zig, terminfo/*, shell integration | apprt, config, os, renderer, terminal, vt, datastruct |
| `ghostty-core` | App.zig, Surface.zig, surface_mouse.zig, global.zig | apprt, config, font, input, inspector, os, renderer, terminal, termio, ... |
| `ghostty-cli` | cli/* actions, build_config.zig (build info) | apprt, config, font, input, os, vt |
| `ghostty-apprt-gtk` | apprt/gtk/* (+ ui/*.blp, css) | apprt, config, input, inspector, os, vt, datastruct |
| `ghostty-bench` | benchmark/*, synthetic/* | terminal, unicode, vt |
| `ghostty-build-data` | main_build_data.zig, extra/*, build/mdgen | cli, config, input, os, termio |
| root `ghostty-rs` (bin `ghostty`) | main_ghostty.zig | apprt, apprt-gtk, cli, config, core, os |

Non-code: `resources/` (shell-integration, themes, terminfo), `dist/` (desktop
integration), `po/` (translations), crate-local `res/`/`shaders/`/`resources/`.

## Key design decisions

1. **Safe Rust by default.** Workspace lint `unsafe_code = "forbid"`. Crates
   that must do FFI/syscalls (renderer GL loader, termio pty/fork, apprt-gtk)
   may override with `unsafe_code = "deny"` and `#[allow(unsafe_code)]` on
   minimal, `// SAFETY:`-commented items. Prefer safe wrapper crates (rustix,
   glow, gtk4-rs) over raw FFI.
2. **Pages without raw memory.** Ghostty's `page.zig` packs a page into one
   mmap'd buffer with offsets. In Rust a `Page` owns `Vec<Row>` + `Vec<Cell>`
   (row-major, `cols` per row) plus per-page `RefCountedSet<Style>`, grapheme
   map `HashMap<cell_index, Vec<u32>>`, hyperlink set/map, string storage.
   **Capacity accounting is preserved logically** (styles, grapheme bytes,
   hyperlink bytes, string bytes, rows/cols), including Ghostty's byte-size
   layout formula (`Page::layout(cap).total_size`), because PageList pruning
   (`scrollback-limit` is in bytes), page splitting and capacity-adjust
   behaviour — and the tests for them — depend on it.
3. **PageList as an index-based linked list.** Nodes live in a slab
   (`Vec<Option<Node>>` + free list); `NodeId` is an index (+ generation for
   debug checks). `Pin { node: NodeId, y, x, garbage }` is `Copy`. Tracked pins
   live in a PageList-owned slab; holders keep a `TrackedPin` handle
   (index) and read/write through the PageList. All operations that move
   rows update tracked pins exactly like the Zig code.
4. **Cell is a compact `Copy` value** (target 8 bytes, bit-packed u64 with
   accessor methods mirroring Zig field names), so memory accounting and
   copying behaviour stay close to Ghostty.
5. **Parser → Stream → Handler.** `ghostty-vt::Parser` yields actions;
   `ghostty-vt::stream::Stream<H: Handler>` decodes CSI/ESC/OSC/DCS/APC into
   high-level actions and calls `Handler`. Terminal state changes live in
   `ghostty-terminal` (`stream_readonly` handler applies to `Terminal`), and
   `ghostty-termio::StreamHandler` handles side-effecting actions.
6. **Rendering:** OpenGL 4.3 via `glow`, inside a `gtk::GLArea`. Shaders are
   copied verbatim from `ZIG/src/renderer/shaders/glsl`. Renderer logic
   follows `renderer/generic.zig` (cell contents rebuilt from
   `terminal::render::RenderState`).
7. **Fonts:** fontconfig (discovery), freetype (rasterization), harfbuzz
   (shaping). Sprite font (box drawing etc.) is pure Rust, ported 1:1.
8. **Threads:** like Ghostty — per surface: IO thread (termio), renderer
   thread, plus a pty read thread. Mailboxes are `crossbeam-channel` /
   `std::sync::mpsc`; the GTK main thread is woken via `glib` channels.
   The `Terminal` is shared behind `Arc<Mutex<..>>` (Ghostty's renderer mutex).
9. **Config:** same file format, same keys, same defaults, same CLI flags,
   same docs text (used by `+show-config --docs` and `--help`).
10. **Tests:** every Zig `test "..."` block in a ported file is ported to a
    Rust `#[test]` with the same name (snake_cased) in the same module.
    `scripts/test-progress.sh` tracks counts.

## Out of scope / deferred

- macOS (Swift app, Metal, CoreText), Windows, WASM, libghostty C API.
- (The ImGui inspector was deferred and is now ported: `docs/rewrite/notes/ghostty-inspector.md`.)

## Scope changes

- 2026-10-12: Sentry crash handling and GTK<4.14/libadwaita<1.5 fallbacks,
  previously omitted, are ported in Phase 8 for full parity.

## Intentional omissions (decided by the orchestrator)

- Sentry crash reporting (`crash/*`): optional and disabled by default in
  upstream Linux builds; `+crash-report` listing is ported.
- GTK < 4.14 / libadwaita < 1.5 compatibility fallbacks (MessageDialog,
  pre-Banner widgets, old about dialog, 1.0–1.3 blueprint variants): the
  Rust port requires GTK ≥ 4.14 and libadwaita ≥ 1.5.
