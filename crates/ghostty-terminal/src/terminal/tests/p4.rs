use super::*;
use ghostty_vt::color::Rgb;
use ghostty_vt::sgr::Attribute as A;

const RED: Rgb = Rgb {
    r: 0xFF,
    g: 0,
    b: 0,
};

fn chars(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

fn set_bg_red(t: &mut Terminal) {
    t.set_attribute(A::DirectColorBg(RED)).unwrap();
}

fn assert_bg_red(t: &Terminal, pt: Point) {
    let c = cell_at(t, pt);
    assert_eq!(ContentTag::BgColorRgb, c.content_tag());
    assert_eq!(RED, c.color_rgb());
}

fn hl_id(t: &Terminal, pt: Point) -> (bool, bool, Option<u16>) {
    let p = pin_at(t, pt);
    let page = t.screen().page(p.node);
    let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
    (
        page.rows[usize::from(p.y)].hyperlink(),
        page.cells[i].hyperlink(),
        page.lookup_hyperlink(i),
    )
}

#[test]
fn terminal_erase_chars_preserves_background_sgr() {
    let mut t = init(10, 10);
    chars(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    set_bg_red(&mut t);
    t.erase_chars(2);
    assert_eq!("  C", plain(&t));
    assert_bg_red(&t, pt_active(0, 0));
    assert_bg_red(&t, pt_active(1, 0));
}

#[test]
fn terminal_erase_chars_handles_refcounted_styles() {
    let mut t = init(10, 10);
    t.set_attribute(A::Bold).unwrap();
    t.print('A' as u32).unwrap();
    t.print('B' as u32).unwrap();
    t.set_attribute(A::Unset).unwrap();
    t.print('C' as u32).unwrap();

    assert_eq!(1, t.screen().cursor_page().styles.count());
    t.set_cursor_pos(1, 1);
    t.erase_chars(2);
    assert_eq!(0, t.screen().cursor_page().styles.count());
}

#[test]
fn terminal_erase_chars_protected_attributes_respected_with_iso() {
    let mut t = init(5, 5);
    t.set_protected_mode(ProtectedMode::Iso);
    chars(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.erase_chars(2);
    assert_eq!("ABC", plain(&t));
}

#[test]
fn terminal_erase_chars_protected_attributes_ignored_with_dec_most_recent() {
    let mut t = init(5, 5);
    t.set_protected_mode(ProtectedMode::Iso);
    chars(&mut t, "ABC");
    t.set_protected_mode(ProtectedMode::Dec);
    t.set_protected_mode(ProtectedMode::Off);
    t.set_cursor_pos(1, 1);
    t.erase_chars(2);
    assert_eq!("  C", plain(&t));
}

#[test]
fn terminal_erase_chars_protected_attributes_ignored_with_dec_set() {
    let mut t = init(5, 5);
    t.set_protected_mode(ProtectedMode::Dec);
    chars(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.erase_chars(2);
    assert_eq!("  C", plain(&t));
}

#[test]
fn terminal_erase_chars_wide_char_boundary_conditions() {
    let mut t = init(8, 1);
    print_str(&mut t, "😀a😀b😀");
    assert_eq!("😀a😀b😀", plain(&t));

    t.set_cursor_pos(1, 2);
    t.erase_chars(3);
    t.screen().cursor_page().assert_integrity();
    assert_eq!("     b😀", plain(&t));
}

#[test]
fn terminal_erase_chars_wide_char_splits_proper_cell_boundaries() {
    let mut t = init(30, 1);
    print_str(&mut t, "x食べて下さい");
    assert_eq!("x食べて下さい", plain(&t));

    t.set_cursor_pos(1, 6); // At: て
    t.erase_chars(4); // Delete: て下
    t.screen().cursor_page().assert_integrity();
    assert_eq!("x食べ    さい", plain(&t));
}

#[test]
fn terminal_erase_chars_wide_char_wrap_boundary_conditions() {
    let mut t = init(8, 3);
    print_str(&mut t, ".......😀abcde😀......");
    assert_eq!(".......\n😀abcde\n😀......", plain(&t));
    assert_eq!(".......😀abcde😀......", plain_unwrapped(&t));

    t.set_cursor_pos(2, 2);
    t.erase_chars(3);
    t.screen().cursor_page().assert_integrity();
    assert_eq!(".......\n    cde\n😀......", plain(&t));
    assert_eq!(".......     cde\n😀......", plain_unwrapped(&t));
}

#[test]
fn terminal_reverse_index() {
    let mut t = init(2, 5);
    t.print('A' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.print('B' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.print('C' as u32).unwrap();
    t.reverse_index();
    t.print('D' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    assert_eq!("A\nBD\nC", plain(&t));
}

#[test]
fn terminal_reverse_index_from_the_top() {
    let mut t = init(2, 5);
    t.print('A' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.print('B' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.carriage_return();
    t.linefeed().unwrap();

    t.set_cursor_pos(1, 1);
    t.reverse_index();
    t.print('D' as u32).unwrap();

    t.carriage_return();
    t.linefeed().unwrap();
    t.set_cursor_pos(1, 1);
    t.reverse_index();
    t.print('E' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    assert_eq!("E\nD\nA\nB", plain(&t));
}

#[test]
fn terminal_reverse_index_top_of_scrolling_region() {
    let mut t = init(2, 10);
    t.set_cursor_pos(2, 1);
    for c in ['A', 'B', 'C', 'D'] {
        t.print(c as u32).unwrap();
        t.carriage_return();
        t.linefeed().unwrap();
    }

    t.set_top_and_bottom_margin(2, 5);
    t.set_cursor_pos(2, 1);
    t.reverse_index();
    t.print('X' as u32).unwrap();
    assert_eq!("\nX\nA\nB\nC", plain(&t));
}

#[test]
fn terminal_reverse_index_top_of_screen() {
    let mut t = init(5, 5);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('C' as u32).unwrap();
    t.set_cursor_pos(1, 1);
    t.reverse_index();
    t.print('X' as u32).unwrap();
    assert_eq!("X\nA\nB\nC", plain(&t));
}

#[test]
fn terminal_reverse_index_not_top_of_screen() {
    let mut t = init(5, 5);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('C' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.reverse_index();
    t.print('X' as u32).unwrap();
    assert_eq!("X\nB\nC", plain(&t));
}

#[test]
fn terminal_reverse_index_top_bottom_margins() {
    let mut t = init(5, 5);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('C' as u32).unwrap();
    t.set_top_and_bottom_margin(2, 3);
    t.set_cursor_pos(2, 1);
    t.reverse_index();
    assert_eq!("A\n\nB", plain(&t));
}

#[test]
fn terminal_reverse_index_outside_top_bottom_margins() {
    let mut t = init(5, 5);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('C' as u32).unwrap();
    t.set_top_and_bottom_margin(2, 3);
    t.set_cursor_pos(1, 1);
    t.reverse_index();
    assert_eq!("A\nB\nC", plain(&t));
}

fn abc_def_ghi(t: &mut Terminal) {
    print_str(t, "ABC");
    t.set_cursor_pos(2, 1);
    print_str(t, "DEF");
    t.set_cursor_pos(3, 1);
    print_str(t, "GHI");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(2, 3);
}

#[test]
fn terminal_reverse_index_left_right_margins() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(1, 2);
    t.reverse_index();
    assert_eq!("A\nDBC\nGEF\n HI", plain(&t));
}

#[test]
fn terminal_reverse_index_outside_left_right_margins() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(1, 1);
    t.reverse_index();
    assert_eq!("ABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_index() {
    let mut t = init(2, 5);
    t.index().unwrap();
    t.print('A' as u32).unwrap();
    assert!(t.is_dirty(pt_active(0, 0)));
    assert!(t.is_dirty(pt_active(0, 1)));
    assert_eq!("\nA", plain(&t));
}

#[test]
fn terminal_index_from_the_bottom() {
    let mut t = init(2, 5);
    t.set_cursor_pos(5, 1);
    t.print('A' as u32).unwrap();
    t.cursor_left(1); // undo moving right from 'A'

    t.clear_dirty();
    t.index().unwrap();
    t.print('B' as u32).unwrap();

    assert!(t.is_dirty(pt_active(0, 3)));
    assert!(t.is_dirty(pt_active(0, 4)));
    assert_eq!("\n\n\nA\nB", plain(&t));
}

#[test]
fn terminal_index_scrolling_with_hyperlink() {
    let mut t = init(2, 5);
    t.set_cursor_pos(5, 1);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.print('A' as u32).unwrap();
    t.screen_mut().end_hyperlink();
    t.cursor_left(1); // undo moving right from 'A'
    t.index().unwrap();
    t.print('B' as u32).unwrap();

    assert_eq!("\n\n\nA\nB", plain(&t));

    let (row, cell, id) = hl_id(&t, pt_viewport(0, 3));
    assert!(row);
    assert!(cell);
    assert_eq!(Some(1), id);
    let (row, cell, id) = hl_id(&t, pt_viewport(0, 4));
    assert!(!row);
    assert!(!cell);
    assert_eq!(None, id);
}

#[test]
fn terminal_index_outside_of_scrolling_region() {
    let mut t = init(2, 5);
    assert_eq!(0, t.screen().cursor.y);
    t.set_top_and_bottom_margin(2, 5);
    t.index().unwrap();
    assert_eq!(1, t.screen().cursor.y);
}

#[test]
fn terminal_index_from_the_bottom_outside_of_scroll_region() {
    let mut t = init(2, 5);
    t.set_top_and_bottom_margin(1, 2);
    t.set_cursor_pos(5, 1);
    t.print('A' as u32).unwrap();
    t.clear_dirty();
    t.index().unwrap();
    t.print('B' as u32).unwrap();
    assert!(t.is_dirty(pt_active(0, 4)));
    assert_eq!("\n\n\n\nAB", plain(&t));
}

#[test]
fn terminal_index_no_scroll_region_top_of_screen() {
    let mut t = init(5, 5);
    t.print('A' as u32).unwrap();
    t.clear_dirty();
    t.index().unwrap();
    t.print('X' as u32).unwrap();
    assert!(t.is_dirty(pt_active(0, 0)));
    assert!(t.is_dirty(pt_active(0, 1)));
    assert_eq!("A\n X", plain(&t));
}

#[test]
fn terminal_index_bottom_of_primary_screen() {
    let mut t = init(5, 5);
    t.set_cursor_pos(5, 1);
    t.print('A' as u32).unwrap();
    t.clear_dirty();
    t.index().unwrap();
    t.print('X' as u32).unwrap();
    assert!(t.is_dirty(pt_active(0, 3)));
    assert!(t.is_dirty(pt_active(0, 4)));
    assert_eq!("\n\n\nA\n X", plain(&t));
}

#[test]
fn terminal_index_bottom_of_primary_screen_background_sgr() {
    let mut t = init(5, 5);
    t.set_cursor_pos(5, 1);
    t.print('A' as u32).unwrap();
    set_bg_red(&mut t);
    t.index().unwrap();

    assert_eq!("\n\n\nA", plain(&t));
    for x in 0..5 {
        assert_bg_red(&t, pt_active(x, 4));
    }
}

#[test]
fn terminal_index_inside_scroll_region() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.print('A' as u32).unwrap();
    t.clear_dirty();
    t.index().unwrap();
    t.print('X' as u32).unwrap();
    assert!(t.is_dirty(pt_active(0, 0)));
    assert!(t.is_dirty(pt_active(0, 1)));
    assert_eq!("A\n X", plain(&t));
}

#[test]
fn terminal_index_bottom_of_scroll_region_with_hyperlinks() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 2);
    t.print('A' as u32).unwrap();
    t.index().unwrap();
    t.carriage_return();
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.print('B' as u32).unwrap();
    t.screen_mut().end_hyperlink();
    t.index().unwrap();
    t.carriage_return();
    t.print('C' as u32).unwrap();

    assert_eq!("B\nC", plain(&t));

    let (row, cell, id) = hl_id(&t, pt_viewport(0, 0));
    assert!(row);
    assert!(cell);
    assert_eq!(Some(1), id);
    let (row, cell, id) = hl_id(&t, pt_viewport(0, 1));
    assert!(!row);
    assert!(!cell);
    assert_eq!(None, id);
}

#[test]
fn terminal_index_bottom_of_scroll_region_clear_hyperlinks() {
    let mut t = init_scrollback(5, 5, 0);
    t.set_top_and_bottom_margin(2, 3);
    t.set_cursor_pos(2, 1);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.print('A' as u32).unwrap();
    t.screen_mut().end_hyperlink();
    t.index().unwrap();
    t.carriage_return();
    t.print('B' as u32).unwrap();
    t.index().unwrap();
    t.carriage_return();
    t.print('C' as u32).unwrap();

    assert_eq!("\nB\nC", plain(&t));

    for y in 1..3 {
        let (row, cell, id) = hl_id(&t, pt_viewport(0, y));
        assert!(!row);
        assert!(!cell);
        assert_eq!(None, id);
        let p = pin_at(&t, pt_viewport(0, y));
        assert_eq!(0, t.screen().page(p.node).hyperlink_set.count());
    }
}

#[test]
fn terminal_index_bottom_of_scroll_region_with_background_sgr() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(4, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    set_bg_red(&mut t);
    t.index().unwrap();

    assert_eq!("\nA\n\nB", plain(&t));
    for x in 0..t.cols {
        assert_bg_red(&t, pt_active(x, 2));
    }
}

#[test]
fn terminal_index_bottom_of_primary_screen_with_scroll_region() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(5, 1);
    t.clear_dirty();
    t.index().unwrap();
    t.index().unwrap();
    t.index().unwrap();
    t.print('X' as u32).unwrap();

    for y in 0..4 {
        assert!(!t.is_dirty(pt_active(0, y)));
    }
    assert!(t.is_dirty(pt_active(0, 4)));
    assert_eq!("\n\nA\n\nX", plain(&t));
}

#[test]
fn terminal_index_outside_left_right_margin() {
    let mut t = init(10, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.scrolling_region.left = 3;
    t.scrolling_region.right = 5;
    t.set_cursor_pos(3, 3);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.clear_dirty();
    t.index().unwrap();
    t.print('X' as u32).unwrap();

    assert!(t.is_dirty(pt_active(0, 2)));
    assert_eq!("\n\nX A", plain(&t));
}

#[test]
fn terminal_index_inside_left_right_margin() {
    let mut t = init(10, 5);
    print_str(&mut t, "AAAAAA");
    t.carriage_return();
    t.linefeed().unwrap();
    print_str(&mut t, "AAAAAA");
    t.carriage_return();
    t.linefeed().unwrap();
    print_str(&mut t, "AAAAAA");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_top_and_bottom_margin(1, 3);
    t.set_left_and_right_margin(1, 3);
    t.set_cursor_pos(3, 1);

    t.clear_dirty();
    t.index().unwrap();

    assert!(t.is_dirty(pt_active(0, 0)));
    assert!(t.is_dirty(pt_active(0, 1)));
    assert!(t.is_dirty(pt_active(0, 2)));

    assert_eq!(2, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);

    assert_eq!("AAAAAA\nAAAAAA\n   AAA", plain(&t));
}

#[test]
fn terminal_index_bottom_of_scroll_region_creates_scrollback() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    print_str(&mut t, "1\n2\n3");
    t.set_cursor_pos(4, 1);
    t.print('X' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.index().unwrap();
    t.print('Y' as u32).unwrap();

    assert_eq!("2\n3\nY\nX", dump(&t, pt_viewport(0, 0)));
    assert_eq!("1\n2\n3\nY\nX", dump(&t, pt_screen(0, 0)));
}

#[test]
fn terminal_index_bottom_of_scroll_region_no_scrollback() {
    let mut t = init_scrollback(5, 5, 0);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(4, 1);
    t.print('B' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    t.clear_dirty();
    t.index().unwrap();
    t.print('X' as u32).unwrap();
    assert_eq!("\nA\n X\nB", plain(&t));
}

#[test]
fn terminal_index_bottom_of_scroll_region_blank_line_preserves_sgr() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    print_str(&mut t, "1\n2\n3");
    t.set_cursor_pos(4, 1);
    t.print('X' as u32).unwrap();
    t.set_cursor_pos(3, 1);
    set_bg_red(&mut t);
    t.index().unwrap();

    assert_eq!("2\n3\n\nX", dump(&t, pt_viewport(0, 0)));
    assert_eq!("1\n2\n3\n\nX", dump(&t, pt_screen(0, 0)));
    for x in 0..t.cols {
        assert_bg_red(&t, pt_active(x, 2));
    }
}

#[test]
fn terminal_cursor_up_basic() {
    let mut t = init(5, 5);
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    t.cursor_up(10);
    t.print('X' as u32).unwrap();
    assert_eq!(" X\n\nA", plain(&t));
}

#[test]
fn terminal_cursor_up_below_top_scroll_margin() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(2, 4);
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    t.cursor_up(5);
    t.print('X' as u32).unwrap();
    assert_eq!("\n X\nA", plain(&t));
}

#[test]
fn terminal_cursor_up_above_top_scroll_margin() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(3, 5);
    t.set_cursor_pos(3, 1);
    t.print('A' as u32).unwrap();
    t.set_cursor_pos(2, 1);
    t.cursor_up(10);
    t.print('X' as u32).unwrap();
    assert_eq!("X\n\nA", plain(&t));
}

#[test]
fn terminal_cursor_up_resets_wrap() {
    let mut t = init(5, 5);
    chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_up(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print('X' as u32).unwrap();
    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_cursor_left_no_wrap() {
    let mut t = init(10, 5);
    t.print('A' as u32).unwrap();
    t.carriage_return();
    t.linefeed().unwrap();
    t.print('B' as u32).unwrap();
    t.cursor_left(10);
    assert_eq!("A\nB", plain(&t));
}

#[test]
fn terminal_cursor_left_unsets_pending_wrap_state() {
    let mut t = init(5, 5);
    chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_left(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print('X' as u32).unwrap();
    assert_eq!("ABCXE", plain(&t));
}

#[test]
fn terminal_cursor_left_unsets_pending_wrap_state_with_longer_jump() {
    let mut t = init(5, 5);
    chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_left(3);
    assert!(!t.screen().cursor.pending_wrap);
    t.print('X' as u32).unwrap();
    assert_eq!("AXCDE", plain(&t));
}

#[test]
fn terminal_cursor_left_reverse_wrap_with_pending_wrap_state() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_left(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print('X' as u32).unwrap();
    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_cursor_left_reverse_wrap_extended_with_pending_wrap_state() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrapExtended, true);
    chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_left(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print('X' as u32).unwrap();
    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_cursor_left_reverse_wrap() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    chars(&mut t, "ABCDE1");
    t.cursor_left(2);
    t.print('X' as u32).unwrap();
    assert!(t.screen().cursor.pending_wrap);
    assert_eq!("ABCDX\n1", plain(&t));
}

#[test]
fn terminal_cursor_left_reverse_wrap_with_no_soft_wrap() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    chars(&mut t, "ABCDE");
    t.carriage_return();
    t.linefeed().unwrap();
    t.print('1' as u32).unwrap();
    t.cursor_left(2);
    t.print('X' as u32).unwrap();
    assert_eq!("ABCDE\nX", plain(&t));
}

#[test]
fn terminal_cursor_left_reverse_wrap_before_left_margin() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    t.set_top_and_bottom_margin(3, 0);
    t.cursor_left(1);
    t.print('X' as u32).unwrap();
    assert_eq!("\n\nX", plain(&t));
}
