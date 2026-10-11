use super::*;
use crate::page::ContentTag;
use crate::point::Tag;
use ghostty_vt::color::Rgb;
use ghostty_vt::modes::Mode;
use ghostty_vt::sgr::Attribute as A;

fn cr_lf(t: &mut Terminal) {
    t.carriage_return();
    t.linefeed().unwrap();
}

/// "ABC\nDEF\nGHI" written with CR+LF between lines (no trailing CR/LF).
fn abc_def_ghi(t: &mut Terminal) {
    print_str(t, "ABC");
    cr_lf(t);
    print_str(t, "DEF");
    cr_lf(t);
    print_str(t, "GHI");
}

fn dirty(t: &Terminal, y: u32) -> bool {
    t.is_dirty(pt_active(0, y))
}

fn print_chars(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

/// (row.hyperlink, cell.hyperlink, lookup id, hyperlink_set.count)
fn link_info(t: &Terminal, pt: Point) -> (bool, bool, Option<u16>, usize) {
    let p = pin_at(t, pt);
    let page = t.screen().page(p.node);
    let (row, cell) = page
        .get_row_and_cell(usize::from(p.x), usize::from(p.y))
        .unwrap();
    let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
    (
        row.hyperlink(),
        cell.hyperlink(),
        page.lookup_hyperlink(i),
        page.hyperlink_set.count(),
    )
}

fn expect_linked(t: &Terminal, x: u16, y: u32) {
    let (row, cell, id, count) = link_info(t, pt_viewport(x, y));
    assert!(row);
    assert!(cell);
    assert_eq!(Some(1), id);
    assert_eq!(1, count);
}

fn expect_unlinked_cell(t: &Terminal, x: u16, y: u32) {
    let (_, cell, id, _) = link_info(t, pt_viewport(x, y));
    assert!(!cell);
    assert!(id.is_none());
}

fn expect_unlinked_row(t: &Terminal, x: u16, y: u32) {
    let (row, cell, id, _) = link_info(t, pt_viewport(x, y));
    assert!(!row);
    assert!(!cell);
    assert!(id.is_none());
}

#[test]
fn terminal_set_top_and_bottom_margin_top_and_bottom() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(1, 2);

    t.clear_dirty();
    t.scroll_down(1);

    assert!(dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(!dirty(&t, 2));
    assert_eq!("\nABC\nGHI", plain(&t));
}

#[test]
fn terminal_set_top_and_bottom_margin_top_equal_to_bottom() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(2, 2);

    t.clear_dirty();
    t.scroll_down(1);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }
    assert_eq!("\nABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_set_left_and_right_margin_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(0, 0);

    t.clear_dirty();
    t.erase_chars(1);

    assert!(dirty(&t, 0));
    assert!(!dirty(&t, 1));
    assert_eq!(" BC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_set_left_and_right_margin_left_only() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(2, 0);
    assert_eq!(1, t.scrolling_region.left);
    assert_eq!(t.cols - 1, t.scrolling_region.right);
    t.set_cursor_pos(1, 2);

    t.clear_dirty();
    t.insert_lines(1);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }
    assert_eq!("A\nDBC\nGEF\n HI", plain(&t));
}

#[test]
fn terminal_set_left_and_right_margin_left_and_right() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(1, 2);
    t.set_cursor_pos(1, 2);

    t.clear_dirty();
    t.insert_lines(1);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }
    assert_eq!("  C\nABF\nDEI\nGH", plain(&t));
}

#[test]
fn terminal_set_left_and_right_margin_left_equal_right() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(2, 2);
    t.set_cursor_pos(1, 2);

    t.clear_dirty();
    t.insert_lines(1);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }
    assert_eq!("\nABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_set_left_and_right_margin_mode_69_unset() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.modes.set(Mode::EnableLeftAndRightMargin, false);
    t.set_left_and_right_margin(1, 2);
    t.set_cursor_pos(1, 2);

    t.clear_dirty();
    t.insert_lines(1);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }
    assert_eq!("\nABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_insert_lines_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);

    t.clear_dirty();
    t.insert_lines(1);

    assert!(!dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));
    assert!(dirty(&t, 3));
    assert_eq!("ABC\n\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_insert_lines_colors_with_bg_color() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);

    t.set_attribute(A::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0,
    }))
    .unwrap();
    t.insert_lines(1);

    assert_eq!("ABC\n\nDEF\nGHI", plain(&t));

    for x in 0..t.cols {
        let cell = cell_at(&t, pt_active(x, 1));
        assert!(cell.content_tag() == ContentTag::BgColorRgb);
        assert_eq!(
            Rgb {
                r: 0xFF,
                g: 0,
                b: 0
            },
            cell.color_rgb()
        );
    }
}

#[test]
fn terminal_insert_lines_handles_style_refs() {
    let mut t = init(5, 3);

    print_str(&mut t, "ABC");
    cr_lf(&mut t);
    print_str(&mut t, "DEF");
    cr_lf(&mut t);

    // For the line being deleted, create a refcounted style
    t.set_attribute(A::Bold).unwrap();
    print_str(&mut t, "GHI");
    t.set_attribute(A::Unset).unwrap();

    // verify we have styles in our style map
    assert_eq!(1, t.screen().cursor_page().styles.count());

    t.set_cursor_pos(2, 2);
    t.insert_lines(1);

    assert_eq!("ABC\n\nDEF", plain(&t));

    // verify we have no styles in our style map
    assert_eq!(0, t.screen().cursor_page().styles.count());
}

#[test]
fn terminal_insert_lines_outside_of_scroll_region() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(3, 4);
    t.set_cursor_pos(2, 2);

    t.clear_dirty();
    t.insert_lines(1);

    assert!(!dirty(&t, 0));
    assert!(!dirty(&t, 1));
    assert!(!dirty(&t, 2));
    assert_eq!("ABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_insert_lines_top_bottom_scroll_region() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    cr_lf(&mut t);
    print_str(&mut t, "123");
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(2, 2);

    t.clear_dirty();
    t.insert_lines(1);

    assert!(!dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));
    assert!(!dirty(&t, 3));
    assert_eq!("ABC\n\nDEF\n123", plain(&t));
}

#[test]
fn terminal_insert_lines_across_page_boundary_marks_all_shifted_rows_dirty() {
    let mut t = init_scrollback(10, 5, 1024);

    let first = t.screen().pages.first.unwrap();
    let first_page_nrows = usize::from(t.screen().page(first).capacity.rows);

    // Fill up the first page minus 3 rows
    for _ in 0..first_page_nrows - 3 {
        t.linefeed().unwrap();
    }

    // Add content that will cross a page boundary
    print_str(&mut t, "1AAAA");
    cr_lf(&mut t);
    print_str(&mut t, "2BBBB");
    cr_lf(&mut t);
    print_str(&mut t, "3CCCC");
    cr_lf(&mut t);
    print_str(&mut t, "4DDDD");
    cr_lf(&mut t);
    print_str(&mut t, "5EEEE");

    // Verify we now have a second page
    assert!(t.screen().pages.node(first).unwrap().next.is_some());

    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_lines(1);

    for y in 0..5 {
        assert!(dirty(&t, y));
    }
    assert_eq!("\n1AAAA\n2BBBB\n3CCCC\n4DDDD", plain(&t));
}

#[test]
fn terminal_insert_lines_legacy_test() {
    let mut t = init(2, 5);

    // Initial value
    for (i, c) in "ABCDE".chars().enumerate() {
        if i > 0 {
            cr_lf(&mut t);
        }
        t.print(u32::from(c)).unwrap();
    }

    // Move to row 2
    t.set_cursor_pos(2, 1);

    // Insert two lines
    t.insert_lines(2);

    assert_eq!("A\n\n\nB\nC", plain(&t));
}

#[test]
fn terminal_insert_lines_zero() {
    let mut t = init(2, 5);

    // This should do nothing
    t.set_cursor_pos(1, 1);
    t.insert_lines(0);
}

#[test]
fn terminal_insert_lines_with_scroll_region() {
    let mut t = init(2, 6);

    // Initial value
    for (i, c) in "ABCDE".chars().enumerate() {
        if i > 0 {
            cr_lf(&mut t);
        }
        t.print(u32::from(c)).unwrap();
    }

    t.set_top_and_bottom_margin(1, 2);
    t.set_cursor_pos(1, 1);

    t.clear_dirty();
    t.insert_lines(1);

    assert!(dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(!dirty(&t, 2));

    t.print(u32::from('X')).unwrap();

    assert_eq!("X\nA\nC\nD\nE", plain(&t));
}

#[test]
fn terminal_insert_lines_more_than_remaining() {
    let mut t = init(2, 5);

    // Initial value
    for (i, c) in "ABCDE".chars().enumerate() {
        if i > 0 {
            cr_lf(&mut t);
        }
        t.print(u32::from(c)).unwrap();
    }

    // Move to row 2
    t.set_cursor_pos(2, 1);

    // Insert a bunch of  lines
    t.clear_dirty();
    t.insert_lines(20);

    assert!(!dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));

    assert_eq!("A", plain(&t));
}

#[test]
fn terminal_insert_lines_resets_pending_wrap() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.insert_lines(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('B')).unwrap();

    assert_eq!("B\nABCDE", plain(&t));
}

#[test]
fn terminal_insert_lines_resets_wrap() {
    let mut t = init(3, 3);

    t.print(u32::from('1')).unwrap();
    cr_lf(&mut t);
    print_chars(&mut t, "ABCDEF");
    t.set_cursor_pos(1, 1);
    t.insert_lines(1);
    t.print(u32::from('X')).unwrap();

    assert_eq!("X\n1\nABC", plain(&t));

    assert!(!row_at(&t, pt_active(0, 2)).wrap());
}

#[test]
fn terminal_insert_lines_multi_codepoint_graphemes() {
    let mut t = init(5, 5);

    // Disable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    print_str(&mut t, "ABC");
    cr_lf(&mut t);

    // This is: 👨‍👩‍👧 (which may or may not render correctly)
    t.print(0x1F468).unwrap();
    t.print(0x200D).unwrap();
    t.print(0x1F469).unwrap();
    t.print(0x200D).unwrap();
    t.print(0x1F467).unwrap();

    cr_lf(&mut t);
    print_str(&mut t, "GHI");
    t.set_cursor_pos(2, 2);
    t.insert_lines(1);

    assert_eq!("ABC\n\n👨‍👩‍👧\nGHI", plain(&t));
}

#[test]
fn terminal_insert_lines_left_right_scroll_region() {
    let mut t = init(10, 10);

    print_str(&mut t, "ABC123");
    cr_lf(&mut t);
    print_str(&mut t, "DEF456");
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(2, 2);

    t.clear_dirty();
    t.insert_lines(1);

    assert!(!dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));
    assert!(dirty(&t, 3));

    assert_eq!("ABC123\nD   56\nGEF489\n HI7", plain(&t));
}

#[test]
fn terminal_scroll_up_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    let viewport_before = t.screen().pages.get_top_left(Tag::Viewport).unwrap();
    t.scroll_up(1).unwrap();
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    // Viewport should have moved. Our entire page should've scrolled!
    // The viewport moving will cause our render state to make the full
    // frame as dirty.
    let viewport_after = t.screen().pages.get_top_left(Tag::Viewport).unwrap();
    assert!(!viewport_before.eql(viewport_after));

    assert_eq!("DEF\nGHI", plain(&t));
}

#[test]
fn terminal_scroll_up_moves_hyperlink() {
    let mut t = init(5, 5);

    print_str(&mut t, "ABC");
    cr_lf(&mut t);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "DEF");
    t.screen_mut().end_hyperlink();
    cr_lf(&mut t);
    print_str(&mut t, "GHI");
    t.set_cursor_pos(2, 2);
    t.scroll_up(1).unwrap();

    assert_eq!("DEF\nGHI", plain(&t));

    for x in 0..3 {
        expect_linked(&t, x, 0);
    }
    for x in 0..3 {
        expect_unlinked_row(&t, x, 1);
    }
}

#[test]
fn terminal_scroll_up_clears_hyperlink() {
    let mut t = init(5, 5);

    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "ABC");
    t.screen_mut().end_hyperlink();
    cr_lf(&mut t);
    print_str(&mut t, "DEF");
    cr_lf(&mut t);
    print_str(&mut t, "GHI");
    t.set_cursor_pos(2, 2);
    t.scroll_up(1).unwrap();

    assert_eq!("DEF\nGHI", plain(&t));

    for x in 0..3 {
        expect_unlinked_row(&t, x, 0);
    }
}

#[test]
fn terminal_scroll_up_top_bottom_scroll_region() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(2, 3);
    t.set_cursor_pos(1, 1);

    t.clear_dirty();
    t.scroll_up(1).unwrap();

    // This is dirty because the cursor moves from this row
    assert!(dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));

    assert_eq!("ABC\nGHI", plain(&t));
}

#[test]
fn terminal_scroll_up_left_right_scroll_region() {
    let mut t = init(10, 10);

    print_str(&mut t, "ABC123");
    cr_lf(&mut t);
    print_str(&mut t, "DEF456");
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(2, 2);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    t.clear_dirty();
    t.scroll_up(1).unwrap();
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    assert!(dirty(&t, 0));
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));

    assert_eq!("AEF423\nDHI756\nG   89", plain(&t));
}

#[test]
fn terminal_scroll_up_left_right_scroll_region_hyperlink() {
    let mut t = init(10, 10);

    print_str(&mut t, "ABC123");
    cr_lf(&mut t);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "DEF456");
    t.screen_mut().end_hyperlink();
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(2, 2);
    t.scroll_up(1).unwrap();

    assert_eq!("AEF423\nDHI756\nG   89", plain(&t));

    // First row gets some hyperlinks
    for x in 0..1 {
        expect_unlinked_cell(&t, x, 0);
    }
    for x in 1..4 {
        expect_linked(&t, x, 0);
    }
    for x in 4..6 {
        expect_unlinked_cell(&t, x, 0);
    }

    // Second row preserves hyperlink where we didn't scroll
    for x in 0..1 {
        expect_linked(&t, x, 1);
    }
    for x in 1..4 {
        expect_unlinked_cell(&t, x, 1);
    }
    for x in 4..6 {
        expect_linked(&t, x, 1);
    }
}

#[test]
fn terminal_scroll_up_preserves_pending_wrap() {
    let mut t = init(5, 5);

    t.set_cursor_pos(1, 5);
    t.print(u32::from('A')).unwrap();
    t.set_cursor_pos(2, 5);
    t.print(u32::from('B')).unwrap();
    t.set_cursor_pos(3, 5);
    t.print(u32::from('C')).unwrap();
    t.scroll_up(1).unwrap();
    t.print(u32::from('X')).unwrap();

    assert_eq!("    B\n    C\n\nX", plain(&t));
}

#[test]
fn terminal_scroll_up_full_top_bottom_region() {
    let mut t = init(5, 5);

    print_str(&mut t, "top");
    t.set_cursor_pos(5, 1);
    print_str(&mut t, "ABCDE");
    t.set_top_and_bottom_margin(2, 5);

    t.clear_dirty();
    t.scroll_up(4).unwrap();

    // This is dirty because the cursor moves from this row
    assert!(dirty(&t, 0));
    assert!(dirty(&t, 1));

    assert_eq!("top", plain(&t));
}

#[test]
fn terminal_scroll_up_full_top_bottomleft_right_scroll_region() {
    let mut t = init(5, 5);

    print_str(&mut t, "top");
    t.set_cursor_pos(5, 1);
    print_str(&mut t, "ABCDE");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_top_and_bottom_margin(2, 5);
    t.set_left_and_right_margin(2, 4);

    t.clear_dirty();
    t.scroll_up(4).unwrap();

    // This is dirty because the cursor moves from this row
    assert!(dirty(&t, 0));
    for y in 1..5 {
        assert!(dirty(&t, y));
    }

    assert_eq!("top\n\n\n\nA   E", plain(&t));
}

#[test]
fn terminal_scroll_up_creates_scrollback_in_primary_screen() {
    // When in primary screen with full-width scroll region at top,
    // scrollUp (CSI S) should push lines into scrollback like xterm.
    let mut t = init_scrollback(5, 5, 10);

    // Fill the screen with content
    print_str(&mut t, "AAAAA");
    cr_lf(&mut t);
    print_str(&mut t, "BBBBB");
    cr_lf(&mut t);
    print_str(&mut t, "CCCCC");
    cr_lf(&mut t);
    print_str(&mut t, "DDDDD");
    cr_lf(&mut t);
    print_str(&mut t, "EEEEE");

    t.clear_dirty();

    // Scroll up by 1, which should push "AAAAA" into scrollback
    t.scroll_up(1).unwrap();

    // The cursor row (new empty row) should be dirty
    assert!(t.screen().cursor_row().dirty());

    // The active screen should now show BBBBB through EEEEE plus one blank line
    assert_eq!("BBBBB\nCCCCC\nDDDDD\nEEEEE", plain(&t));

    // Now scroll to the top to see scrollback - AAAAA should be there
    t.screen_mut().scroll(Scroll::Top);
    assert_eq!("AAAAA\nBBBBB\nCCCCC\nDDDDD\nEEEEE", plain(&t));
}

#[test]
fn terminal_scroll_up_with_max_scrollback_zero() {
    // When max_scrollback is 0, scrollUp should still work but not retain history
    let mut t = init_scrollback(5, 5, 0);

    print_str(&mut t, "AAAAA");
    cr_lf(&mut t);
    print_str(&mut t, "BBBBB");
    cr_lf(&mut t);
    print_str(&mut t, "CCCCC");

    t.scroll_up(1).unwrap();

    // Active screen should show scrolled content
    assert_eq!("BBBBB\nCCCCC", plain(&t));

    // Scroll to top - should be same as active since no scrollback
    t.screen_mut().scroll(Scroll::Top);
    assert_eq!("BBBBB\nCCCCC", plain(&t));
}

#[test]
fn terminal_scroll_up_with_max_scrollback_zero_and_top_margin() {
    // When max_scrollback is 0 and top margin is set, should use deleteLines path
    let mut t = init_scrollback(5, 5, 0);

    print_str(&mut t, "AAAAA");
    cr_lf(&mut t);
    print_str(&mut t, "BBBBB");
    cr_lf(&mut t);
    print_str(&mut t, "CCCCC");
    cr_lf(&mut t);
    print_str(&mut t, "DDDDD");

    // Set top margin (not at row 0)
    t.set_top_and_bottom_margin(2, 5);

    t.scroll_up(1).unwrap();

    // First row preserved, rest scrolled
    assert_eq!("AAAAA\nCCCCC\nDDDDD", plain(&t));
}

#[test]
fn terminal_scroll_up_with_max_scrollback_zero_and_left_right_margin() {
    // When max_scrollback is 0 with left/right margins, uses deleteLines path
    let mut t = init_scrollback(10, 5, 0);

    print_str(&mut t, "AAAAABBBBB");
    cr_lf(&mut t);
    print_str(&mut t, "CCCCCDDDDD");
    cr_lf(&mut t);
    print_str(&mut t, "EEEEEFFFFF");

    // Set left/right margins (columns 2-6, 1-indexed = indices 1-5)
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(2, 6);

    t.scroll_up(1).unwrap();

    // cols 1-5 scroll, col 0 and cols 6+ preserved
    assert_eq!("ACCCCDBBBB\nCEEEEFDDDD\nE     FFFF", plain(&t));
}

#[test]
fn terminal_scroll_down_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    t.clear_dirty();
    t.scroll_down(1);
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    for y in 0..5 {
        assert!(dirty(&t, y));
    }

    assert_eq!("\nABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_scroll_down_hyperlink_moves() {
    let mut t = init(5, 5);

    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "ABC");
    t.screen_mut().end_hyperlink();
    cr_lf(&mut t);
    print_str(&mut t, "DEF");
    cr_lf(&mut t);
    print_str(&mut t, "GHI");
    t.set_cursor_pos(2, 2);
    t.scroll_down(1);

    assert_eq!("\nABC\nDEF\nGHI", plain(&t));

    for x in 0..3 {
        expect_linked(&t, x, 1);
    }
    for x in 0..3 {
        expect_unlinked_row(&t, x, 0);
    }
}

#[test]
fn terminal_scroll_down_outside_of_scroll_region() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(3, 4);
    t.set_cursor_pos(2, 2);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    t.clear_dirty();
    t.scroll_down(1);
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    assert!(!dirty(&t, 0));

    // This is dirty because the cursor moves from this row
    assert!(dirty(&t, 1));
    assert!(dirty(&t, 2));
    assert!(dirty(&t, 3));

    assert_eq!("ABC\nDEF\n\nGHI", plain(&t));
}

#[test]
fn terminal_scroll_down_left_right_scroll_region() {
    let mut t = init(10, 10);

    print_str(&mut t, "ABC123");
    cr_lf(&mut t);
    print_str(&mut t, "DEF456");
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(2, 2);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    t.clear_dirty();
    t.scroll_down(1);
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }

    assert_eq!("A   23\nDBC156\nGEF489\n HI7", plain(&t));
}

#[test]
fn terminal_scroll_down_left_right_scroll_region_hyperlink() {
    let mut t = init(10, 10);

    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "ABC123");
    t.screen_mut().end_hyperlink();
    cr_lf(&mut t);
    print_str(&mut t, "DEF456");
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(2, 2);
    t.scroll_down(1);

    assert_eq!("A   23\nDBC156\nGEF489\n HI7", plain(&t));

    // First row preserves hyperlink where we didn't scroll
    for x in 0..1 {
        expect_linked(&t, x, 0);
    }
    for x in 1..4 {
        expect_unlinked_cell(&t, x, 0);
    }
    for x in 4..6 {
        expect_linked(&t, x, 0);
    }

    // Second row gets some hyperlinks
    for x in 0..1 {
        expect_unlinked_cell(&t, x, 1);
    }
    for x in 1..4 {
        expect_linked(&t, x, 1);
    }
    for x in 4..6 {
        expect_unlinked_cell(&t, x, 1);
    }
}

#[test]
fn terminal_scroll_down_outside_of_left_right_scroll_region() {
    let mut t = init(10, 10);

    print_str(&mut t, "ABC123");
    cr_lf(&mut t);
    print_str(&mut t, "DEF456");
    cr_lf(&mut t);
    print_str(&mut t, "GHI789");
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t.set_cursor_pos(1, 1);

    let (cx, cy) = (t.screen().cursor.x, t.screen().cursor.y);
    t.clear_dirty();
    t.scroll_down(1);
    assert_eq!(cx, t.screen().cursor.x);
    assert_eq!(cy, t.screen().cursor.y);

    for y in 0..4 {
        assert!(dirty(&t, y));
    }

    assert_eq!("A   23\nDBC156\nGEF489\n HI7", plain(&t));
}

#[test]
fn terminal_scroll_down_preserves_pending_wrap() {
    let mut t = init(5, 10);

    t.set_cursor_pos(1, 5);
    t.print(u32::from('A')).unwrap();
    t.set_cursor_pos(2, 5);
    t.print(u32::from('B')).unwrap();
    t.set_cursor_pos(3, 5);
    t.print(u32::from('C')).unwrap();
    t.scroll_down(1);
    t.print(u32::from('X')).unwrap();

    assert_eq!("\n    A\n    B\nX   C", plain(&t));
}

#[test]
fn terminal_erase_chars_simple_operation() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.erase_chars(2);
    t.print(u32::from('X')).unwrap();

    assert!(dirty(&t, 0));
    assert!(!dirty(&t, 1));

    assert_eq!("X C", plain(&t));
}

#[test]
fn terminal_erase_chars_minimum_one() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.erase_chars(0);
    t.print(u32::from('X')).unwrap();
    assert!(dirty(&t, 0));

    assert_eq!("XBC", plain(&t));
}

#[test]
fn terminal_erase_chars_beyond_screen_edge() {
    let mut t = init(5, 5);

    print_chars(&mut t, "  ABC");
    t.set_cursor_pos(1, 4);
    t.erase_chars(10);

    assert_eq!("  A", plain(&t));
}

#[test]
fn terminal_erase_chars_wide_character() {
    let mut t = init(5, 5);

    t.print(u32::from('橋')).unwrap();
    print_chars(&mut t, "BC");
    t.set_cursor_pos(1, 1);
    t.erase_chars(1);
    t.print(u32::from('X')).unwrap();

    assert_eq!("X BC", plain(&t));
}

#[test]
fn terminal_erase_chars_resets_pending_wrap() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.erase_chars(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('X')).unwrap();

    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_erase_chars_resets_wrap() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABCDE123");
    assert!(row_at(&t, pt_active(0, 0)).wrap());

    t.set_cursor_pos(1, 1);
    t.erase_chars(1);

    assert!(!row_at(&t, pt_active(0, 0)).wrap());

    t.print(u32::from('X')).unwrap();

    assert_eq!("XBCDE\n123", plain(&t));
}
