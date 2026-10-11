# ghostty-terminal page memory model — partial port

This is **not a completed Ghostty PageList parity port**. The core structures and
many operations are implemented, but the remaining PageList cases below must be
ported and used to drive corrections before Screen/Terminal relies on exact Zig
reflow, allocation, and viewport behavior. In particular, passing the current
crate tests does not establish equivalence of page boundaries under reflow.

No version bump or commit was made. Only `ghostty-vt` is required; no unicode,
datastruct, or config dependency was introduced. Reference files were read only.

## Module and test inventory

| Zig source | Rust module | Source cases represented | Extra Rust tests | Unported / skipped |
|---|---|---:|---:|---:|
| size.zig | size | 1 / 5 | 0 | 4 pointer-only tests skipped |
| point.zig | point | 0 / 0 | 0 | 0 |
| style.zig | style | 37 / 37 | 0 | 0 |
| hyperlink.zig | hyperlink | 0 / 0 | 0 | 0 |
| ref_counted_set.zig | ref_counted_set | 0 / 0 | 2 | 0 |
| page.zig | page | 46 / 46 | 0 | 0 |
| PageList.zig | page_list | 151 / 202 | 2 | 50 unported, 1 skipped |

Total: **239 passing Rust tests**, consisting of 235 source-named regression
cases and four added regressions. Some PageList cases use smaller equivalent
fixtures and consolidate their assertions; matching a test name is not a claim
that all incidental assertions or allocation paths in that Zig case were copied.
Style formatter fixtures retain their reference expected strings. All 17
semantic-highlight cases retain the reference grid setup and expected endpoints.

Intentional skips:

- `Offset ptr u8`, `Offset ptr structural`, `getOffset bytes`, `getOffset structs`:
  raw-address/base-pointer machinery was replaced with owned storage and logical
  cell indices. `Offset<T>` is a typed u32 logical offset; it does not dereference
  page-relative pointers.
- `PageList init error`: Zig tripwire/allocator failure injection has no Rust
  counterpart here. Fallible grid reservations return errors, but allocation
  failure coverage is incomplete.
- `bitmap_allocator.zig` (21 tests) and `hash_map.zig` (22 tests) are not ported
  as modules, as requested. Their logical accounting is implemented in page.rs.

## Implemented structures and behavior

- Cell and Row are transparent u64 wrappers, Copy + Clone, exactly eight bytes.
  Cell preserves the Zig bit offsets: tag 0, payload 2, style ID 26, wide 42,
  protected 44, hyperlink 45, semantic content 46. Row stores a cell index in its
  low 32 bits and uses the same flag positions as Zig. Row cell indices are not
  byte offsets; rotating rows moves metadata and preserves auxiliary-map keys.
- Page owns row/cell Vecs, a deduplicated style set, grapheme data and allocation
  maps, a hyperlink map/set, and logical string/grapheme chunk allocators.
- `Page::layout` preserves the reference 64-bit Linux layout calculation,
  including the unusual bitmap allocator formula `aligned_cap * chunk_size`,
  bitmap rounding to 64 chunks, map headers even at zero capacity, set 13/16
  load factor and reserved ID zero, and 4096-byte final rounding. A standalone
  Zig ABI probe confirmed Style item size/alignment 36/4, hyperlink item 48/8,
  and offset-slice 16/8. The layout does not report actual Rust heap overhead.
- Logical allocation tracks contiguous chunks, including temporary allocation
  of a larger grapheme before freeing its old chunks. Grapheme and hyperlink
  maps enforce separate entry limits. Dead hyperlink entries retain their
  string chunks until their set slot is reclaimed.
- Page provides styles, graphemes, hyperlinks, copy/clone, partial-row copying,
  moves, swaps, clearing, exact row capacity, and debug/test verification.
  Unmanaged partial-row copies use a slice copy without per-cell maps or
  allocation. Default styles bypass set lookup.
- PageList uses `Vec<Option<Node>>`, a free-index list, generation-bearing
  NodeId, and a separate reusable standard-page pool. It accounts substandard
  pages as standard pool allocations. Growing prunes whole history pages while
  protecting the active rows. Reused node indices receive new generations.
- PageList has list-owned tracked handles, viewport scrolling, byte/row totals,
  splitting, capacity growth, compaction, erasure, reset, history clearing,
  cloning, row/column resize, reflow, prompt iteration/highlighting, and forward
  or reverse page/row/cell iterators. Iterators also implement DoubleEndedIterator
  where appropriate. PromptIterator follows continuation handling from Zig.

## Important deviations / remaining work

1. **Reflow and wider no-reflow resize rebuild a snapshot of rows into new pages.**
   They do not yet implement Zig's incremental ReflowCursor and page backfill
   algorithm. New page auxiliary capacities are sized from encountered content,
   instead of following the exact sequence of capacity doubling and splitting.
   Page boundaries, intermediate allocations, pruning, and maximum-size behavior
   can differ. This is the most important unfinished work.
2. Snapshot reflow is linear in all cells and allocates temporary row vectors,
   copies managed values, and scans tracked locations while copying. It is not
   ready for the intended terminal hot-path performance requirements. Point
   conversion and viewport offsets traverse the list; the Zig active/viewport
   caches have not been reproduced.
3. Cursor preservation during combined row/column resize is incomplete. Basic
   tracked cursor positions, blank-cell clamping, trailing blank trimming, and
   several wide/spacer cases are tested, but cross-page cursors, old/new wrap
   counts, viewport remapping under reflow, and all combinations need the
   remaining reference cases.
4. The HashMap-backed RefCountedSet preserves zero reservation, bounded live
   slots, sequential IDs, trailing dead-slot trimming, `add_with_id`, checked
   reference counts, and OutOfMemory/NeedsRehash errors. It does **not** reproduce
   Robin Hood probe-dependent dead-ID reuse or the 32-probe failure threshold.
   Thus exact reused IDs and the point at which NeedsRehash occurs can differ.
   Hash values are Rust implementation details; Zig Style/Wyhash output is not
   reproduced. Style implements Hash/Eq for the Rust deduplication map.
5. Direct Page clone uses Rust Clone. Some auxiliary temporary allocations and
   owned-value clones still use infallible standard collection operations.
   Actual process allocator exhaustion is not completely surfaced as Result;
   logical capacity exhaustion is. Allocation-failure atomicity has not been
   fully audited. There is no unsafe code or todo!/unimplemented! path.
6. The row-range eraser supports arbitrary start ranges, while Zig's tested
   eraseRows use is primarily prefix/history/active erasure. Exact destination
   pins for arbitrary whole-page removal in the middle need further parity work.
7. `style::BoldColor` is a small local policy enum because terminal must not
   depend on config. TODO(integration): convert the configuration type at the
   Screen boundary. Rgb/Palette/Underline are reused from ghostty-vt.
8. Logical layout is for 64-bit Linux with a 4096-byte minimum page size. Other
   ABIs are not implemented. Pool reuse reuses Pages with matching capacities;
   it does not share a mmap allocator between independent PageLists.

## API overview for Screen / Terminal agents

### Coordinate and lifetime model

`Point::{Active,Viewport,Screen,History}(Coordinate { x: u16, y: u32 })`
selects the coordinate frame. `Point::new(Tag, Coordinate)` is useful for dynamic
frames. `pin(point)` and `point_from_pin(tag, pin)` return Option for bounds errors.
`get_top_left` and `get_bottom_right` return None for an empty region such as
history before the first scroll.

`Pin { node: NodeId, y: u16, x: u16, garbage: bool }` is Copy and contains no
references. Pass the owning list explicitly to its access and movement methods:
`p.row_and_cell(&list)`, `p.style(&list)`, `p.grapheme(&list)`,
`p.cells(&list, CellSubset::All)`, `p.up(&list, n)`, `p.right_wrap(&list, n)`, etc.
`CellSubset::Left` and `Right` both include the pin column. Vertical overflow
methods return `Overflow::{Offset,Overflow { end, remaining }}`. `diagonal` uses
signed deltas and clamps columns. `before`/`is_between` use row-major order.

Untracked pins must be reacquired after content-moving mutations. For a cursor,
selection endpoint, or saved location, call `track_pin(pin) -> Result<TrackedPin>`.
Read its current Copy value using `tracked_pin(handle)`; change its position with
`set_tracked_pin(handle, pin)`. Release with `untrack_pin(handle)`. Handles are
specific to the owning list. Recycled handles have a new generation, so releasing
an old handle does not release its successor. Pruning/reset mark external pins
as garbage; ordinary erasure remaps them without setting garbage. The list owns
one internal tracked viewport handle, included in `count_tracked_pins()`.

### Reading and writing cells

`PageList::get_cell(point)` returns a Copy Cell, and `get_cell_mut(point)` returns
a direct mutable reference. `node(id)` / `node_mut(id)` expose the page.
`Page::get_row(y)`, `get_cells(y)`, `get_cells_mut(y)`, and `cell_index(x,y)` use
logical row coordinates and preserve O(1) row access even after row rotation.
Auxiliary lookup methods take a **cell index** returned by cell_index, not a
column or pointer. The vectors include unused capacity; iteration should respect
`page.size`, not the vector lengths.

Construct plain text with `Cell::init(codepoint)`. Set flags with `set_wide`,
`set_protected`, `set_semantic_content`, etc. `set_codepoint` changes only the
payload; use `set_content_tag` when changing between text and background colors.
For managed cells, clear the old data with page operations before assigning a
new raw Cell. Direct writes must maintain row flags and dirty tracking.

- `set_style(x,y,Style)` handles style add/release and marks the row styled.
  Default style is ID 0; `page.style(cell)` resolves it without map access.
- `append_grapheme(x,y,cp)` and `set_graphemes(x,y,&extra)` store only codepoints
  beyond the cell's first codepoint. Set the first nonzero codepoint beforehand.
- `insert_hyperlink(Hyperlink)` returns an already-retained ID. Attach it with
  `set_hyperlink(x,y,id)`, which consumes that reference without incrementing it.
  For another cell, call `page.hyperlink_set.r#use(id)` first. If attachment fails,
  release the retained ID. `clear_hyperlink(index)` releases the cell reference.
- `clear_cells(y,start,end,protect)` uses half-open columns. `protect=true` is a
  Rust convenience that skips protected cells. Move and swap helpers transfer
  auxiliary ownership and avoid changing reference counts for moved values.

`row.set_dirty(true)` / `pin.mark_dirty(&mut list)` support direct writes;
`list.clear_dirty()` resets page and row flags. `is_dirty` checks both levels.
Conservative managed-row flags may be true even after their last value is cleared.
Use the explicit `update_row_*_flag` methods when a precise flag is needed.

### Errors and capacity recovery

`page::Error` includes separate style set, grapheme allocator/map, hyperlink
set/map, and string exhaustion variants. `ref_counted_set::AddError` distinguishes
OutOfMemory from NeedsRehash, plus invalid IDs/counts for safe API misuse handling.
`RefCountedSet::add_with_id(value,id)` returns None when the requested ID was used,
otherwise Some(actual_id). `get` and `lookup` do not retain values.

`increase_capacity(node, Some(IncreaseCapacity::{Styles,GraphemeBytes,
HyperlinkBytes,StringBytes}))` replaces a page with increased limits and remaps
tracked pins; None rebuilds the same capacity to reclaim dead IDs.
`adjust_capacity` is an alias. OutOfSpace means doubling/page-size limits prevent
further growth; `split(pin)` can divide at a row boundary. Splitting row zero is
otherwise a no-op, and a single-row page returns OutOfSpace. `compact(node)`
returns Some(replacement) only when an oversized page can shrink.
Always use the returned NodeId, or resolve a tracked pin again after replacement.

### List mutations and iteration

`grow()` adds one row, returning Some(new_node) if another page was needed.
`scroll(Scroll::{Active,Top,Row,DeltaRow,DeltaPrompt,Pin})` only changes viewport.
`scrollbar()` reports row counts and hides physical history at max_size zero.
`scroll_clear()` moves the nonempty active rows into history.

`erase_rows(top, optional_bottom)` removes inclusive rows and ensures active height.
`erase_row(point)` shifts the remainder of the screen while retaining page sizes.
`erase_row_bounded(point,limit)` shifts at most limit following rows.
`erase_cells_bounded(start_pin,end_pin,left,right)` is an extra rectangular helper.
`reset()` clears content and invalidates tracked contents; `clear_history()`
preserves the active area.

`resize(Resize { cols, rows, reflow, cursor })` takes optional new dimensions and
an optional active `Cursor { x,y }`. Reflow defaults to true. Cursor is input
context; track the actual cursor pin separately to read back its new position.
Consult the limitations above before relying on complex resize behavior.

`clone_range(top,bottom)` clones inclusive **whole rows**, ignoring x, pads to the
active height, and returns `(PageList, HashMap<old_handle,new_handle>)`.
`try_clone()` clones the whole screen without retaining external tracked pins.
Clone viewport defaults to Active; dirty information is retained by rows.
Page itself implements Clone for an independent full copy; its inherent
`clone_from(source,start,end)` copies a half-open row range into existing capacity.
`clone_partial_row_from` accepts source/destination row and column plus length;
`clone_cells_within` handles same-page copies without conflicting borrows.

`page_iterator`, `row_iterator`, `cell_iterator` take Direction, start Point,
and optional inclusive end Point. End None uses the frame's bottom-right.
Reverse direction traverses the same bounds backwards. PageIterator yields
`Chunk { node,start,end }` with half-open page-local rows. Iterators borrow the
list immutably; collect Copy pins/chunks or end iteration before mutating it.
`prompt_iterator` finds semantic prompt starts; `highlight_semantic_content`
returns an optional inclusive `(start_pin,end_pin)` for a prompt's Prompt, Input,
or Output region.

`Page::verify_integrity` and `PageList::verify_integrity` are available in debug
and test builds, as are assert_integrity helpers. Nestable pause methods support
batched low-level updates. Verification is explicit; raw mutable access can
break invariants until the caller completes its update.

## Remaining PageList source tests

The following 50 cases are **unported**, rather than justified skips. They include
important production behavior and should drive the next implementation pass.
- `PageList grow allows exceeding max size for active area`
- `PageList grow prune required with a single page`
- `PageList scroll delta row back without space preserves active`
- `PageList: jump zero prompts`
- `Screen: jump back one prompt`
- `Screen: jump forward prompt skips multiline continuation`
- `PageList eraseRows invalidates viewport offset cache`
- `PageList eraseRow invalidates viewport offset cache`
- `PageList eraseRowBounded invalidates viewport offset cache`
- `PageList eraseRowBounded multi-page invalidates viewport offset cache`
- `PageList eraseRowBounded full page shift invalidates viewport offset cache`
- `PageList eraseRowBounded exhausts pages invalidates viewport offset cache`
- `PageList promptIterator right_down continuation at start`
- `PageList promptIterator right_down with prompt before continuation`
- `PageList promptIterator right_down limit inclusive`
- `PageList promptIterator left_up limit inclusive`
- `PageList erase resets viewport to active if moves within active`
- `PageList erase resets viewport if inside erased page but not active`
- `PageList erase resets viewport to active if top is inside active`
- `PageList resize (no reflow) less rows cursor on bottom`
- `PageList resize (no reflow) less rows cursor in scrollback`
- `PageList resize (no reflow) more rows contains viewport`
- `PageList resize (no reflow) more cols with spacer head`
- `PageList resize (no reflow) grow cols fast path with spacer head`
- `PageList resize (no reflow) more cols forces less rows per page`
- `PageList resize more rows and cols doesn't fit in single std page`
- `PageList resize (no reflow) more cols forces smaller cap`
- `PageList resize (no reflow) more rows adds blank rows if cursor at bottom`
- `PageList resize reflow invalidates viewport offset cache`
- `PageList resize reflow more cols creates multiple pages`
- `PageList resize reflow more cols wrap across page boundary`
- `PageList resize reflow more cols wrap across page boundary cursor in second page`
- `PageList resize reflow less cols wrap across page boundary cursor in second page`
- `PageList resize reflow more cols cursor in wrapped row`
- `PageList resize reflow more cols cursor in not wrapped row`
- `PageList resize reflow more cols cursor in wrapped row that isn't unwrapped`
- `PageList resize reflow exceeds hyperlink memory forcing capacity increase`
- `PageList resize reflow exceeds grapheme memory forcing capacity increase`
- `PageList resize reflow exceeds style memory forcing capacity increase`
- `PageList resize reflow more cols unwrap wide spacer head`
- `PageList resize reflow more cols unwrap wide spacer head across two rows`
- `PageList resize reflow less cols cursor goes to scrollback`
- `PageList resize reflow less cols copy kitty placeholder`
- `PageList resize reflow more cols clears kitty placeholder`
- `PageList resize reflow wrap moves kitty placeholder`
- `PageList resize reflow grapheme map capacity exceeded`
- `PageList resize grow cols with unwrap fixes viewport pin`
- `PageList grow reuses non-standard page without leak`
- `PageList grow non-standard page prune protection`
- `PageList resize (no reflow) more cols remaps pins in backfill path`

## Quality gate

Final gate (2026-10-11):

- `cargo fmt -p ghostty-terminal`: passed; only the documented nightly-option
  warnings from workspace rustfmt configuration.
- `cargo build -p ghostty-terminal`: passed.
- `cargo test -p ghostty-terminal --quiet`: 239 passed, 0 failed, 0 ignored.
- `cargo clippy -p ghostty-terminal --all-targets -- -D warnings`: passed.
- `git diff --check -- crates/ghostty-terminal docs/rewrite/notes/ghostty-terminal-pagelist.md`:
  passed for tracked changes.

The early missing `ghostty-vt::stream` dependency file was resolved by concurrent
workspace work; no change to that crate was needed here. The terminal Cargo.toml
already had local changes at the beginning of this task and was not edited here.
