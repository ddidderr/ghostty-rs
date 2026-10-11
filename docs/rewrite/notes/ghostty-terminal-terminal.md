# ghostty-terminal: Terminal, stream_readonly, StringMap

## Ported
| Zig | Rust | tests |
|---|---|---|
| Terminal.zig | `terminal.rs` (+ `terminal/tests/p1..p8.rs`, helpers in `tests/mod.rs`) | 372 of 372 (370 pass, 2 `#[ignore = "pagelist: ..."]`) |
| stream_readonly.zig | `stream_readonly.rs` | 33 of 33 |
| StringMap.zig | `string_map.rs` (uses the `onig` crate; `ghostty-config` is a dev-dependency for the URL regex tests; `Screen`'s stand-in `StringMap` is now a re-export) | 3 of 3 |

Ignored tests (PageList gaps, see `ghostty-terminal-pagelist-requests.md`):
`terminal_index_bottom_of_scroll_region_clear_hyperlinks` (erase_row_bounded keeps the row hyperlink flag) and
`terminal_resize_with_wraparound_on` (reflow to fewer cols loses a row in the active dump).

Small non-test deviations: `glitch.txt` copied next to the tests (used by the glitch-text test); zero-width attach in
`print` restores the row dirty flag because `Page::append_grapheme` marks rows dirty (Zig does not; request in the pagelist file).

## API overview (for termio / renderer / core)
```rust
use ghostty_terminal::terminal::{Terminal, Options};
let mut t = Terminal::new(Options::new(80, 24))?;       // Options{cols,rows,max_scrollback=10_000,colors,default_modes}
{
    let mut s = t.vt_stream();                           // ghostty_vt::stream::Stream<stream_readonly::Handler<'_>> (new_alloc)
    s.next_slice(b"hello\r\n\x1b[31mred\x1b[0m")?;       // applies every state-changing action; queries are ignored
}                                                         // s.handler.terminal gives &mut Terminal while the stream lives
t.resize(100, 30)?;                                       // Zig resize(alloc, cols, rows); primary reflows (if wraparound mode), alt does not
t.print(cp)?; t.print_string("text\n")?; t.full_reset();
let text = t.plain_string()?;                             // viewport as text (plain_string_unwrapped respects wrap)
```
- `t.screens` is a `ScreenSet`: `t.screen()` / `t.screen_mut()` (= `screens.active()/active_mut()`), `t.screens.active_key`, `get(Key)`.
- Reading cells: `scr.pages.get_cell(Point::Viewport(Coordinate{x,y}))` -> `Cell` (Copy); rows/pages via `scr.pages.pin(pt)` + `pin.row_and_cell(&scr.pages)`; page data `scr.page(node)`; style of a cell `page.style(cell)`; graphemes `page.lookup_grapheme(page.cell_index(x,y)?)`.
- Cursor: `t.screen().cursor` (x, y, pending_wrap, style, `cursor_style`, hyperlink_id, semantic_content). Selection: `screen.select(..)`, `screen.selection_string(..)` (Screen API).
- State: `t.modes` (`ModeState`), `t.flags` (mouse_event/format, focused, `dirty` renderer flags, shell_redraws_prompt, ...), `t.colors` (DynamicRgb fg/bg/cursor + DynamicPalette), `t.scrolling_region`, `t.tabstops`, `t.pwd`/`get_pwd()`, `t.mouse_shape`, `t.status_display`, `t.width_px/height_px`.
- Viewport: `t.scroll_viewport(ScrollViewport::{Top,Bottom,Delta(n)})`. Screen switching: `switch_screen(Key) -> Option<old Key>`, `switch_screen_mode(SwitchScreenMode::{M47,M1047,M1049}, enabled)`.
- `print_attributes()` returns a `String` (DECRQSS SGR reply body) instead of writing into a buffer; `set_pwd(&[u8])`.
- Termio's own handler can call the same `Terminal` methods; it should handle side-effect actions (queries, title, clipboard, bell, APC/DCS) itself and delegate the rest, mirroring `stream_readonly::Handler::vt`.
- Tests: Zig tests that mixed `s.nextSlice` and direct `t.` calls use `feed(&mut t, bytes)` (fresh stream per call) or `t.vt_stream()` + `s.handler.terminal`.

## TODO(integration)
- `Terminal::kitty_graphics(cmd)` is a stub returning `None` (Zig `kitty.graphics.execute`); `kitty_images_delete_all` (erase display complete/scroll complete) only sets `kitty_images.dirty`; kitty image storage limit on lazily-created screens uses `Options::DEFAULT`. `kitty_images.dirty = true` hooks are in place at every Zig location (index, scroll_up, insert/delete lines, switch_screen, Screen::scroll).
- tmux control-mode viewer: not required by Terminal/stream_readonly; not ported here.
- `clone_partial_row_between` local helper (pagelist request).
- Oniguruma: `onig` crate lacks the retry-limit-in-search setter; only retry-limit-in-match is set.
