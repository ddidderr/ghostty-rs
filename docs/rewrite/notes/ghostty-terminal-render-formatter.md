# ghostty-terminal: render, highlight, formatter

## Ported
| Zig | Rust | tests |
|---|---|---|
| render.zig | `render.rs` | 15 of 15 (+2 extra) |
| highlight.zig | `highlight.rs` | 0 in Zig (+2 extra) |
| formatter.zig | `formatter.rs` (+ `formatter/tests/p1..p4.rs`) | 100 of 100 (p1 33, p2 26, p3 26, p4 15) |

`Screen::selection_string[_map]`, `dump_string[_alloc[_unwrapped]]` (and so `Terminal::plain_string[_unwrapped]`) now delegate to
`formatter::ScreenFormatter` (the minimal `plain_format` in screen.rs is gone).

## RenderState API (for the renderer / font shaper)
```rust
use ghostty_terminal::render::{RenderState, Dirty};
let mut state = RenderState::new();        // Zig `.empty`; keep it across frames (buffers are reused)
state.update(&mut terminal)?;              // hold the renderer mutex; consumes terminal.flags.dirty + screen.dirty
```
`update` follows Zig exactly: full redraw (`Dirty::Full`) if the active screen key, terminal dirty flags, screen dirty flags,
size or viewport pin changed; otherwise only rows whose page/row dirty flag is set are rebuilt (`Dirty::Partial`), clearing row/page dirty.
The renderer must reset `state.dirty = Dirty::False` (and each `row.dirty = false`) after drawing; `update` only ever raises them.

Fields: `rows, cols, colors {background, foreground, cursor: Option<Rgb>, palette}` (fg/bg already swapped for reverse-colors mode),
`cursor {active: Coordinate, viewport: Option<CursorViewport{x,y,wide_tail}>, cell, style, visual_style, password_input, visible, blinking}`,
`row_data: Vec<Row>` (length == rows), `dirty`, `screen: ScreenKey`, `viewport_pin`.
`Row { pin, raw: page::Row, cells: Cells, dirty, selection: Option<[x0,x1]>, highlights: Vec<Highlight{tag, range:[x0,x1]}> }`.
`Row.pin` is a plain (untracked) `Pin`; only valid until the terminal changes.

`Cells` is struct-of-arrays (Zig `MultiArrayList(Cell)`): `raw: Vec<page::Cell>` always `cols` long; `grapheme` / `style` are EITHER empty
(row has no managed memory => no graphemes, default styles) or `cols` long. Use the accessors: `cells.get(x) -> Cell{raw, grapheme: &[u32], style}`
(same shape as the font crate's `RunCell`: `raw`, `grapheme` = extra codepoints, `style`), `cells.grapheme(x)`, `cells.style(x)`, `cells.iter()`.
As in Zig, bg-color-only cells (`ContentTag::BgColor*`) get their `style.bg_color` filled only on rows with managed memory; otherwise read the raw cell.

Other methods:
- `update_highlights_flattened(tag: u8, &[highlight::Flattened])` (infallible; appends to `row.highlights`, marks rows dirty; does not clear old ones).
- `string(&mut String, Option<&mut Vec<Coordinate>>)` – unwrapped viewport text, \0 for empty cells, `\n` after non-wrapped rows (map: newline => x = cols).
- `link_cells(&PageList, Coordinate) -> HashSet<Coordinate>` – DEVIATION: needs the page list (`&terminal.screen().pages`) because Rust pins are
  node ids, not page pointers. Must be called with the terminal unchanged since `update`.
Selection rows are cached (`selection_cache`) exactly as in Zig.

## highlight.rs
`Untracked{start,end}`, `Tracked{start,end: TrackedPin}` (handles into `screen.pages`; `Tracked::init(&mut Screen, ..)`, `deinit(&mut Screen)`, `untracked(&PageList)`),
`Flattened{chunks: Vec<Chunk{node,serial,start,end}>, top_x, bot_x}` with `init(&PageList, start, end)`, `start_pin()`, `end_pin()`, `untracked()`.
Zig's `Flattened.init` has a typo (`.end_x`) in unreferenced code; the port sets `bot_x = end.x`.
`highlight::pin_page_iterator(&PageList, start, end)` is the local equivalent of `Pin.pageIterator(.right_down, limit)`.

## formatter.rs
- Writers are `&mut dyn std::io::Write`; every formatter has `format_string(self) -> io::Result<String>` as a convenience. Formatters consume `self` on `format`.
- `Options::{plain(), vt(), html()}` (+ pub fields; `palette: Option<Palette>` owned, `codepoint_map: Option<Vec<CodepointMap>>`, default `Some(vec![])`).
- Formatters: `TerminalFormatter::new(&Terminal, opts)` (extra: `TerminalExtra::{NONE,STYLES,ALL}`), `ScreenFormatter::new(&Screen, opts)` (`ScreenExtra`, `Content::{None, Selection(Option<Selection>)}`),
  `PageListFormatter::new(&PageList, opts)`, `PageFormatter::new(&Page, &Options)` (+ `format_with_state` -> `TrailingState`).
- Pin maps: `pin_map: Option<&mut Vec<Pin>>`; page formatter `point_map: Option<&mut Vec<Coordinate>>`. Instead of Zig's discarding writers, extras are rendered into a scratch buffer whose length is used for the map.
- Quirk kept from Zig: blank-cell point-map entries are appended in reverse order (see PageFormatter); `</div>`/`</a>`/`ESC[0m` closers map to the previous byte's coordinate.

## Skipped / deviations
- No Zig tests skipped. Allocator-failure paths do not exist in Rust.
- Hyperlink URI bytes are escaped per byte (as Zig `writeCodepoint(byte)`), so non-ASCII URI bytes become numeric entities.
