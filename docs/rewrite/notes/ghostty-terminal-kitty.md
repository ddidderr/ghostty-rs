# ghostty-terminal: kitty graphics (agent K1)

## Ported (Zig -> Rust, `crates/ghostty-terminal/src/kitty/`)
| Zig | Rust | tests |
|---|---|---|
| graphics_image.zig | `graphics_image.rs` (`LoadingImage`, `Image`, `Rect`, `Error`) | 11/11 Zig + 3 extra (nul/`/proc` path rejection, file offset+size, shared memory) |
| graphics_storage.zig | `graphics_storage.rs` (`ImageStorage`, `Placement`, `PlacementKey`, `TerminalInfo`) | 19/19 Zig + 3 extra (placement replace releases pin, eviction order + limit, image_by_number newest) |
| graphics_unicode.zig | `graphics_unicode.rs` (`placement_iterator`, `Placement`, `render_placement`) | 16/16 Zig + 2 extra |
| graphics_render.zig | `graphics_render.rs` (`Placement`) | no tests in Zig |
| graphics_exec.zig | `graphics_exec.rs` (`execute`, `ExecTerminal` trait) | 8/8 Zig + 4 extra (run against a harness, see below) |
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

## graphics_exec and Terminal (TODO(integration), needs T3)
`Screen.kitty_images` is still the placeholder `KittyImages { dirty }`, so `Terminal` cannot implement `ExecTerminal` yet.
`execute` is generic over the `ExecTerminal` trait; the exec tests run against a `Harness { t: Terminal, storage: ImageStorage }`
(real `Terminal::index`/`set_cursor_pos`/pages; storage held beside it). No tests are ignored.

### Integration steps
1. `screen.rs`: replace `KittyImages` with `pub kitty_images: crate::kitty::graphics_storage::ImageStorage` (`Default` = Zig `.{}`).
   - `Screen::new`: after constructing, `result.kitty_images.set_limit(&mut result.pages, opts.kitty_image_storage_limit).unwrap()`
     (infallible there; `set_limit` returns `Result`). Opts default is already 320_000_000.
   - `Screen::reset` (Zig `self.kitty_images.deinit(self); self.kitty_images = .{ .dirty = true }`):
     `self.kitty_images.deinit(&mut self.pages); self.kitty_images = ImageStorage { dirty: true, ..Default::default() };`
   - `Drop`/explicit teardown: `kitty_images.deinit(&mut pages)` must run before pages are dropped (tracked pins). `clone_screen` does not clone images.
   - the existing `self.kitty_images.dirty = true` sites keep working unchanged.
2. `terminal.rs`:
   - `erase_display` `.complete` and `.scrollback`... (Zig lines ~2495/2552): `let info = TerminalInfo::from_terminal(self); let s = self.screens.active_mut(); s.kitty_images.delete(&mut s.pages, &info, Delete::All(true));`
   - `switch_screen_mode`/`set_alternate`: new-screen `kitty_image_storage_limit` = primary `kitty_images.total_limit`; then `new.kitty_images.dirty = true`.
   - add `Terminal::kitty_graphics_command(&mut self, cmd: &Command) -> Option<Response> { kitty::graphics_exec::execute(self, cmd) }`
     (Zig `Terminal.kittyGraphics`).
   - `impl ExecTerminal for Terminal`:
     ```rust
     fn kitty_parts(&mut self) -> (&mut ImageStorage, &mut PageList) { let s = self.screens.active_mut(); (&mut s.kitty_images, &mut s.pages) }
     fn exec_info(&self) -> TerminalInfo { TerminalInfo::from_terminal(self) }
     fn exec_cursor_pin(&self) -> Pin { self.screens.active().cursor_pin() }
     fn exec_index(&mut self) -> Result<(), page::Error> { self.index() }
     fn exec_set_cursor_pos(&mut self, row: usize, col: usize) { self.set_cursor_pos(row, col) }
     ```
   - `print`: Zig sets row `kitty_virtual_placeholder` when printing `kitty::graphics_unicode::PLACEHOLDER` (already present in Rust, uses its own constant; can switch to `crate::kitty::unicode::PLACEHOLDER`).
3. `stream_readonly`/termio: APC `G` -> `ghostty_vt::apc` already yields a parsed `Command`; termio calls `terminal.kitty_graphics_command(&cmd)` and, if `Some(resp)`, writes `resp.encode(..)` to the pty.
   Termio config: `screen.kitty_images.set_limit(&mut screen.pages, limit)` (Termio.zig 265/469); clear-screen binding: `Delete::All(true)` as in step 2 (Termio.zig 615).
4. Renderer (`renderer/image.zig` equivalent): iterate `storage.placements` / `storage.images`, `Placement::rect`/`grid_size`/`calculated_size`,
   `graphics_unicode::placement_iterator(&pages, top, Some(bot))` + `Placement::render_placement(storage, img, cell_w, cell_h)`; reset `storage.dirty = false` after upload.
   Font shaper uses `kitty::unicode::PLACEHOLDER`.
5. Once Terminal implements the trait, the exec tests' `Harness` can be replaced by `Terminal` directly.

## Quality gate (2026-10-11)
`cargo build/test/clippy -D warnings -p ghostty-terminal` pass (1243 tests incl. 66 kitty: 54 ported + 12 extra); rustfmt applied to `kitty/*.rs`.
