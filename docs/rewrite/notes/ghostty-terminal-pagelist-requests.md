# Requests for page.rs / page_list.rs (from the Screen port)

1. **Pin-based iterators** (Zig `PageList.zig` ~5100-5170): `Pin::{row_iterator, cell_iterator, page_iterator, prompt_iterator}(direction, limit: Option<Pin>)`.
   The limit pin is inclusive and applies at row granularity. Screen uses a local copy in `pin_iter.rs`.
2. **`PageList::set_tracked_pin` validity**: it rejects pins with `y >= node.size.rows`. Zig writes the pin raw; `Screen::split_for_capacity` temporarily restores the cursor to the pre-split node, which may have shrunk. Worked around with a clamped stand-in.
3. **Cross-page row clone**: Zig `Page.cloneRowFrom(other_page, ...)` between two nodes of one list needs two page borrows. Screen implements it with `export_cell`/`import_cell` (`Screen::clone_row_between`). A `PageList::clone_row_between(src_pin, dst_pin)` would be better.
4. **String allocator probing**: `Page.string_alloc` is private; Zig `cursorSetHyperlink` probes it before growing. Screen instead re-adds the cursor hyperlink after `increase_capacity`. Also `insert_hyperlink` allocates strings before checking set dedup, so near-full pages can fail needlessly.
5. **`fixup_viewport` must handle `Viewport::Top`** after `erase_rows` (Zig PageList.zig ~3043-3070). Worked around in `Screen::erase_rows`.
6. **Resize reflow / no-reflow fewer rows** must drop trailing blank rows, including rows holding only bg-color cells (Zig `resizeCols`/`resizeWithoutReflow`). Three Screen tests are `#[ignore = "pagelist: ..."]`:
   `screen_resize_no_reflow_less_rows_trims_blank_lines`, `screen_resize_less_cols_with_reflow_but_row_space`, `screen_resize_less_cols_to_wrap_wide_char`.
7. `PageList::reset` and `erase_rows` return `Result`; `erase_rows` with an empty history start point returns `InvalidCoordinate` (Zig would assert). Screen guards/ignores.
