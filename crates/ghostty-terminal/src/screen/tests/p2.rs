use super::*;
use ghostty_vt::{color::Rgb, sgr::Attribute as A};

fn bg155() -> A<'static> {
    A::DirectColorBg(Rgb { r: 155, g: 0, b: 0 })
}

fn assert_bg155(s: &Screen, x: u16, y: u32) {
    let cell = s.pages.get_cell(active(x, y)).unwrap();
    assert!(cell.content_tag() == ContentTag::BgColorRgb);
    assert_eq!(Rgb { r: 155, g: 0, b: 0 }, cell.color_rgb());
}

fn bold_style(s: &Screen) -> bool {
    let page = s.cursor_page();
    page.styles.get(s.cursor.style_id).unwrap().flags.bold
}

/// Scroll down enough to go to another page. Returns the start page.
fn scroll_to_new_page(s: &mut Screen) -> NodeId {
    let start_page = s.pages.last.unwrap();
    let rem = s.page(start_page).capacity.rows;
    s.page_mut(start_page).pause_integrity_checks(true);
    for _ in 0..rem {
        s.cursor_down_or_scroll().unwrap();
    }
    s.page_mut(start_page).pause_integrity_checks(false);
    start_page
}

#[test]
fn screen_cursor_down_across_pages_preserves_style() {
    let mut s = init(10, 3, 1);

    // Scroll down enough to go to another page
    let start_page = scroll_to_new_page(&mut s);

    // We need our page to change for this test o make sense. If this
    // assertion fails then the bug is in the test: we should be scrolling
    // above enough for a new page to show up.
    assert!(start_page != s.cursor_pin().node);

    // Scroll back to the previous page
    s.cursor_up(1);
    assert!(start_page == s.cursor_pin().node);

    // Go back up, set a style
    s.set_attribute(A::Bold).unwrap();
    assert!(bold_style(&s));

    // Go back down into the next page and we should have that style
    s.cursor_down(1);
    assert!(bold_style(&s));
}

#[test]
fn screen_cursor_up_across_pages_preserves_style() {
    let mut s = init(10, 3, 1);

    let start_page = scroll_to_new_page(&mut s);
    assert!(start_page != s.cursor_pin().node);

    // Go back up, set a style
    s.set_attribute(A::Bold).unwrap();
    assert!(bold_style(&s));

    // Go back down into the prev page and we should have that style
    s.cursor_up(1);
    assert!(start_page == s.cursor_pin().node);
    assert!(bold_style(&s));
}

#[test]
fn screen_cursor_absolute_across_pages_preserves_style() {
    let mut s = init(10, 3, 1);

    let start_page = scroll_to_new_page(&mut s);
    assert!(start_page != s.cursor_pin().node);

    // Go back up, set a style
    s.set_attribute(A::Bold).unwrap();
    assert!(bold_style(&s));

    // Go back down into the prev page and we should have that style
    s.cursor_absolute(1, 1);
    assert!(start_page == s.cursor_pin().node);
    assert!(bold_style(&s));
}

#[test]
fn screen_cursor_absolute_to_page_with_insufficient_capacity() {
    // This test checks for a very specific edge case
    // which previously resulted in memory corruption.
    //
    // The conditions for this edge case are as such:
    // - The cursor has an associated style or other managed memory.
    // - The cursor moves to a different page.
    // - The new page is at capacity and must have its capacity adjusted.
    let mut s = init(10, 3, 1);

    let start_page = scroll_to_new_page(&mut s);
    let new_page = s.cursor_pin().node;

    // We need our page to change for this test to make sense. If this
    // assertion fails then the bug is in the test: we should be scrolling
    // above enough for a new page to show up.
    assert!(start_page != new_page);

    // Add styles to the start page until it reaches capacity.
    {
        // Pause integrity checks because they're slow and
        // we're not testing this, this is just setup.
        s.page_mut(start_page).pause_integrity_checks(true);

        let mut n: u32 = 1;
        while s
            .page_mut(start_page)
            .styles
            .add(Style {
                bg_color: style::Color::Rgb(Rgb {
                    r: (n & 0xff) as u8,
                    g: ((n >> 8) & 0xff) as u8,
                    b: ((n >> 16) & 0xff) as u8,
                }),
                ..Style::default()
            })
            .is_ok()
        {
            n += 1;
        }

        s.page_mut(start_page).pause_integrity_checks(false);
        s.page(start_page).assert_integrity();
    }

    // Set a style on the cursor.
    s.set_attribute(A::Bold).unwrap();
    {
        let styleval = s.page(new_page).styles.get(s.cursor.style_id).unwrap();
        assert!(styleval.flags.bold);
    }

    // Go back up into the start page and we should still have that style.
    s.cursor_absolute(1, 1);
    {
        let cur_page = s.cursor_pin().node;
        // The page we're on now should NOT equal start_page, since its
        // capacity should have been adjusted, which invalidates our ptr.
        assert!(start_page != cur_page);
        // To make sure we DID change pages we check we're not on new_page.
        assert!(new_page != cur_page);

        let styleval = s.page(cur_page).styles.get(s.cursor.style_id).unwrap();
        assert!(styleval.flags.bold);
    }

    s.cursor_page().assert_integrity();
    s.page(new_page).assert_integrity();
}

#[test]
fn screen_scrolling() {
    let mut s = init(10, 3, 0);
    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    // Scroll down, should still be bottom
    s.cursor_down_scroll().unwrap();
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 2);

    // Everything is dirty because we have no scrollback
    assert!(s.pages.is_dirty(active(0, 0)));
    assert!(s.pages.is_dirty(active(0, 1)));
    assert!(s.pages.is_dirty(active(0, 2)));

    // Scrolling to the bottom does nothing
    s.scroll(Scroll::Active);

    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrolling_with_a_single_row_screen_no_scrollback() {
    let mut s = init(10, 1, 0);
    s.test_write_string("1ABCD").unwrap();

    // Scroll down, should still be bottom
    s.cursor_down_scroll().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));

    // Screen should be dirty
    assert!(s.pages.is_dirty(active(0, 0)));
}

#[test]
fn screen_scrolling_with_a_single_row_screen_with_scrollback() {
    let mut s = init(10, 1, 1);
    s.test_write_string("1ABCD").unwrap();

    // Scroll down, should still be bottom
    s.cursor_down_scroll().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));

    // Active should be dirty
    assert!(s.pages.is_dirty(active(0, 0)));

    // Scrollback also dirty because cursor moved from there
    assert!(s.pages.is_dirty(screen_pt(0, 0)));

    s.scroll(Scroll::DeltaRow(-1));
    assert_eq!("1ABCD", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrolling_across_pages_preserves_style() {
    let mut s = init(10, 3, 1);
    s.set_attribute(A::Bold).unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    let start_page = s.pages.last.unwrap();

    // Scroll down enough to go to another page
    let rem = {
        let p = s.page(start_page);
        p.capacity.rows - p.size.rows + 1
    };
    s.page_mut(start_page).pause_integrity_checks(true);
    for _ in 0..rem {
        s.cursor_down_or_scroll().unwrap();
    }
    s.page_mut(start_page).pause_integrity_checks(false);

    // We need our page to change for this test o make sense. If this
    // assertion fails then the bug is in the test: we should be scrolling
    // above enough for a new page to show up.
    let page = s.pages.last.unwrap();
    assert!(start_page != page);

    let styleval = s.page(page).styles.get(s.cursor.style_id).unwrap();
    assert!(styleval.flags.bold);
}

#[test]
fn screen_scroll_down_from_0() {
    let mut s = init(10, 3, 0);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    // Scrolling up does nothing, but allows it
    s.scroll(Scroll::DeltaRow(-1));
    assert!(s.pages.viewport == Viewport::Active);

    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrollback_various_cases() {
    let mut s = init(10, 3, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.cursor_down_scroll().unwrap();

    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling to the bottom
    s.scroll(Scroll::Active);
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling back should make it visible again
    s.scroll(Scroll::DeltaRow(-1));
    assert!(s.pages.viewport != Viewport::Active);
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling back again should do nothing
    s.scroll(Scroll::DeltaRow(-1));
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling to the bottom
    s.scroll(Scroll::Active);
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling forward with no grow should do nothing
    s.scroll(Scroll::DeltaRow(1));
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling to the top should work
    s.scroll(Scroll::Top);
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Should be able to easily clear active area only
    s.clear_rows(active(0, 0), None, false);
    assert_eq!("1ABCD", dump(&s, viewport(0, 0)));

    // Scrolling to the bottom
    s.scroll(Scroll::Active);
    assert_eq!("", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrollback_with_multi_row_delta() {
    let mut s = init(10, 3, 3);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH\n6IJKL")
        .unwrap();

    // Scroll to top
    s.scroll(Scroll::Top);
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scroll down multiple
    s.scroll(Scroll::DeltaRow(5));
    assert!(s.pages.viewport == Viewport::Active);
    assert_eq!("4ABCD\n5EFGH\n6IJKL", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrollback_empty() {
    let mut s = init(10, 3, 50);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.scroll(Scroll::DeltaRow(1));
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrollback_doesnt_move_viewport_if_not_at_bottom() {
    let mut s = init(10, 3, 3);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH")
        .unwrap();

    // First test: we scroll up by 1, so we're not at the bottom anymore.
    s.scroll(Scroll::DeltaRow(-1));
    assert_eq!("2EFGH\n3IJKL\n4ABCD", dump(&s, viewport(0, 0)));

    // Next, we scroll back down by 1, this grows the scrollback but we
    // shouldn't move.
    s.cursor_down_scroll().unwrap();
    assert_eq!("2EFGH\n3IJKL\n4ABCD", dump(&s, viewport(0, 0)));

    // Scroll again, this clears scrollback so we should move viewports
    // but still see the same thing since our original view fits.
    s.cursor_down_scroll().unwrap();
    assert_eq!("2EFGH\n3IJKL\n4ABCD", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_scrolling_moves_selection() {
    let mut s = init(5, 3, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    // Select a single line
    let sel = Selection::new(
        pin(&s, active(0, 1)),
        pin(&s, active(s.pages.cols - 1, 1)),
        false,
    );
    s.select(Some(sel)).unwrap();

    // Scroll down, should still be bottom
    s.cursor_down_scroll().unwrap();

    // Our selection should've moved up
    {
        let sel = s.selection.unwrap();
        assert_eq!(
            active(0, 0),
            pt_from_pin(&s, Tag::Active, sel.start(&s.pages))
        );
        assert_eq!(
            active(s.pages.cols - 1, 0),
            pt_from_pin(&s, Tag::Active, sel.end(&s.pages))
        );
    }

    // Test our contents rotated
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scrolling to the bottom does nothing
    s.scroll(Scroll::Active);

    // Our selection should've stayed the same
    {
        let sel = s.selection.unwrap();
        assert_eq!(
            active(0, 0),
            pt_from_pin(&s, Tag::Active, sel.start(&s.pages))
        );
        assert_eq!(
            active(s.pages.cols - 1, 0),
            pt_from_pin(&s, Tag::Active, sel.end(&s.pages))
        );
    }

    // Test our contents rotated
    assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    // Scroll up again
    s.cursor_down_scroll().unwrap();

    // Test our contents rotated
    assert_eq!("3IJKL", dump(&s, viewport(0, 0)));

    // Our selection should be null because it left the screen.
    {
        let sel = s.selection.unwrap();
        assert!(
            s.pages
                .point_from_pin(Tag::Active, sel.start(&s.pages))
                .is_none()
        );
        assert!(
            s.pages
                .point_from_pin(Tag::Active, sel.end(&s.pages))
                .is_none()
        );
    }
}

#[test]
fn screen_scrolling_moves_viewport() {
    let mut s = init(10, 3, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n").unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.scroll(Scroll::DeltaRow(-2));

    // Test our contents rotated
    assert_eq!("2EFGH\n3IJKL\n1ABCD", dump(&s, viewport(0, 0)));

    let tl = s.pages.get_top_left(Tag::Viewport).unwrap();
    assert_eq!(screen_pt(0, 1), pt_from_pin(&s, Tag::Screen, tl));
}

#[test]
fn screen_scrolling_when_viewport_is_pruned() {
    let mut s = init(215, 3, 1);

    // Write some to create scrollback and move back into our scrollback.
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n").unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.scroll(Scroll::DeltaRow(-2));

    // Our viewport is now somewhere pinned. Create so much scrollback
    // that we prune it.
    s.test_write_string("\n").unwrap();
    for _ in 0..1000 {
        s.test_write_string("1ABCD\n2EFGH\n3IJKL\n").unwrap();
    }
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    let tl = s.pages.get_top_left(Tag::Viewport).unwrap();
    assert_eq!(screen_pt(0, 0), pt_from_pin(&s, Tag::Screen, tl));
}

#[test]
fn screen_scroll_and_clear_full_screen() {
    let mut s = init(10, 3, 5);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));

    s.scroll_clear().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));
    assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_scroll_and_clear_partial_screen() {
    let mut s = init(10, 3, 5);
    s.test_write_string("1ABCD\n2EFGH").unwrap();

    assert_eq!("1ABCD\n2EFGH", dump(&s, viewport(0, 0)));

    s.scroll_clear().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));
    assert_eq!("1ABCD\n2EFGH", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_scroll_and_clear_empty_screen() {
    let mut s = init(10, 3, 5);
    s.scroll_clear().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));
    assert_eq!("", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_scroll_and_clear_ignore_blank_lines() {
    let mut s = init(10, 3, 10);
    s.test_write_string("1ABCD\n2EFGH").unwrap();
    s.scroll_clear().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));

    // Move back to top-left
    s.cursor_absolute(0, 0);

    // Write and clear
    s.test_write_string("3ABCD\n").unwrap();
    assert_eq!("3ABCD", dump(&s, active(0, 0)));

    s.scroll_clear().unwrap();
    assert_eq!("", dump(&s, viewport(0, 0)));

    // Move back to top-left
    s.cursor_absolute(0, 0);
    s.test_write_string("X").unwrap();

    assert_eq!("1ABCD\n2EFGH\n3ABCD\nX", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_scroll_above_same_page() {
    let mut s = init(10, 3, 10);
    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // At this point:
    //  +-------------+ ACTIVE
    //   +----------+ : = PAGE 0
    // 0 |1ABCD00000| | 0
    // 1 |2EFGH00000| | 1
    //   :^         : : = PIN 0
    // 2 |3IJKL00000| | 2
    //   +----------+ :
    //  +-------------+

    s.cursor_scroll_above().unwrap();

    //   +----------+ = PAGE 0
    // 0 |1ABCD00000|
    //  +-------------+ ACTIVE
    // 1 |2EFGH00000| | 0
    // 2 |          | | 1
    //   :^         : : = PIN 0
    // 3 |3IJKL00000| | 2
    //   +----------+ :
    //  +-------------+

    assert_eq!("2EFGH\n\n3IJKL", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0 row 1 (active row 0) is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // Page 0 row 2 (active row 1) is dirty because it was cleared.
    assert!(s.pages.is_dirty(active(0, 1)));
    // Page 0 row 3 (active row 2) is dirty because it's new.
    assert!(s.pages.is_dirty(active(0, 2)));
}

#[test]
fn screen_scroll_above_same_page_but_cursor_on_previous_page() {
    let mut s = init(10, 5, 10);

    // We need to get the cursor to a new page
    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 3 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1A\n2B\n3C\n4D\n5E").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // Ensure we're still on the first page and have a second
    assert!(s.cursor_pin().node == s.pages.first.unwrap());
    assert!(s.pages.node(s.pages.first.unwrap()).unwrap().next.is_some());

    // At this point:
    //      +----------+ = PAGE 0
    //  ... :          :
    //     +-------------+ ACTIVE
    // 4305 |1A00000000| | 0
    // 4306 |2B00000000| | 1
    //      :^         : : = PIN 0
    // 4307 |3C00000000| | 2
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |4D00000000| | 3
    //    1 |5E00000000| | 4
    //      +----------+ :
    //     +-------------+

    s.cursor_scroll_above().unwrap();

    //      +----------+ = PAGE 0
    //  ... :          :
    // 4305 |1A00000000|
    //     +-------------+ ACTIVE
    // 4306 |2B00000000| | 0
    // 4307 |          | | 1
    //      :^         : : = PIN 0
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |3C00000000| | 2
    //    1 |4D00000000| | 3
    //    2 |5E00000000| | 4
    //      +----------+ :
    //     +-------------+

    assert_eq!("2B\n\n3C\n4D\n5E", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0's penultimate row is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // The rest of the rows are dirty because they've been modified or are new.
    assert!(s.pages.is_dirty(active(0, 1)));
    assert!(s.pages.is_dirty(active(0, 2)));
    assert!(s.pages.is_dirty(active(0, 3)));
    assert!(s.pages.is_dirty(active(0, 4)));
}

#[test]
fn screen_scroll_above_same_page_but_cursor_on_previous_page_last_row() {
    let mut s = init(10, 5, 10);

    // We need to get the cursor to a new page
    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 2 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1A\n2B\n3C\n4D\n5E").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // Ensure we're still on the first page and have a second
    assert!(s.cursor_pin().node == s.pages.first.unwrap());
    assert!(s.pages.node(s.pages.first.unwrap()).unwrap().next.is_some());

    // At this point:
    //      +----------+ = PAGE 0
    //  ... :          :
    //     +-------------+ ACTIVE
    // 4306 |1A00000000| | 0
    // 4307 |2B00000000| | 1
    //      :^         : : = PIN 0
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |3C00000000| | 2
    //    1 |4D00000000| | 3
    //    2 |5E00000000| | 4
    //      +----------+ :
    //     +-------------+

    s.cursor_scroll_above().unwrap();

    //      +----------+ = PAGE 0
    //  ... :          :
    // 4306 |1A00000000|
    //     +-------------+ ACTIVE
    // 4307 |2B00000000| | 0
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |          | | 1
    //      :^         : : = PIN 0
    //    1 |3C00000000| | 2
    //    2 |4D00000000| | 3
    //    3 |5E00000000| | 4
    //      +----------+ :
    //     +-------------+

    assert_eq!("2B\n\n3C\n4D\n5E", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0's final row is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // Page 1's rows are all dirty because every row was moved.
    assert!(s.pages.is_dirty(active(0, 1)));
    assert!(s.pages.is_dirty(active(0, 2)));
    assert!(s.pages.is_dirty(active(0, 3)));
    assert!(s.pages.is_dirty(active(0, 4)));

    // Attempt to clear the style from the cursor and
    // then assert the integrity of both of our pages.
    //
    // This catches a case of memory corruption where the cursor
    // is moved between pages without accounting for style refs.
    s.set_attribute(A::ResetBg).unwrap();
    s.page(s.pages.first.unwrap()).assert_integrity();
    s.page(s.pages.last.unwrap()).assert_integrity();
}

#[test]
fn screen_scroll_above_creates_new_page() {
    let mut s = init(10, 3, 10);

    // We need to get the cursor to a new page
    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 3 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // Ensure we're still on the first page
    assert!(s.cursor_pin().node == s.pages.first.unwrap());

    // At this point:
    //      +----------+ = PAGE 0
    //  ... :          :
    //     +-------------+ ACTIVE
    // 4305 |1ABCD00000| | 0
    // 4306 |2EFGH00000| | 1
    //      :^         : : = PIN 0
    // 4307 |3IJKL00000| | 2
    //      +----------+ :
    //     +-------------+
    s.cursor_scroll_above().unwrap();

    //      +----------+ = PAGE 0
    //  ... :          :
    // 4305 |1ABCD00000|
    //     +-------------+ ACTIVE
    // 4306 |2EFGH00000| | 0
    // 4307 |          | | 1
    //      :^         : : = PIN 0
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |3IJKL00000| | 2
    //      +----------+ :
    //     +-------------+

    assert_eq!("2EFGH\n\n3IJKL", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0's penultimate row is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // Page 0's final row is dirty because it was cleared.
    assert!(s.pages.is_dirty(active(0, 1)));
    // Page 1's row is dirty because it's new.
    assert!(s.pages.is_dirty(active(0, 2)));
}

#[test]
fn screen_scroll_above_with_cursor_on_non_final_row() {
    let mut s = init(10, 4, 10);

    // Get the cursor to be 2 rows above a new page
    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 3 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    // Write 3 lines of text, forcing the last line into the first
    // row of a new page. Move our cursor onto the previous page.
    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1AB\n2BC\n3DE\n4FG").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // Ensure we're still on the first page. So our cursor is on the first
    // page but we have two pages of data.
    assert!(s.cursor_pin().node == s.pages.first.unwrap());

    //      +----------+ = PAGE 0
    //  ... :          :
    //     +-------------+ ACTIVE
    // 4305 |1AB0000000| | 0
    // 4306 |2BC0000000| | 1
    //      :^         : : = PIN 0
    // 4307 |3DE0000000| | 2
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |4FG0000000| | 3
    //      +----------+ :
    //     +-------------+
    s.cursor_scroll_above().unwrap();

    //     +----------+ = PAGE 0
    //  ... :          :
    // 4305 |1AB0000000|
    //     +-------------+ ACTIVE
    // 4306 |2BC0000000| | 0
    // 4307 |          | | 1
    //      :^         : : = PIN 0
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |3DE0000000| | 2
    //    1 |4FG0000000| | 3
    //      +----------+ :
    //     +-------------+

    assert_eq!("2BC\n\n3DE\n4FG", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0's penultimate row is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // Page 0's final row is dirty because it was cleared.
    assert!(s.pages.is_dirty(active(0, 1)));
    // Page 1's row is dirty because it's new.
    assert!(s.pages.is_dirty(active(0, 2)));
}

#[test]
fn screen_scroll_above_no_scrollback_bottom_of_page() {
    let mut s = init(10, 3, 0);

    let first = s.pages.first.unwrap();
    let first_page_size = s.page(first).capacity.rows;
    s.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 3 {
        s.test_write_string("\n").unwrap();
    }
    s.page_mut(first).pause_integrity_checks(false);

    s.set_attribute(bg155()).unwrap();
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s.cursor_absolute(0, 1);
    s.pages.clear_dirty();

    // At this point:
    //  +-------------+ ACTIVE
    //   +----------+ : = PAGE 0
    // 0 |1ABCD00000| | 0
    // 1 |2EFGH00000| | 1
    //   :^         : : = PIN 0
    // 2 |3IJKL00000| | 2
    //   +----------+ :
    //  +-------------+

    s.cursor_scroll_above().unwrap();

    //   +----------+ = PAGE 0
    // 0 |1ABCD00000|
    //  +-------------+ ACTIVE
    // 1 |2EFGH00000| | 0
    // 2 |          | | 1
    //   :^         : : = PIN 0
    // 3 |3IJKL00000| | 2
    //   +----------+ :
    //  +-------------+

    assert_eq!("2EFGH\n\n3IJKL", dump(&s, viewport(0, 0)));
    assert_bg155(&s, 0, 1);

    // Page 0 row 1 (active row 0) is dirty because the cursor moved off of it.
    assert!(s.pages.is_dirty(active(0, 0)));
    // Page 0 row 2 (active row 1) is dirty because it was cleared.
    assert!(s.pages.is_dirty(active(0, 1)));
    // Page 0 row 3 (active row 2) is dirty because it is new.
    assert!(s.pages.is_dirty(active(0, 2)));
}

#[test]
fn screen_clone() {
    let mut s = init(10, 3, 10);
    s.test_write_string("1ABCD\n2EFGH").unwrap();
    assert_eq!("1ABCD\n2EFGH", dump(&s, active(0, 0)));
    assert_eq!(5, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    // Clone
    let s2 = s.clone_screen(active(0, 0), None).unwrap();
    assert_eq!("1ABCD\n2EFGH", dump(&s2, active(0, 0)));
    assert_eq!(5, s2.cursor.x);
    assert_eq!(1, s2.cursor.y);

    // Write to s1, should not be in s2
    s.test_write_string("\n34567").unwrap();
    assert_eq!("1ABCD\n2EFGH\n34567", dump(&s, active(0, 0)));
    assert_eq!("1ABCD\n2EFGH", dump(&s2, active(0, 0)));
    assert_eq!(5, s2.cursor.x);
    assert_eq!(1, s2.cursor.y);
}

#[test]
fn screen_clone_partial() {
    let mut s = init(10, 3, 10);
    s.test_write_string("1ABCD\n2EFGH").unwrap();
    assert_eq!("1ABCD\n2EFGH", dump(&s, active(0, 0)));
    assert_eq!(5, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    // Clone
    let s2 = s.clone_screen(active(0, 1), None).unwrap();
    assert_eq!("2EFGH", dump(&s2, active(0, 0)));

    // Cursor is shifted since we cloned partial
    assert_eq!(5, s2.cursor.x);
    assert_eq!(0, s2.cursor.y);
}

#[test]
fn screen_clone_partial_cursor_out_of_bounds() {
    let mut s = init(10, 3, 10);
    s.test_write_string("1ABCD\n2EFGH").unwrap();
    assert_eq!("1ABCD\n2EFGH", dump(&s, active(0, 0)));
    assert_eq!(5, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    // Clone
    let s2 = s.clone_screen(active(0, 0), Some(active(0, 0))).unwrap();
    assert_eq!("1ABCD", dump(&s2, active(0, 0)));

    // Cursor is shifted since we cloned partial
    assert_eq!(0, s2.cursor.x);
    assert_eq!(0, s2.cursor.y);
}

/// Common setup: 5x3 with scrollback 1 containing three lines.
fn sel_screen() -> Screen {
    let mut s = init(5, 3, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    s
}

fn sel_points(s: &Screen) -> (Point, Point) {
    let sel = s.selection.unwrap();
    (
        pt_from_pin(s, Tag::Active, sel.start(&s.pages)),
        pt_from_pin(s, Tag::Active, sel.end(&s.pages)),
    )
}

#[test]
fn screen_clone_contains_full_selection() {
    let mut s = sel_screen();

    // Select a single line
    let sel = Selection::new(
        pin(&s, active(0, 1)),
        pin(&s, active(s.pages.cols - 1, 1)),
        false,
    );
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 0), None).unwrap();

    // Our selection should remain valid
    let (a, b) = sel_points(&s2);
    assert_eq!(active(0, 1), a);
    assert_eq!(active(s2.pages.cols - 1, 1), b);
}

#[test]
fn screen_clone_contains_none_of_selection() {
    let mut s = sel_screen();

    // Select a single line
    let sel = Selection::new(
        pin(&s, active(0, 0)),
        pin(&s, active(s.pages.cols - 1, 0)),
        false,
    );
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 1), None).unwrap();

    // Our selection should be null
    assert!(s2.selection.is_none());
}

#[test]
fn screen_clone_contains_selection_start_cutoff() {
    let mut s = sel_screen();

    // Select a single line
    let sel = Selection::new(
        pin(&s, active(0, 0)),
        pin(&s, active(s.pages.cols - 1, 1)),
        false,
    );
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 1), None).unwrap();

    // Our selection should remain valid
    let (a, b) = sel_points(&s2);
    assert_eq!(active(0, 0), a);
    assert_eq!(active(s2.pages.cols - 1, 0), b);
}

#[test]
fn screen_clone_contains_selection_end_cutoff() {
    let mut s = sel_screen();

    // Select a single line
    let sel = Selection::new(pin(&s, active(0, 1)), pin(&s, active(2, 2)), false);
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 0), Some(active(0, 1))).unwrap();

    // Our selection should remain valid
    let (a, b) = sel_points(&s2);
    assert_eq!(active(0, 1), a);
    assert_eq!(active(s2.pages.cols - 1, 2), b);
}

#[test]
fn screen_clone_contains_selection_end_cutoff_reversed() {
    let mut s = sel_screen();

    // Select a single line
    let sel = Selection::new(pin(&s, active(2, 2)), pin(&s, active(0, 1)), false);
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 0), Some(active(0, 1))).unwrap();

    // Our selection should remain valid
    let (a, b) = sel_points(&s2);
    assert_eq!(active(0, 1), a);
    assert_eq!(active(s2.pages.cols - 1, 2), b);
}

#[test]
fn screen_clone_contains_subset_of_selection() {
    let mut s = init(5, 4, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n4ABCD").unwrap();

    // Select the full screen
    let sel = Selection::new(pin(&s, active(0, 0)), pin(&s, active(0, 3)), false);
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 1), Some(active(0, 2))).unwrap();

    // Our selection should remain valid
    let (a, b) = sel_points(&s2);
    assert_eq!(active(0, 0), a);
    assert_eq!(active(s2.pages.cols - 1, 3), b);
}

#[test]
fn screen_clone_contains_subset_of_rectangle_selection() {
    let mut s = init(5, 4, 1);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n4ABCD").unwrap();

    // Select the full screen from x=1 to x=3
    let sel = Selection::new(pin(&s, active(1, 0)), pin(&s, active(3, 3)), true);
    s.select(Some(sel)).unwrap();

    // Clone
    let s2 = s.clone_screen(active(0, 1), Some(active(0, 2))).unwrap();

    // Our selection should remain valid and be properly clipped
    // preserving the columns of the start and end points of the
    // selection.
    let (a, b) = sel_points(&s2);
    assert_eq!(active(1, 0), a);
    assert_eq!(active(3, 3), b);
}

#[test]
fn screen_clone_basic() {
    let mut s = init(10, 3, 0);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();

    {
        let s2 = s.clone_screen(active(0, 1), Some(active(0, 1))).unwrap();

        // Test our contents rotated
        assert_eq!("2EFGH", dump(&s2, active(0, 0)));
    }

    {
        let s2 = s.clone_screen(active(0, 1), Some(active(0, 2))).unwrap();

        // Test our contents rotated
        assert_eq!("2EFGH\n3IJKL", dump(&s2, active(0, 0)));
    }
}
