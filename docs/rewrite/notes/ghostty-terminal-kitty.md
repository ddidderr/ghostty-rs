# ghostty-terminal: kitty graphics (agent K1)

## Ported (Zig -> Rust, `crates/ghostty-terminal/src/kitty/`)
| Zig | Rust | tests |
|---|---|---|
| graphics_image.zig | `graphics_image.rs` (`LoadingImage`, `Image`, `Rect`, `Error`) | 11/11 Zig + 3 extra (nul/`/proc` path rejection, file offset+size, shared memory) |
| graphics_storage.zig | `graphics_storage.rs` (`ImageStorage`, `Placement`, `PlacementKey`, `TerminalInfo`) | 19/19 Zig + 3 extra (placement replace releases pin, eviction order + limit, image_by_number newest) |
| graphics_unicode.zig | `graphics_unicode.rs` (`placement_iterator`, `Placement`, `render_placement`) | 16/16 Zig + 2 extra |
| graphics_render.zig | `graphics_render.rs` (`Placement`) | no tests in Zig |
| graphics_exec.zig | `graphics_exec.rs` (`execute`, `ExecTerminal` trait) | 8/8 Zig + 8 extra (real Terminal, including lifecycle integration) |
| kitty.zig / graphics.zig | `kitty/mod.rs` (re-exports; the command parser stays in `ghostty_vt::kitty::graphics_command`) | - |

`pub mod kitty;` added to `lib.rs`. Test data copied to `kitty/testdata/`.
Deps added to `ghostty-terminal`: `flate2`, `png`, `rustix` (features `shm`, `fs`), `thiserror`.
Zig tests skipped: none. (`debugDump` not ported: debug-only helper.)

## Deviations
- **Borrowing**: Zig passes `*Screen`/`*Terminal` to storage methods. Rust's `ImageStorage` is meant to live in
  `Screen.kitty_images`, so methods take `&mut PageList` (disjoint field borrow `&mut screen.pages`) plus a
  `TerminalInfo { cols, rows, width_px, height_px, cursor_x, cursor_y }` snapshot (`TerminalInfo::from_terminal(&Terminal)`).
  `deinit(pages)`, `delete(pages, &info, cmd)`, `set_limit(pages, limit)`, `add_image(pages, img)`, `add_placement(pages, image_id, placement_id, p)`.
- `add_image`/`add_placement`/`set_limit` take the PageList only so that placements evicted or replaced release their tracked pin
  (Zig leaks them in `evictImage` and when an external placement id is re-used).
- Placement location is `Location::Pin(TrackedPin)` (handle into the PageList) instead of `*Pin`; `Placement::rect(&image, &info, &pages)`.
- `imageById`/`imageByNumber` return `Option<&Image>` (no copy). Placements are a `BTreeMap` (deterministic order); images a `HashMap`.
- zlib via `flate2`, PNG via `png` crate (EXPAND|STRIP_16, then converted to 8-bit RGBA like wuffs' RGBA_NONPREMUL); shm via
  `rustix::shm::open` + `File::read_exact_at` (no mmap, so no unsafe). The segment is unlinked after reading, as in Zig.
- File medium `S=` (size): data is truncated to `min(S, 400MB)`. (Zig's `appendRemaining(.limited(n))` may error when the file is longer; I chose
  the Kitty-spec behavior. Zig tests only use S=0.)
- `Placement::rect` uses `saturating_sub(1)` on rows/cols (Zig would underflow for a 0-sized grid).
- Preserved Zig quirks: `delete range` uses `||` (`id >= first || id <= last`, matches everything); eviction stops only when
  `evicted > req` (strictly); `set_limit(0)` resets storage to defaults (limit then set to 0); screen reset `.{ .dirty = true }` also resets total_limit to default.
- `execute(terminal, &cmd)` has no allocator; `Command` stays borrowed (the loading image copies the payload).

## graphics_exec and Terminal integration — complete

`Screen.kitty_images` is the real `ImageStorage`. Initialization applies the
configured limit, reset deinitializes storage and restores default limits with
`dirty = true`, and Drop releases placement pins before dropping PageList.
`clone_screen` does not copy images. The default is still 320,000,000 bytes.

`Terminal` implements `ExecTerminal` by splitting active-screen storage/pages,
snapshotting `TerminalInfo`, and forwarding cursor/index operations. Both
`kitty_graphics(&mut Command)` and the new borrowed
`kitty_graphics_command(&Command)` execute commands and return real responses.
All graphics_exec tests now use Terminal directly; the separate harness is gone.

Reference hook audit:
- `.complete` and `.scroll_complete` display erasure call `Delete::All(true)`.
  `.scrollback` only erases history, as in Terminal.zig.
- Lazy alternate-screen creation inherits the primary image limit; screen
  switching marks the new screen's images dirty.
- All existing image dirty hooks in Screen and Terminal remain at their Zig
  locations (scrolling, index, insert/delete lines, screen switching).
- Full reset drops alternate storage and resets primary storage. Placeholder
  printing uses the shared `graphics_unicode::PLACEHOLDER` constant.

Four additional integration regressions cover screen clone/reset and pin
release, erasure hooks, alternate-screen isolation/limit inheritance/full
reset, and the legacy command entrypoint plus in-progress transfer reset.

## API changes and consumer wiring

Added `Terminal::kitty_graphics_command(&Command)` and `ExecTerminal for
Terminal`; retained `Terminal::kitty_graphics(&mut Command)` unchanged.
`Screen.kitty_images` now has the required concrete ImageStorage type;
existing `.dirty` access remains valid. No downstream callers needed edits.

Termio can pass APC `G` commands through either terminal entrypoint and encode
returned responses to the pty. Storage limits use
`screen.kitty_images.set_limit(&mut screen.pages, limit)`; clear-screen bindings
use `Delete::All(true)`. Renderer consumers can inspect placements/images and
Unicode placements and clear the storage dirty flag after updating graphics.
Consumer implementation beyond the public terminal API remains owned by the
concurrent renderer/termio agents.

## Quality gate (2026-10-11 integration pass)

`cargo test -p ghostty-terminal`: 1252 unit tests (including 70 Kitty tests:
54 source cases + 16 extra) and 1 documentation test pass; 0 failed, 0 ignored.
Clippy with `--all-targets -- -D warnings`, rustfmt, and the combined build of
`ghostty-font`, `ghostty-renderer`, and `ghostty-termio` pass. No commit or version
change was made. Existing non-integration deviations above are unchanged.
