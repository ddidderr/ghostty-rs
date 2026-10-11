# Ghostty-rs Architecture

Rust rewrite of Ghostty 1.3.1. Reference source: `../ghostty/` (Zig), referred
to below as `ZIG/` (= `../../../ghostty`).

Target platform: **Linux (GTK4 + libadwaita, OpenGL)**. macOS/Metal/CoreText,
WASM, Windows and the embedded C API (`libghostty`) are out of scope.

## Crate map (workspace `crates/`)

| Crate | Ports (ZIG/src/...) | Depends on |
|---|---|---|
| `ghostty-vt` | terminal/Parser, parse_table, UTF8Decoder, osc.zig + osc/*, csi, dcs, apc, sgr, color, x11_color, charsets, modes, ansi, device_status, mouse_shape, kitty/key, kitty/color, kitty/graphics_command, stream.zig | – |
| `ghostty-unicode` | unicode/* (width, grapheme break, props) | – |
| `ghostty-datastruct` | datastruct/* (split_tree, lru, circ_buf, blocking_queue, cache_table, ...) | – |
| `ghostty-os` | os/* (xdg, homedir, shell, locale, passwd, env, open, desktop, resourcesdir, hostname, TempDir, uri, ...) | – |
| `ghostty-input` | input/* (key, key_mods, keycodes, function_keys, key_encode, Binding, command, mouse, paste, kitty, Link) | vt |
| `ghostty-config` | config/*, cli/args.zig, cli/CommaSplitter.zig, cli/diagnostics.zig | input, os, vt |
| `ghostty-terminal` | terminal/ page, PageList, Screen, ScreenSet, Terminal, style, hyperlink, Tabstops, size, point, Selection, formatter, render, highlight, search/*, kitty/graphics_{image,storage,exec,unicode,render}, tmux/*, stream_readonly, StringMap, ref_counted_set, bitmap_allocator semantics | vt, unicode, datastruct |
| `ghostty-font` | font/* (Metrics, sprite/*, Atlas, discovery via fontconfig, face via freetype, shaper via harfbuzz, Collection, CodepointResolver, SharedGrid(Set), opentype/*, nerd_font_attributes) | config, unicode, terminal |
| `ghostty-renderer` | renderer/ generic, OpenGL (+ opengl/*), cell, cursor, image, link, size, State, Thread, Overlay, shadertoy, row, shaders/glsl | config, font, terminal |
| `ghostty-termio` | termio/*, pty.zig, Command.zig, shell-integration resources | config, terminal, os |
| `ghostty-core` | App.zig, Surface.zig, surface_mouse.zig, apprt/action.zig, apprt/structs.zig, apprt/surface.zig, global.zig, cli/* actions (+list-fonts, +show-config, ...) | everything below |
| `ghostty-apprt-gtk` | apprt/gtk/* | core, config, input |
| root `ghostty-rs` (bin `ghostty`) | main_ghostty.zig, build_config.zig | core, config, apprt-gtk |

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
- The ImGui inspector is deferred until everything else works (Phase 7).
