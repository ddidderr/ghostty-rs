# ghostty-terminal page memory model

The PageList parity pass is complete. All 50 previously missing PageList tests
are ported. Earlier consolidated tests now use the reference dimensions,
capacity-derived page boundaries, operations, and assertions. Shared fixture and
assertion helpers retain the complete source checks, including managed values,
page links, dirty/wrap flags, viewport offsets, and tracked locations.

No public signatures were removed or changed. No version bump or commit was
made. Only the assigned source files and these notes were changed. The Zig
reference remained read-only; no dependency or production module was added.

## API changes

**No breaking public API changes.** Additions to `RefCountedSet`:

- `init_with_hasher(cap, fn(&T) -> u64)` supplies deterministic probe ordering.
- `clear()` resets entries while retaining allocated storage and hash context.

The existing PageList, Page, Point, Pin, TrackedPin, Style, and Hyperlink APIs
remain available to the concurrent Screen port. New copy/reflow helpers are
private or crate-private.

## Module and test inventory

| Zig source | Rust module | Source cases ported | Extra Rust tests | Zig-only skips | Rust total |
|---|---|---:|---:|---:|---:|
| size.zig | size | 1 / 5 | 0 | 4 | 1 |
| point.zig | point | 0 / 0 | 0 | 0 | 0 |
| style.zig | style | 37 / 37 | 0 | 0 | 37 |
| hyperlink.zig | hyperlink | 0 / 0 | 2 | 0 | 2 |
| ref_counted_set.zig | ref_counted_set | 0 / 0 | 7 | 0 | 7 |
| page.zig | page | 46 / 46 | 0 | 0 | 46 |
| PageList.zig | page_list | 201 / 202 | 4 | 1 | 205 |

Total: **298 passing Rust tests**, including 285 source cases and 13 additional
regressions. No functional source test remains unported. Style formatter fixtures
retain the source strings; all 17 semantic-highlight tests retain their grids and
endpoints. Tests with repeated cases use loops without dropping checks.

The guide explicitly permits these Zig-only skips:

- `Offset ptr u8`, `Offset ptr structural`, `getOffset bytes`, `getOffset structs`:
  raw base-address dereferencing is replaced by safe owned storage and logical
  offsets. There are no page-relative Rust pointers to test.
- `PageList init error`: the Zig tripwire allocator/failure-injection harness has
  no counterpart in this safe Rust ownership model. Logical capacity errors are
  covered; process allocator failure injection is not ported.

`bitmap_allocator.zig` and `hash_map.zig` are not new modules. Their bounded
storage/layout responsibilities remain in page.rs as required by the scope.

## Implementation

- Cell and Row remain transparent eight-byte u64 wrappers with the reference
  field positions. Row cell references are logical indices, so rotating row
  metadata preserves auxiliary keys without moving cell contents.
- Page retains the reference logical layout and allocation capacities: 64-bit
  Linux, 4096-byte page rounding, bitmap chunk rounding, map headers, 13/16 set
  load factor, reserved ID zero, and managed-storage item sizes. Logical byte
  accounting intentionally does not measure Rust collection overhead.
- Graphemes use a page-owned codepoint arena and preallocated maps. Appending
  across a chunk boundary allocates the new logical block before releasing the
  old one. Copying/reflowing a grapheme allocates no per-cell Vec. Hyperlink
  duplicates borrow source values, cloning owned strings only for a new
  persistent destination entry. String allocations retain Zig's chunk limits
  and dead-entry lifetimes, including deletion during a failed insertion.
- RefCountedSet uses a Vec-backed Robin Hood table with the reference probe
  ordering, reference-count tie break, backwards-shift deletion, trailing dead-ID
  trimming, encountered dead-ID reuse, preferred IDs, 32-probe ceiling, and
  NeedsRehash/OutOfMemory decision order. The observable living-count behavior
  of a dead preferred-ID upsert is retained and tested. Page style hashes use
  Zig's packed 128-bit fold/integer hash; hyperlink hashes use its Wyhash/tag/
  slice-length encoding. Wyhash block-boundary and hyperlink encoding vectors
  were checked against a standalone Zig probe.
- PageList uses generation-bearing node and tracked-pin handles. Standard-sized
  pages are recycled, oversized pages are released, and byte/row accounting
  follows the reference allocation/pruning rules. Reset releases excess pooled
  history. Split, compact, and capacity growth allocate one destination page and
  clone directly into it.
- Reflow follows resizeCols/ReflowCursor: column/row resize ordering, inherited
  capacities, deferred blank rows, cursor wrap-count preservation, cell-by-cell
  tracked-pin remapping, wrap and continuation metadata, wide/spacer handling,
  graphemes, styles, hyperlinks, and Kitty placeholder metadata. Capacity grows
  in the reference sequence; an exhausted page splits off the current final row
  and retries. Consumed source pages are released immediately. There are no
  temporary row snapshots or per-cell temporary heap allocations in reflow.
- Widening without reflow uses the reference capacity fast path and spacer-head
  fallback, backfills previous pages, remaps backfilled pins, and creates the
  required remaining pages. Shrinking clears truncated managed cells and clamps
  pins. Background-only trailing rows follow the reference trimming behavior.
- Active point lookup starts at the tail, Viewport lookup starts at its anchor,
  and pin-to-point walks only from the requested region's origin. Pin movement
  and ordering traverse adjacent nodes. Pinned viewport offsets are cached and
  updated/invalidated during scrolling, pruning, erasure, and resize. Grow never
  computes an uncached viewport offset. Debug integrity checks validate cached
  offsets against the actual list.
- Row erasure rotates row metadata, copies only cross-page boundary rows, and
  remaps pins in place. Active range erasure regrows the removed rows even when
  history already supplies enough total rows. Clone preserves source capacities,
  chunk boundaries, managed data, dirty flags, and optional pin remapping.

The safe Rust representation replaces mmap/base-pointer machinery with owned
allocations. Allocation addresses are therefore not compared in the nonstandard
page reuse test; node generations and live page/pool membership verify the same
lifetime property. No unsafe code or reachable todo!/unimplemented! path was
introduced. `style::BoldColor` remains the local policy type mandated by the
terminal/config dependency boundary; Screen converts configuration at integration.

## API overview for Screen / Terminal agents

### Coordinate and lifetime model

`Point::{Active,Viewport,Screen,History}(Coordinate { x: u16, y: u32 })`
selects the coordinate frame. `Point::new(Tag, Coordinate)` is useful for dynamic
frames. `pin(point)` and `point_from_pin(tag, pin)` return Option for bounds errors.
`get_top_left(History)` is the screen origin even without history, as in Zig;
`get_bottom_right(History)` returns None when history is empty. Conversion starts
at the selected frame and may continue through subsequent rows in the list, as
in Zig; it does not impose an extra frame-height bound. Active lookup traverses
backward from the tail; Viewport lookup starts at its active, top, or tracked
anchor. `viewport_row_offset(&self)` caches pinned offsets with an atomic value,
preserving the existing immutable API and Send/Sync traits.

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
`clear_cells` updates managed-row flags using the reference full/partial clear
rules. Individual clear helpers may leave conservative flags set; use the
explicit `update_row_*_flag` methods when a precise flag is needed.

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

`erase_rows(top, optional_bottom)` removes inclusive rows. Active erasure regrows
every removed row, preserving history; other frames retain at least active height.
`erase_row(point)` shifts the remainder of the screen while retaining page sizes.
`erase_row_bounded(point,limit)` shifts at most limit following rows.
`erase_cells_bounded(start_pin,end_pin,left,right)` is an extra rectangular helper.
`reset()` clears content and invalidates tracked contents; `clear_history()`
preserves the active area.

`resize(Resize { cols, rows, reflow, cursor })` takes optional new dimensions and
an optional active `Cursor { x,y }`. Reflow defaults to true. Cursor is input
context; track the actual cursor pin separately to read back its new position.
Reflow writes incrementally with a ReflowCursor. Widening without reflow first
backfills the preceding page, then splits into pages using adjusted source
capacities. Both paths remap tracked handles as rows/cells move.

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

## Quality gate

Final gate (2026-10-11):

- `cargo build -p ghostty-terminal`: passed.
- `cargo test -p ghostty-terminal`: 298 passed, 0 failed, 0 ignored; doc tests passed.
- `cargo clippy -p ghostty-terminal --all-targets -- -D warnings`: passed.
- `cargo fmt -p ghostty-terminal`: passed (only workspace nightly-option warnings).
- `git diff --check`: passed.

Remaining functional limitations or missing PageList cases: **none**. The five
Zig-only test skips and ownership/ABI representation choices are documented above.
