# Requests for page.rs / page_list.rs (from the Screen port)

All requests resolved and verified in the 2026-10-11 integration pass.

1. **Resolved — pin-based iterators.** `Pin::{row_iterator, cell_iterator,
   page_iterator, prompt_iterator}` take the owning `&PageList`, direction, and
   an inclusive optional limit pin. Limits apply to whole rows; cell iteration
   begins at the starting column. Construction uses pins directly, without
   point conversion or a list scan. Screen, Terminal, and Selection use these
   methods. Public `pin_iter` functions/types remain as compatibility wrappers.
2. **Resolved — tracked-pin restoration.** `set_tracked_pin` validates the
   handle and node lifetime, allowing a temporarily out-of-range row during
   split recovery. Screen restores the exact old pin, with no clamped stand-in.
   Cursor dirtying accesses the still-allocated row, matching Zig's cached
   `page_row`. The maximum-style page-split regression passes.
3. **Resolved — cross-page row clone.** `PageList::clone_row_between(src, dst)`
   borrows the two pages directly and calls `Page::clone_row_from`. Screen's
   export/import row snapshot helper is removed.
4. **Resolved — string allocation and hyperlink deduplication.** Screen probes
   `Page::can_allocate_string` before capacity growth in `cursor_set_hyperlink`,
   following Zig's string-growth/hyperlink-growth/retry order. The probe frees
   its temporary reservation in Rust; the page is then rebuilt. The cursor-link
   recovery workaround is removed. `insert_hyperlink` retains existing live
   entries before allocating strings; a full-allocator regression covers this.
5. **Resolved — Top viewport after erasure.** `fixup_viewport` handles `Top`
   inside PageList; Screen has no viewport fixup. Resize still only fixes a
   pinned viewport, preserving Zig's `Top` behavior (covered by the existing
   resize-no-reflow-more-rows-contains-viewport test).
6. **Verified resolved — trailing blank rows during resize.** All three tests
   are enabled and pass: `screen_resize_no_reflow_less_rows_trims_blank_lines`,
   `screen_resize_less_cols_with_reflow_but_row_space`, and
   `screen_resize_less_cols_to_wrap_wide_char`.
7. **Resolved — reset/erase error handling.** Existing public `Result` signatures
   remain for backwards compatibility. Erasing empty history is a successful
   no-op. Screen's extra history guard and error swallowing are removed;
   valid internal erasure/reset calls assert success as in Zig.

- **Verified resolved (T3/p4):** `erase_row_bounded` clears the blanked row's
  managed flags; `terminal_index_bottom_of_scroll_region_clear_hyperlinks`
  is enabled and passes.
- **Verified resolved (T3/p8):** pending-wrap reflow retains the correct active
  rows; `terminal_resize_with_wraparound_on` is enabled and passes.
- **Resolved (T3/p1):** `set_graphemes` and `append_grapheme` preserve row dirty
  state, including allocation growth. Terminal's save/restore workaround is
  removed; the VS16 test and a direct append regression pass.
- **Resolved (T3/terminal):** `PageList::clone_partial_row_between(src, dst,
  left, right)` directly borrows distinct pages and delegates managed-memory
  and row-metadata handling to `Page::clone_partial_row_from`. Terminal's
  export/import implementation is removed. The regression checks graphemes,
  hyperlinks, untouched columns, dirty/wrap flags, and Kitty placeholder flags.

## API changes

Additive methods: the four Pin iterators and both PageList row-copy helpers.
Existing iterator wrappers and `Result` signatures are retained. No downstream
caller edits were needed.

## Validation

`cargo test -p ghostty-terminal`: 1252 unit tests and 1 documentation test pass,
0 failed, 0 ignored. Clippy with `-D warnings`, rustfmt, and the combined build
of `ghostty-font`, `ghostty-renderer`, and `ghostty-termio` pass.
