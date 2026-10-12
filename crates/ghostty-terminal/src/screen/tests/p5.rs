use ghostty_vt::{color::Rgb, sgr::Attribute as A};

use super::*;
use crate::{page::STD_CAPACITY, style::Color};

fn sel_str(s: &Screen, sel: Selection, trim: bool) -> String {
    s.selection_string(SelectionString { sel, trim })
}

fn sel_active(s: &Screen, a: (u16, u32), b: (u16, u32), rect: bool) -> Selection {
    Selection::new(pin(s, active(a.0, a.1)), pin(s, active(b.0, b.1)), rect)
}

fn sel_screen(s: &Screen, a: (u16, u32), b: (u16, u32), rect: bool) -> Selection {
    Selection::new(
        pin(s, screen_pt(a.0, a.1)),
        pin(s, screen_pt(b.0, b.1)),
        rect,
    )
}

#[test]
fn screen_selection_string_trim_empty_line() {
    let mut s = init(5, 5, 0);
    s.test_write_string("1AB  \n\n2EFGH\n3IJKL").unwrap();

    let sel = sel_screen(&s, (0, 0), (2, 2), false);

    assert_eq!("1AB\n\n2EF", sel_str(&s, sel, true));

    // No trim
    assert_eq!("1AB  \n\n2EF", sel_str(&s, sel, false));
}

#[test]
fn screen_selection_string_soft_wrap() {
    let mut s = init(5, 3, 0);
    s.test_write_string("1ABCD2EFGH3IJKL").unwrap();

    let sel = sel_screen(&s, (0, 1), (2, 2), false);
    assert_eq!("2EFGH3IJ", sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_wide_char() {
    let mut s = init(5, 3, 0);
    let str = "1A\u{26A1}";
    s.test_write_string(str).unwrap();

    {
        let sel = sel_screen(&s, (0, 0), (3, 0), false);
        assert_eq!(str, sel_str(&s, sel, true));
    }

    {
        let sel = sel_screen(&s, (0, 0), (2, 0), false);
        assert_eq!(str, sel_str(&s, sel, true));
    }

    {
        let sel = sel_screen(&s, (3, 0), (3, 0), false);
        assert_eq!("\u{26A1}", sel_str(&s, sel, true));
    }
}

#[test]
fn screen_selection_string_wide_char_with_header() {
    let mut s = init(5, 3, 0);
    let str = "1ABC\u{26A1}";
    s.test_write_string(str).unwrap();

    let sel = sel_screen(&s, (0, 0), (4, 0), false);
    assert_eq!(str, sel_str(&s, sel, true));
}

// https://github.com/mitchellh/ghostty/issues/289
#[test]
fn screen_selection_string_empty_with_soft_wrap() {
    let mut s = init(5, 2, 0);

    // Let me describe the situation that caused this because this
    // test is not obvious. By writing an emoji below, we introduce
    // one cell with the emoji and one cell as a "wide char spacer".
    // We then soft wrap the line by writing spaces.
    //
    // By selecting only the tail, we'd select nothing and we had
    // a logic error that would cause a crash.
    s.test_write_string("\u{1F468}").unwrap();
    s.test_write_string("      ").unwrap();

    let sel = sel_screen(&s, (1, 0), (2, 0), false);
    assert_eq!("\u{1F468}", sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_with_zero_width_joiner() {
    let mut s = init(10, 1, 0);
    let str = "\u{1F468}\u{200D}"; // this has a ZWJ
    s.test_write_string(str).unwrap();

    // Integrity check
    {
        let p = pin(&s, screen_pt(0, 0));
        let cell = s.pin_cell(p);
        assert_eq!(0x1F468, cell.codepoint());
        assert_eq!(Wide::Wide, cell.wide());
        let cps = p.grapheme(&s.pages).unwrap();
        assert_eq!(1, cps.len());
    }

    // The real test
    {
        let sel = sel_screen(&s, (0, 0), (1, 0), false);
        assert_eq!("\u{1F468}\u{200D}", sel_str(&s, sel, true));
    }
}

const LOREM: &str = "Lorem ipsum dolor\nsit amet, consectetur\nadipiscing elit, sed do\neiusmod tempor incididunt\nut labore et dolore";

#[test]
fn screen_selection_string_rectangle_basic() {
    let mut s = init(30, 5, 0);
    let sel = sel_screen(&s, (2, 1), (6, 3), true);
    let expected = "t ame\nipisc\nusmod";
    s.test_write_string(LOREM).unwrap();

    assert_eq!(expected, sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_rectangle_w_eol() {
    let mut s = init(30, 5, 0);
    let sel = sel_screen(&s, (12, 0), (26, 4), true);
    let expected = "dolor\nnsectetur\nlit, sed do\nor incididunt\n dolore";
    s.test_write_string(LOREM).unwrap();

    assert_eq!(expected, sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_rectangle_more_complex_w_breaks() {
    let mut s = init(30, 8, 0);
    let str = "Lorem ipsum dolor\nsit amet, consectetur\nadipiscing elit, sed do\neiusmod tempor incididunt\nut labore et dolore\n\nmagna aliqua. Ut enim\nad minim veniam, quis";
    let sel = sel_screen(&s, (11, 2), (26, 7), true);
    let expected = "elit, sed do\npor incididunt\nt dolore\n\na. Ut enim\nniam, quis";
    s.test_write_string(str).unwrap();

    assert_eq!(expected, sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_multi_page() {
    let mut s = init(10, 3, 2048);

    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;

    // Lazy way to seek to the first page boundary.
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 1 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    s.test_write_string("123456789\n!@#$%^&*(\n123456789")
        .unwrap();

    {
        let sel = sel_active(&s, (0, 0), (2, 2), false);
        assert_eq!("123456789\n!@#$%^&*(\n123", sel_str(&s, sel, true));
    }
}

#[test]
fn screen_line_iterator() {
    let mut s = init(5, 5, 0);
    s.test_write_string("1ABCD\n2EFGH").unwrap();

    // Test the line iterator
    let mut iter = s.line_iterator(pin(&s, viewport(0, 0)));
    {
        let sel = iter.next().unwrap();
        assert_eq!("1ABCD", sel_str(&s, sel, false));
    }
    {
        let sel = iter.next().unwrap();
        assert_eq!("2EFGH", sel_str(&s, sel, false));
    }
}

#[test]
fn screen_line_iterator_soft_wrap() {
    let mut s = init(5, 5, 0);
    s.test_write_string("1ABCD2EFGH\n3ABCD").unwrap();

    // Test the line iterator
    let mut iter = s.line_iterator(pin(&s, viewport(0, 0)));
    {
        let sel = iter.next().unwrap();
        assert_eq!("1ABCD2EFGH", sel_str(&s, sel, false));
    }
    {
        let sel = iter.next().unwrap();
        assert_eq!("3ABCD", sel_str(&s, sel, false));
    }
    // assert!(iter.next().is_none());
}

#[test]
fn screen_hyperlink_start_end() {
    let mut s = init(5, 5, 0);
    assert!(s.cursor.hyperlink_id == 0);
    assert_eq!(0, s.cursor_page().hyperlink_set.count());

    s.start_hyperlink(b"http://example.com", None).unwrap();
    assert!(s.cursor.hyperlink_id != 0);
    assert_eq!(1, s.cursor_page().hyperlink_set.count());

    s.end_hyperlink();
    assert!(s.cursor.hyperlink_id == 0);
    assert_eq!(0, s.cursor_page().hyperlink_set.count());
}

#[test]
fn screen_hyperlink_reuse() {
    let mut s = init(5, 5, 0);

    assert!(s.cursor.hyperlink_id == 0);
    assert_eq!(0, s.cursor_page().hyperlink_set.count());

    // Use it for the first time
    s.start_hyperlink(b"http://example.com", None).unwrap();
    assert!(s.cursor.hyperlink_id != 0);
    let id = s.cursor.hyperlink_id;

    // Reuse the same hyperlink, expect we have the same ID
    s.start_hyperlink(b"http://example.com", None).unwrap();
    assert_eq!(id, s.cursor.hyperlink_id);
    assert_eq!(1, s.cursor_page().hyperlink_set.count());

    s.end_hyperlink();
    assert!(s.cursor.hyperlink_id == 0);
    assert_eq!(0, s.cursor_page().hyperlink_set.count());
}

#[test]
fn screen_hyperlink_cursor_state_on_resize() {
    // This test depends on underlying PageList implementation so
    // it may be invalid one day. It's here to document/verify the
    // current behavior.

    let mut s = init(5, 10, 0);

    // Start a hyperlink
    s.start_hyperlink(b"http://example.com", None).unwrap();
    assert!(s.cursor.hyperlink_id != 0);
    assert_eq!(1, s.cursor_page().hyperlink_set.count());

    // Resize. Any column growth will trigger a page to be reallocated.
    s.resize(Resize::new(10, 10)).unwrap();
    assert!(s.cursor.hyperlink_id != 0);
    assert_eq!(1, s.cursor_page().hyperlink_set.count());

    s.end_hyperlink();
    assert!(s.cursor.hyperlink_id == 0);
    assert_eq!(0, s.cursor_page().hyperlink_set.count());
}

#[test]
fn screen_cursor_set_hyperlink_oom_uri_too_large_for_string_alloc() {
    let mut s = init(80, 24, 0);

    // Start a hyperlink with a URI that just barely fits in the string alloc.
    // This will ensure that additional string alloc space is needed for the
    // redundant copy of the URI when the page is re-alloced.
    let uri = vec![b'a'; STD_CAPACITY.string_bytes as usize - 8];
    s.start_hyperlink(&uri, None).unwrap();

    // Figure out how many cells should can have hyperlinks in this page,
    // and write twice that number, to guarantee the capacity needs to be
    // increased at some point.
    let base_capacity = s.cursor_page().hyperlink_capacity();
    let base_string_bytes = s.cursor_page().capacity.string_bytes;
    for _ in 0..base_capacity * 2 {
        s.cursor_set_hyperlink().unwrap();
        if s.cursor.x >= s.pages.cols - 1 {
            s.cursor_down_or_scroll().unwrap();
            s.cursor_horizontal_absolute(0);
        } else {
            s.cursor_right(1);
        }
    }

    // Make sure the capacity really did increase.
    assert!(base_capacity < s.cursor_page().hyperlink_capacity());
    // And that our string_bytes increased as well.
    assert!(base_string_bytes < s.cursor_page().capacity.string_bytes);
}

#[test]
fn screen_increase_capacity_cursor_style_ref_count_preserved() {
    let mut s = init(5, 5, 0);
    s.set_attribute(A::Bold).unwrap();
    s.test_write_string("1ABCD").unwrap();

    // We should have one page and it should be our cursor page
    assert!(s.pages.first == s.pages.last);
    assert!(s.pages.first == Some(s.cursor_pin().node));

    let old_style = s.cursor.style;

    {
        let page = s.page(s.pages.last.unwrap());
        // 5 chars + cursor = 6 refs
        assert_eq!(6, page.styles.ref_count(s.cursor.style_id));
    }

    // This forces the page to change via increase_capacity.
    let new_node = s
        .increase_capacity(s.cursor_pin().node, Some(IncreaseCapacity::GraphemeBytes))
        .unwrap();

    // Cursor's page_pin should now point to the new node
    assert!(s.cursor_pin().node == new_node);

    // Cursor row/cell are derived from the pin; verify they resolve.
    let cp = s.cursor_pin();
    let (row, cell) = cp.row_and_cell(&s.pages).unwrap();
    assert_eq!(s.cursor_row(), *row);
    assert_eq!(s.cursor_cell(), *cell);

    // Style should be preserved
    assert_eq!(old_style, s.cursor.style);
    assert!(s.cursor.style_id != style::DEFAULT_ID);

    // After increase_capacity, the 5 chars are cloned (5 refs) and
    // the cursor's style is re-added (1 ref) = 6 total.
    {
        let page = s.page(s.pages.last.unwrap());
        let ref_count = page.styles.ref_count(s.cursor.style_id);
        assert_eq!(6, ref_count);
    }
}

#[test]
fn screen_increase_capacity_cursor_hyperlink_ref_count_preserved() {
    let mut s = init(5, 5, 0);
    s.start_hyperlink(b"https://example.com/", None).unwrap();
    s.test_write_string("1ABCD").unwrap();

    // We should have one page and it should be our cursor page
    assert!(s.pages.first == s.pages.last);
    assert!(s.pages.first == Some(s.cursor_pin().node));

    {
        let page = s.page(s.pages.last.unwrap());
        // Cursor has the hyperlink active = 1 count in hyperlink_set
        assert_eq!(1, page.hyperlink_set.count());
        assert!(s.cursor.hyperlink_id != 0);
        assert!(s.cursor.hyperlink.is_some());
    }

    // This forces the page to change via increase_capacity.
    s.increase_capacity(s.cursor_pin().node, Some(IncreaseCapacity::GraphemeBytes))
        .unwrap();

    // Hyperlink should be preserved with correct URI
    assert!(s.cursor.hyperlink.is_some());
    assert!(s.cursor.hyperlink_id != 0);
    assert_eq!(
        b"https://example.com/".as_slice(),
        s.cursor.hyperlink.as_ref().unwrap().uri.as_slice()
    );

    // After increase_capacity, the hyperlink is re-added to the new page.
    {
        let page = s.page(s.pages.last.unwrap());
        assert_eq!(1, page.hyperlink_set.count());
    }
}

#[test]
fn screen_increase_capacity_cursor_with_both_style_and_hyperlink_preserved() {
    let mut s = init(5, 5, 0);

    // Set both a non-default style AND an active hyperlink.
    // Write one character first with bold to mark the row as styled,
    // then start the hyperlink and write more characters.
    s.set_attribute(A::Bold).unwrap();
    s.start_hyperlink(b"https://example.com/", None).unwrap();
    s.test_write_string("1ABCD").unwrap();

    // We should have one page and it should be our cursor page
    assert!(s.pages.first == s.pages.last);
    assert!(s.pages.first == Some(s.cursor_pin().node));

    let old_style = s.cursor.style;

    {
        let page = s.page(s.pages.last.unwrap());
        // 5 chars + cursor = 6 refs for bold style
        assert_eq!(6, page.styles.ref_count(s.cursor.style_id));
        // Cursor has the hyperlink active = 1 count in hyperlink_set
        assert_eq!(1, page.hyperlink_set.count());
        assert!(s.cursor.style_id != style::DEFAULT_ID);
        assert!(s.cursor.hyperlink_id != 0);
        assert!(s.cursor.hyperlink.is_some());
    }

    // This forces the page to change via increase_capacity.
    s.increase_capacity(s.cursor_pin().node, Some(IncreaseCapacity::GraphemeBytes))
        .unwrap();

    // Style should be preserved
    assert_eq!(old_style, s.cursor.style);
    assert!(s.cursor.style_id != style::DEFAULT_ID);

    // Hyperlink should be preserved with correct URI
    assert!(s.cursor.hyperlink.is_some());
    assert!(s.cursor.hyperlink_id != 0);
    assert_eq!(
        b"https://example.com/".as_slice(),
        s.cursor.hyperlink.as_ref().unwrap().uri.as_slice()
    );

    // After increase_capacity, both style and hyperlink are re-added to the new page.
    {
        let page = s.page(s.pages.last.unwrap());
        let ref_count = page.styles.ref_count(s.cursor.style_id);
        assert_eq!(6, ref_count);
        assert_eq!(1, page.hyperlink_set.count());
    }
}

#[test]
fn screen_increase_capacity_non_cursor_page_returns_early() {
    // Test that calling increase_capacity on a page that is NOT the cursor's
    // page properly delegates to pages.increase_capacity without doing the
    // extra cursor accounting (style/hyperlink re-adding).
    let mut s = init(80, 24, 10000);

    // Set up a custom style and hyperlink on the cursor
    s.set_attribute(A::Bold).unwrap();
    s.start_hyperlink(b"https://example.com/", None).unwrap();
    s.test_write_string("Hello").unwrap();

    // Store cursor state before growing pages
    let old_style = s.cursor.style;
    let old_style_id = s.cursor.style_id;
    let old_hyperlink = s.cursor.hyperlink.clone();
    let old_hyperlink_id = s.cursor.hyperlink_id;

    // The cursor is on the first (and only) page
    assert!(s.pages.first == s.pages.last);
    assert!(Some(s.cursor_pin().node) == s.pages.first);

    // Grow pages until we have multiple pages. The cursor's pin stays on
    // the first page since we're just adding rows.
    let first_page_node = s.pages.first.unwrap();
    s.page_mut(first_page_node).pause_integrity_checks(true);
    let remaining = {
        let p = s.page(first_page_node);
        p.capacity.rows - p.size.rows
    };
    for _ in 0..remaining {
        s.pages.grow().unwrap();
    }
    s.page_mut(first_page_node).pause_integrity_checks(false);
    s.pages.grow().unwrap();

    // Now we have two pages
    assert!(s.pages.first != s.pages.last);
    let second_page = s.pages.last.unwrap();

    // Cursor should still be on the first page (where it was created)
    assert!(Some(s.cursor_pin().node) == s.pages.first);
    assert!(s.cursor_pin().node != second_page);

    let second_page_styles_cap = s.page(second_page).capacity.styles;
    let cursor_page_styles_cap = s.cursor_page().capacity.styles;

    // Call increase_capacity on the second page (NOT the cursor's page)
    let new_second_page = s
        .increase_capacity(second_page, Some(IncreaseCapacity::Styles))
        .unwrap();

    // The second page should have increased capacity
    assert_eq!(
        second_page_styles_cap * 2,
        s.page(new_second_page).capacity.styles
    );

    // The cursor's page (first page) should be unchanged
    assert_eq!(cursor_page_styles_cap, s.cursor_page().capacity.styles);

    // Cursor state should be completely unchanged since we didn't touch its page
    assert_eq!(old_style, s.cursor.style);
    assert_eq!(old_style_id, s.cursor.style_id);
    assert_eq!(old_hyperlink, s.cursor.hyperlink);
    assert_eq!(old_hyperlink_id, s.cursor.hyperlink_id);

    // Verify hyperlink is still valid
    assert!(s.cursor.hyperlink.is_some());
    assert_eq!(
        b"https://example.com/".as_slice(),
        s.cursor.hyperlink.as_ref().unwrap().uri.as_slice()
    );
}

/// Fill a page's style set until `add` fails, with unique bg colors.
fn fill_styles(page: &mut Page) {
    page.pause_integrity_checks(true);
    let mut n: u32 = 1;
    while page
        .styles
        .add(Style {
            bg_color: Color::Rgb(Rgb {
                r: (n >> 16) as u8,
                g: (n >> 8) as u8,
                b: n as u8,
            }),
            ..Style::default()
        })
        .is_ok()
    {
        n += 1;
    }
    page.pause_integrity_checks(false);
    page.assert_integrity();
}

#[test]
fn screen_cursor_down_to_page_with_insufficient_capacity() {
    // Regression test for https://github.com/ghostty-org/ghostty/issues/10282
    //
    // This test exposes a use-after-realloc bug in cursorDown (and similar
    // cursor movement functions). The bug pattern:
    //
    // 1. cursorDown creates a by-value copy of the pin via page_pin.down(n)
    // 2. cursorChangePin is called, which may trigger adjustCapacity
    //    if the target page's style map is full
    // 3. adjustCapacity frees the old page and creates a new one
    // 4. The local pin copy still points to the freed page
    // 5. rowAndCell() on the stale pin accesses freed memory

    // Small screen to make page boundary crossing easy to set up
    let mut s = init(10, 3, 1);

    // Scroll down enough to create a second page
    let start_page = s.pages.last.unwrap();
    let rem = s.page(start_page).capacity.rows;
    s.page_mut(start_page).pause_integrity_checks(true);
    for _ in 0..rem {
        s.cursor_down_or_scroll().unwrap();
    }
    s.page_mut(start_page).pause_integrity_checks(false);

    // Cursor should now be on a new page
    let new_page = s.cursor_pin().node;
    assert!(start_page != new_page);

    // Fill new_page's style map to capacity. When we move INTO this page
    // with a style set, adjustCapacity will be triggered.
    fill_styles(s.page_mut(new_page));

    // Move cursor to start of active area and set a style
    s.cursor_absolute(0, 0);
    s.set_attribute(A::Bold).unwrap();
    assert!(s.cursor.style.flags.bold);
    assert!(s.cursor.style_id != style::DEFAULT_ID);

    // Find the row just before the page boundary
    let mut found = false;
    for row in 0..s.pages.rows - 1 {
        s.cursor_absolute(0, row);
        let cur_node = s.cursor_pin().node;
        if let Some(next_pin) = s.cursor_pin().down(&s.pages, 1)
            && next_pin.node != cur_node
        {
            // Cursor is at 'row', moving down crosses to new_page
            assert!(next_pin.node == new_page);

            // This cursorDown triggers the bug: the local page_pin copy
            // becomes stale after adjustCapacity, causing rowAndCell()
            // to access freed memory.
            s.cursor_down(1);

            // If the fix is applied, verify correct state
            assert!(s.cursor.y == row + 1);
            assert!(s.cursor.style.flags.bold);

            found = true;
            break;
        }
    }
    // Didn't find boundary
    assert!(found);
}

#[test]
fn screen_set_attribute_increases_capacity_when_style_map_is_full() {
    // Tests that setAttribute succeeds when the style map is full by
    // increasing page capacity. When capacity is at max and increaseCapacity
    // returns OutOfSpace, manualStyleUpdate will split the page instead.

    // Use a small screen with multiple rows
    let mut s = init(10, 5, 10);

    // Write content to multiple rows
    s.test_write_string("line1\nline2\nline3\nline4\nline5")
        .unwrap();

    // Get the page and fill its style map to capacity
    let page = s.cursor_pin().node;
    let original_styles_capacity = s.page(page).capacity.styles;

    // Fill the style map to capacity
    fill_styles(s.page_mut(page));

    // Now try to set a new unique attribute that would require a new style slot
    // This should succeed by increasing capacity (or splitting if at max capacity)
    s.set_attribute(A::Bold).unwrap();

    // The style should have been applied (bold flag set)
    assert!(s.cursor.style.flags.bold);

    // The cursor should have a valid non-default style_id
    assert!(s.cursor.style_id != style::DEFAULT_ID);

    // Either the capacity increased or the page was split/changed
    let current_page = s.cursor_pin().node;
    let capacity_increased = s.page(current_page).capacity.styles > original_styles_capacity;
    let page_changed = current_page != page;
    assert!(capacity_increased || page_changed);
}

#[test]
fn screen_set_attribute_splits_page_on_out_of_space_at_max_styles() {
    let mut s = init(10, 10, 0);

    // Write content to multiple rows so we have something to split
    s.test_write_string("line1\nline2\nline3\nline4\nline5")
        .unwrap();

    // Remember the original node
    let original_node = s.cursor_pin().node;

    // Increase the page's style capacity to max by repeatedly calling increase_capacity
    // Use Screen.increase_capacity to properly maintain cursor state
    let max_styles = size::CellCountInt::MAX;
    while s.cursor_page().capacity.styles < max_styles {
        if s.increase_capacity(s.cursor_pin().node, Some(IncreaseCapacity::Styles))
            .is_err()
        {
            break;
        }
    }

    // Get the page reference after increase_capacity - cursor may have moved
    let node = s.cursor_pin().node;
    assert_eq!(max_styles, s.page(node).capacity.styles);

    // Fill the style map to capacity
    fill_styles(s.page_mut(node));

    // Track the node before setAttribute
    let node_before_set = s.cursor_pin().node;

    // Now try to set a new unique attribute that would require a new style slot
    // At max capacity, increase_capacity will return OutOfSpace, triggering page split
    s.set_attribute(A::Bold).unwrap();

    // The style should have been applied (bold flag set)
    assert!(s.cursor.style.flags.bold);

    // The cursor should have a valid non-default style_id
    assert!(s.cursor.style_id != style::DEFAULT_ID);

    // The page should have been split
    let nb = s.pages.node(node_before_set);
    let page_was_split = s.cursor_pin().node != node_before_set
        || nb.is_some_and(|n| n.next.is_some())
        || nb.is_some_and(|n| n.prev.is_some())
        || s.cursor_pin().node != original_node;
    assert!(page_was_split);
}

#[test]
fn screen_selection_string_map_allocation_failure_cleanup() {
    use crate::tripwire::ResetMode;

    // If building the StringMap fails, the already-built string must be
    // dropped and the error returned (Zig: errdefer free(map_string)).
    let mut s = init(10, 5, 0);
    s.test_write_string("hello").unwrap();
    let sel = sel_active(&s, (0, 0), (4, 0), false);

    selection_string_tw::error_always(
        SelectionStringPoint::CopyMap,
        SelectionStringError::OutOfMemory,
    );
    let result = s.try_selection_string_map(SelectionString { sel, trim: true });
    assert!(matches!(result, Err(SelectionStringError::OutOfMemory)));
    selection_string_tw::end(ResetMode::Reset).unwrap();

    // And it works again once the tripwire is reset.
    let map = s
        .try_selection_string_map(SelectionString { sel, trim: true })
        .unwrap();
    assert_eq!(map.string, "hello");
    assert_eq!(map.map.len(), 5);
}

fn setup_prompt(cols: u16, click: Click) -> Screen {
    let mut s = init(cols, 5, 0);
    // Enable line click mode
    s.semantic_prompt.click = SemanticClick::Cl(click);
    s
}

fn prompt(s: &mut Screen) {
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
}
fn input(s: &mut Screen) {
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
}
fn output(s: &mut Screen) {
    s.cursor_set_semantic_content(SemanticContentSet::Output);
}
fn click_at(s: &Screen, x: u16, y: u32) -> PromptClickMove {
    let p = pin(s, active(x, y));
    s.prompt_click_move(p)
}

#[test]
fn screen_prompt_click_move_line_right_basic() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor back to start of input (column 2, the 'h')
    s.cursor_absolute(2, 0);

    // Click on first 'l' (column 4), should require 2 right movements (h->e->l)
    let result = click_at(&s, 4, 0);

    assert_eq!(2, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_line_right_cursor_not_on_input() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();
    output(&mut s);

    // Move cursor back to prompt area (column 0, the '>')
    s.cursor_absolute(0, 0);

    // Cursor is on prompt, not input - should return zero
    let result = click_at(&s, 4, 0);

    assert_eq!(PromptClickMove::ZERO, result);
}

#[test]
fn screen_prompt_click_move_line_right_click_on_same_position() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor to column 4
    s.cursor_absolute(4, 0);

    // Click on same position - no movement needed
    let result = click_at(&s, 4, 0);

    assert_eq!(PromptClickMove::ZERO, result);
}

#[test]
fn screen_prompt_click_move_line_right_skips_non_input_cells() {
    let mut s = setup_prompt(20, Click::Line);
    // Write: "> h" then output "X" then input "llo"
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("h").unwrap();
    output(&mut s);
    s.test_write_string("X").unwrap();
    input(&mut s);
    s.test_write_string("llo").unwrap();

    // Move cursor to column 2 (the 'h')
    s.cursor_absolute(2, 0);

    // Click on 'l' at column 5 - should skip the 'X' output cell
    // Movement: h (start) -> l (col 4) -> l (col 5) = 2 right movements
    let result = click_at(&s, 5, 0);

    assert_eq!(2, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_line_right_soft_wrapped_line() {
    let mut s = setup_prompt(10, Click::Line);
    // Write a prompt and input that wraps
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    // Write 8 chars of input, first row has 2 for prompt + 8 input = 10 cols
    s.test_write_string("abcdefgh").unwrap();
    // Continue on next row (soft-wrapped)
    s.test_write_string("ij").unwrap();

    // Verify soft wrap occurred
    assert_eq!("> abcdefgh\nij", dump(&s, viewport(0, 0)));

    // Move cursor to column 2 (the 'a')
    s.cursor_absolute(2, 0);

    // Click on 'j' at column 1, row 1 - should count all input cells
    // Movement: a->b->c->d->e->f->g->h->i->j = 9 right movements
    let result = click_at(&s, 1, 1);

    assert_eq!(9, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_disabled_when_click_is_none() {
    let mut s = setup_prompt(20, Click::Line);
    // Click mode is .none by default (disabled)
    s.semantic_prompt.click = SemanticClick::None;
    assert_eq!(SemanticClick::None, s.semantic_prompt.click);

    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor to start of input
    s.cursor_absolute(2, 0);

    // Click should return zero since click mode is disabled
    let result = click_at(&s, 4, 0);

    assert_eq!(PromptClickMove::ZERO, result);
}

#[test]
fn screen_prompt_click_move_line_right_stops_at_hard_wrap() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();
    // Hard wrap (newline)
    s.test_write_string("\n").unwrap();
    s.test_write_string("world").unwrap();

    // Move cursor to column 2 (the 'h')
    s.cursor_absolute(2, 0);

    // Click on 'w' at column 0, row 1 - but line mode stops at hard wrap
    // Should only move to end of first line: h->e->l->l->o = 4 movements
    let result = click_at(&s, 0, 1);

    // Should stop at end of first line, not cross hard wrap
    assert_eq!(5, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_line_right_stops_at_non_continuation_row() {
    let mut s = setup_prompt(20, Click::Line);
    // Row 0: PROMPT "> hello"
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello\n").unwrap();

    // Row 1: CONTINUATION "world"
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Continuation));
    input(&mut s);
    s.test_write_string("world\n").unwrap();

    // Row 2: NEW PROMPT "> again"
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("again").unwrap();

    // Verify content
    assert_eq!("> hello\nworld\n> again", dump(&s, viewport(0, 0)));

    // Move cursor to 'w' at column 0, row 1
    s.cursor_absolute(0, 1);

    // Click on 'a' at column 2, row 2 - but row 2 is a new prompt
    // Should stop at end of "world": w->o->r->l->d = 4 movements
    let result = click_at(&s, 2, 2);

    // Should stop at 'd' (end of world), not cross to new prompt
    assert_eq!(5, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_line_left_basic() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Cursor is at column 7 (after 'o'), move it to column 6 (the 'o')
    s.cursor_absolute(6, 0);

    // Click on 'h' (column 2), should require 4 left movements (o->l->l->e->h)
    let result = click_at(&s, 2, 0);

    assert_eq!(0, result.right);
    assert_eq!(4, result.left);
}

#[test]
fn screen_prompt_click_move_line_left_skips_non_input_cells() {
    let mut s = setup_prompt(20, Click::Line);
    // Write: "> h" then output "X" then input "llo"
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("h").unwrap();
    output(&mut s);
    s.test_write_string("X").unwrap();
    input(&mut s);
    s.test_write_string("llo").unwrap();

    // Move cursor to column 6 (the 'o')
    s.cursor_absolute(6, 0);

    // Click on 'h' at column 2 - should skip the 'X' output cell
    // Movement: o->l->l->h = 3 left movements (skipping X)
    let result = click_at(&s, 2, 0);

    assert_eq!(0, result.right);
    assert_eq!(3, result.left);
}

#[test]
fn screen_prompt_click_move_line_left_soft_wrapped_line() {
    let mut s = setup_prompt(10, Click::Line);
    // Write a prompt and input that wraps
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    // Write 8 chars of input, first row has 2 for prompt + 8 input = 10 cols
    s.test_write_string("abcdefgh").unwrap();
    // Continue on next row (soft-wrapped)
    s.test_write_string("ij").unwrap();

    // Verify soft wrap occurred
    assert_eq!("> abcdefgh\nij", dump(&s, viewport(0, 0)));

    // Cursor is at column 2, row 1 (after 'j'). Move to 'j' at column 1.
    s.cursor_absolute(1, 1);

    // Click on 'a' at column 2, row 0 - should count all input cells backwards
    // Movement: j->i->h->g->f->e->d->c->b->a = 9 left movements
    let result = click_at(&s, 2, 0);

    assert_eq!(0, result.right);
    assert_eq!(9, result.left);
}

#[test]
fn screen_prompt_click_move_line_left_stops_at_hard_wrap() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();
    // Hard wrap (newline)
    s.test_write_string("\n").unwrap();
    s.test_write_string("world").unwrap();

    // Move cursor to 'd' at column 4, row 1 (an actual input cell)
    s.cursor_absolute(4, 1);

    // Click on 'h' at column 2, row 0 - but line mode stops at hard wrap
    // Should only move to start of second line, not cross to row 0
    let result = click_at(&s, 2, 0);

    // Should stop at start of second line: d->l->r->o->w = 4 movements
    assert_eq!(0, result.right);
    assert_eq!(4, result.left);
}

#[test]
fn screen_prompt_click_move_click_right_of_input_same_line() {
    let mut s = setup_prompt(20, Click::Line);
    // Set up: "> hello" where "> " is prompt and "hello" is input
    // Clicking to the right of the 'o' should move cursor past the input
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor to start of input (column 2, the 'h')
    s.cursor_absolute(2, 0);

    // Click beyond the input (column 15) - should move to one past the 'o'
    let result = click_at(&s, 15, 0);

    assert_eq!(5, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_click_right_of_input_cursor_at_end() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Cursor is already at column 7 (one past 'o') after writing
    // Click beyond the input (column 15) - no movement needed since
    // cursor is already at the end position
    let result = click_at(&s, 15, 0);

    assert_eq!(0, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_click_right_of_input_on_lower_line() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor to start of input (column 2, the 'h')
    s.cursor_absolute(2, 0);

    // Click on a lower line (row 1) - should move to end of input
    // This is outside the prompt area so should clamp to end
    let result = click_at(&s, 5, 1);

    // From 'h', we need to pass e, l, l, o (4 cells) + 1 past end = 5
    assert_eq!(5, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_click_right_of_input_cursor_at_end_lower_line() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Cursor is at column 7 after writing (one past 'o')
    // Click on a lower line (row 1) - cursor already at end, no movement needed
    let result = click_at(&s, 5, 1);

    assert_eq!(0, result.right);
    assert_eq!(0, result.left);
}

#[test]
fn screen_prompt_click_move_click_right_of_input_cursor_on_last_char() {
    let mut s = setup_prompt(20, Click::Line);
    prompt(&mut s);
    s.test_write_string("> ").unwrap();
    input(&mut s);
    s.test_write_string("hello").unwrap();

    // Move cursor to last input char (column 6, the 'o')
    s.cursor_absolute(6, 0);

    // Click beyond the input (column 15)
    let result = click_at(&s, 15, 0);

    assert_eq!(1, result.right);
    assert_eq!(0, result.left);
}
