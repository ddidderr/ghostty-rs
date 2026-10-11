# ghostty-terminal: Screen, ScreenSet, Tabstops, Selection

## Ported
| Zig | Rust | tests |
|---|---|---|
| Screen.zig | `screen.rs` (+ `cursor.rs` for cursor.zig `Style`) | 183 of 184 (`screen/tests/p1..p5.rs`) |
| ScreenSet.zig | `screen_set.rs` | 1/1 |
| Tabstops.zig | `tabstops.rs` | 4/5 |
| Selection.zig | `selection.rs` | 18/18 |

Skipped: `Tabstops: resize alloc failure preserves state` and `selectionString map allocation failure cleanup` (Zig tripwire failure injection).
All three formerly blocked resize tests are enabled and pass. No ignored tests.
Deps added: ghostty-unicode (width in `test_write_string`), ghostty-datastruct (unused so far), log.

## Deviations
- Cursor `page_pin` is a `TrackedPin`; `page_row`/`page_cell` are accessors: `cursor_pin()`, `cursor_row()/_mut()`, `cursor_cell()/_mut()`, `cursor_page()/_mut()`, `cursor_row_cells()`, `cursor_cell_left/right[_mut](n)`.
- Errors are `page::Error` (re-exported `screen::Error`). `Screen::reset()` returns `Result`.
- `Screen::new(Options)`; `Options::new(cols, rows, max_scrollback)` (0 = no scrollback).
- `clone` is `clone_screen(top, bot)`. `cursor_copy(&Cursor, hyperlink: bool)` copies the hyperlink from the other cursor's heap copy.
- `append_grapheme(x, cp)` takes a column of the cursor row. `clear_cells(node, y, x0, x1)` takes a half-open column range.
- Selection methods take `&PageList` (`&screen.pages`) since tracked bounds are handles: `sel.start(&pages)`, `top_left`, `order`, `contains`, `contained_row`, `ordered`, `eql(&pages, &other)`, `adjust(&mut pages, adj)`, `track(&mut pages)`, `deinit(&mut pages)`. `Selection` is `Copy`.
- `kitty_images` owns `kitty::graphics_storage::ImageStorage`: initialization applies the configured limit; reset releases placement pins and restores default storage with `dirty = true`; Drop releases storage before pages. `clone_screen` does not clone images.
- `StringMap` re-exports the real `string_map::StringMap`.
- Selection/dump strings use the real `formatter::ScreenFormatter`.
- Tabstops::unset XORs like Zig (unsetting an unset stop sets it).
- `cursor_set_hyperlink` follows the reference allocator probe and capacity-growth order. PageList owns viewport repair and cross-page row copying; Screen uses native Pin iterators and exact cursor restoration after page splitting. No PageList workarounds remain.

## API overview for Terminal
`Screen` fields: `pages, no_scrollback, cursor, saved_cursor, selection, charset, protected_mode, kitty_keyboard, kitty_images, semantic_prompt, dirty`.
Cursor motion: `cursor_up/down/left/right(n)`, `cursor_absolute(x,y)`, `cursor_horizontal_absolute(x)`, `cursor_down_scroll()`, `cursor_scroll_above()`, `cursor_reset_wrap()`, `cursor_mark_dirty()`, `cursor_reload()`.
Editing: `clear_cells`, `clear_unprotected_cells`, `clear_rows`, `erase_rows`, `split_cell_boundary(x)`, `blank_cell()`, `resize(Resize)`, `scroll(Scroll)`, `scroll_clear()`.
Style/links: `set_attribute(sgr::Attribute)`, `manual_style_update()`, `start_hyperlink(uri, id)`, `end_hyperlink()`, `cursor_set_hyperlink()`, `cursor_set_semantic_content(..)`.
Selection: `select(Option<Selection>)`, `clear_selection()`, `select_line/all/word/word_between/output`, `line_iterator`, `prompt_click_move`, `selection_string`.
Test helper: `test_write_string`.

## API changes (integration pass)

`Screen.kitty_images` now exposes the required real `ImageStorage`; its existing
`dirty` field access remains valid. The old standalone `KittyImages` value type
is retained for compatibility, but is no longer used by Screen. Other public
signatures, including `Screen::reset() -> Result`, are unchanged. No renderer,
font, or termio callers needed edits.

## Integration verification (2026-10-11)

All Screen/Terminal TODO and Zig-reference comments were reviewed against the
reference. Remaining TODOs also occur in Zig (capacity-growth recovery and a
selection test note); no integration workaround remains. Lifecycle regressions
cover real image storage, clone isolation, reset, and placement-pin release.
Crate gate: 1252 unit + 1 documentation tests pass, none ignored; clippy with
warnings denied, rustfmt, and the combined font/renderer/termio build pass.
