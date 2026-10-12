# TODO

## Smooth resize with very large scrollback

**Symptom:** with a large `scrollback-limit` (e.g. 1 GiB) and a pane holding a
lot of history, resizing the window or adding a split freezes the UI for a
moment.

### Why it freezes

1. **Every resize rewrites the whole scrollback.** Any column change (window
   drag, new split) runs `PageList::resize_cols` → `reflow_pages`
   (`crates/ghostty-terminal/src/page_list.rs`), which rewrites every page cell
   by cell into new pages. With ~1 GiB of history that is 100M+ cells and
   roughly 2 GiB of memory traffic: a single synchronous O(scrollback) job.
2. **It holds the terminal lock the whole time.** `Termio::resize`
   (`crates/ghostty-termio/src/termio.rs`) runs the reflow inside
   `lock_terminal()`, so:
   - the renderer thread can't build a frame and that pane stops updating;
   - the GTK main thread blocks whenever it touches that terminal. Adding a
     split changes focus, and `focus_callback`
     (`crates/ghostty-core/src/surface/mod.rs`) does
     `self.lock_terminal().flags.focused = …`. Mouse motion, the mouse cursor
     shape and key handling also take the lock. So the whole window freezes,
     not just the pane.
3. **A drag means many full reflows in a row.** Resizes are only coalesced
   for 25 ms (`COALESCE_MIN_MS`, `crates/ghostty-termio/src/thread.rs`). Each
   coalesced step reflows everything, so during a drag the IO thread holds the
   lock almost all the time.
4. **The inner loop does extra work.** `reflow_row` and `remap_cell` scan
   every tracked pin for every cell, which is O(cells × pins). Every cell also
   goes through `write_cell`, even on plain short lines that would fit
   unchanged.

Upstream Zig Ghostty has the same design (including the per-cell pin scan in
`reflowRow`), so this was inherited faithfully, not introduced by the rewrite.

**Key point:** making the reflow faster helps but can't get to "smooth at all
times". Even at memory bandwidth, rewriting ~1 GiB takes 100 ms or more, far
beyond a 16 ms frame. To truly never stall, the full reflow must not happen
under the lock in one go.

### Options, from cheapest to most thorough

- [ ] **0. Resize benchmark** in `ghostty-bench` (fill N GiB, resize the
  columns) to get real numbers before and after each change.

- [ ] **A. Speed up the reflow** (low risk, contained in `page_list.rs`)
  - [ ] Skip the pin scan for pages with no tracked pins (nearly all
    scrollback), or pre-sort pins per page. This removes the O(cells × pins)
    factor.
  - [ ] Fast path for rows that fit at the new width and aren't wrapped: copy
    the whole row at once (cells plus style/grapheme/hyperlink refs) instead
    of cell by cell.
  - [ ] Page-level fast path: when shrinking, a page with no wrapped rows
    whose content already fits keeps its storage and just gets a smaller
    `size.cols`, with no copy.
  - Likely several times faster, but only the benchmark will tell. Still
    won't be frame-time at 1 GiB.

- [ ] **B. Stop the GTK main thread from blocking on the terminal lock**
  (moderate, independent of A)
  - [ ] Hot main-thread paths shouldn't need the lock: the focus flag via an
    IO message or atomic; the mouse-reporting mode and mouse shape cached in
    atomics.
  - Result: the window, other panes, split creation and dragging stay smooth.
    Only the pane being reflowed briefly shows its last frame.
  - Probably the biggest perceived improvement for the effort.

- [ ] **C. Defer the history reflow until the drag settles** (moderate)
  - During a drag, reflow only the active screen. Reflow history once the
    size has been stable for about 100–200 ms.
  - Avoids repeated full reflows mid-drag; one stall at the end remains unless
    combined with D.

- [ ] **D. Incremental, chunked reflow** (the real fix, biggest change)
  - Reflow from the newest content toward the oldest, a few milliseconds of
    work per lock hold, releasing the lock in between so the renderer and pty
    keep running.
  - Requires the page list to allow pages of different widths temporarily, so
    the renderer, selection, search and scrollbar must cope with "not yet
    reflowed" rows. If another resize arrives mid-way, the job just switches
    to the new target width.
  - Variant: only reflow the last N rows eagerly and do older history in the
    background, or when it's scrolled into view.

- [ ] **E. Config escape hatch** (trivial)
  - A setting such as "reflow only the last N MB/rows of scrollback" and leave
    older history unreflowed. Useful as a stopgap but changes behaviour.

### Recommended order

1. **B + A first.** They're mostly contained, low risk, and remove the
   whole-window freeze plus most of the per-pane stall.
2. **Then C** for live drags.
3. **D only if the remaining single-pane hitch still matters.** It's the only
   option that fully guarantees smoothness, but it's a real design change and
   diverges from upstream.
