use ghostty_vt::{color::Rgb, sgr::Attribute};

use super::*;
use crate::page::ContentTag;

fn prints(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

fn nl(t: &mut Terminal) {
    t.carriage_return();
    t.linefeed().unwrap();
}

fn dirty(t: &Terminal, x: u16, y: u32) -> bool {
    t.is_dirty(pt_active(x, y))
}

fn abc_def_ghi(t: &mut Terminal) {
    print_str(t, "ABC");
    nl(t);
    print_str(t, "DEF");
    nl(t);
    print_str(t, "GHI");
}

fn abcd_lines(t: &mut Terminal) {
    for (i, c) in ['A', 'B', 'C', 'D'].into_iter().enumerate() {
        if i > 0 {
            nl(t);
        }
        t.print(u32::from(c)).unwrap();
    }
}

const RED: Rgb = Rgb {
    r: 0xFF,
    g: 0,
    b: 0,
};

#[test]
fn terminal_cursor_left_extended_reverse_wrap() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrapExtended, true);
    prints(&mut t, "ABCDE");
    nl(&mut t);
    t.print(u32::from('1')).unwrap();
    t.cursor_left(2);
    t.print(u32::from('X')).unwrap();
    assert_eq!("ABCDX\n1", plain(&t));
}

#[test]
fn terminal_cursor_left_extended_reverse_wrap_bottom_wraparound() {
    let mut t = init(5, 3);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrapExtended, true);
    prints(&mut t, "ABCDE");
    nl(&mut t);
    t.print(u32::from('1')).unwrap();
    t.cursor_left(1 + usize::from(t.cols) + 1);
    t.print(u32::from('X')).unwrap();
    assert_eq!("ABCDE\n1\n    X", plain(&t));
}

#[test]
fn terminal_cursor_left_extended_reverse_wrap_is_priority_if_both_set() {
    let mut t = init(5, 3);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    t.modes.set(Mode::ReverseWrapExtended, true);
    prints(&mut t, "ABCDE");
    nl(&mut t);
    t.print(u32::from('1')).unwrap();
    t.cursor_left(1 + usize::from(t.cols) + 1);
    t.print(u32::from('X')).unwrap();
    assert_eq!("ABCDE\n1\n    X", plain(&t));
}

#[test]
fn terminal_cursor_left_extended_reverse_wrap_above_top_scroll_region() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrapExtended, true);
    t.set_top_and_bottom_margin(3, 0);
    t.set_cursor_pos(2, 1);
    t.cursor_left(1000);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor.y);
}

#[test]
fn terminal_cursor_left_reverse_wrap_on_first_row() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, true);
    t.modes.set(Mode::ReverseWrap, true);
    t.set_top_and_bottom_margin(3, 0);
    t.set_cursor_pos(1, 2);
    t.cursor_left(1000);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor.y);
}

#[test]
fn terminal_cursor_down_basic() {
    let mut t = init(5, 5);
    t.print(u32::from('A')).unwrap();
    t.cursor_down(10);
    t.print(u32::from('X')).unwrap();
    assert_eq!("A\n\n\n\n X", plain(&t));
}

#[test]
fn terminal_cursor_down_above_bottom_scroll_margin() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.print(u32::from('A')).unwrap();
    t.cursor_down(10);
    t.print(u32::from('X')).unwrap();
    assert_eq!("A\n\n X", plain(&t));
}

#[test]
fn terminal_cursor_down_below_bottom_scroll_margin() {
    let mut t = init(5, 5);
    t.set_top_and_bottom_margin(1, 3);
    t.print(u32::from('A')).unwrap();
    t.set_cursor_pos(4, 1);
    t.cursor_down(10);
    t.print(u32::from('X')).unwrap();
    assert_eq!("A\n\n\n\nX", plain(&t));
}

#[test]
fn terminal_cursor_down_resets_wrap() {
    let mut t = init(5, 5);
    prints(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_down(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('X')).unwrap();
    assert_eq!("ABCDE\n    X", plain(&t));
}

#[test]
fn terminal_cursor_right_resets_wrap() {
    let mut t = init(5, 5);
    prints(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.cursor_right(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('X')).unwrap();
    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_cursor_right_to_the_edge_of_screen() {
    let mut t = init(5, 5);
    t.cursor_right(100);
    t.print(u32::from('X')).unwrap();
    assert_eq!("    X", plain(&t));
}

#[test]
fn terminal_cursor_right_left_of_right_margin() {
    let mut t = init(5, 5);
    t.scrolling_region.right = 2;
    t.cursor_right(100);
    t.print(u32::from('X')).unwrap();
    assert_eq!("  X", plain(&t));
}

#[test]
fn terminal_cursor_right_right_of_right_margin() {
    let mut t = init(5, 5);
    t.scrolling_region.right = 2;
    t.set_cursor_pos(1, 4);
    t.cursor_right(100);
    t.print(u32::from('X')).unwrap();
    assert_eq!("    X", plain(&t));
}

#[test]
fn terminal_delete_lines_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);
    t.clear_dirty();
    t.delete_lines(1);
    assert!(!dirty(&t, 0, 0));
    assert!(dirty(&t, 0, 1));
    assert!(dirty(&t, 0, 2));
    assert!(dirty(&t, 0, 3));
    assert_eq!("ABC\nGHI", plain(&t));
}

#[test]
fn terminal_delete_lines_colors_with_bg_color() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_cursor_pos(2, 2);
    t.set_attribute(Attribute::DirectColorBg(RED)).unwrap();
    t.delete_lines(1);
    assert_eq!("ABC\nGHI", plain(&t));
    for x in 0..t.cols {
        let c = cell_at(&t, pt_active(x, 4));
        assert_eq!(ContentTag::BgColorRgb, c.content_tag());
        assert_eq!(RED, c.color_rgb());
    }
}

#[test]
fn terminal_delete_lines_across_page_boundary_marks_all_shifted_rows_dirty() {
    let mut t = init_scrollback(10, 5, 1024);
    let first = t.screen().pages.first.unwrap();
    let first_page_nrows = t.screen().page(first).capacity.rows;

    // Fill up the first page minus 3 rows
    for _ in 0..first_page_nrows - 3 {
        t.linefeed().unwrap();
    }

    // Add content that will cross a page boundary
    print_str(&mut t, "1AAAA");
    nl(&mut t);
    print_str(&mut t, "2BBBB");
    nl(&mut t);
    print_str(&mut t, "3CCCC");
    nl(&mut t);
    print_str(&mut t, "4DDDD");
    nl(&mut t);
    print_str(&mut t, "5EEEE");

    // Verify we now have a second page
    assert!(t.screen().pages.node(first).unwrap().next.is_some());

    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.delete_lines(1);

    for y in 0..5 {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("2BBBB\n3CCCC\n4DDDD\n5EEEE", plain(&t));
}

#[test]
fn terminal_delete_lines_legacy() {
    let mut t = init(80, 80);
    abcd_lines(&mut t);
    t.cursor_up(2);
    t.delete_lines(1);
    t.print(u32::from('E')).unwrap();
    nl(&mut t);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(2, t.screen().cursor.y);
    assert_eq!("A\nE\nD", plain(&t));
}

#[test]
fn terminal_delete_lines_with_scroll_region() {
    let mut t = init(80, 80);
    abcd_lines(&mut t);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.delete_lines(1);
    assert!(dirty(&t, 0, 0));
    assert!(dirty(&t, 0, 1));
    assert!(dirty(&t, 0, 2));
    assert!(!dirty(&t, 0, 3));
    t.print(u32::from('E')).unwrap();
    nl(&mut t);
    assert_eq!("E\nC\n\nD", plain(&t));
}

#[test]
fn terminal_delete_lines_with_scroll_region_large_count() {
    let mut t = init(80, 80);
    abcd_lines(&mut t);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.delete_lines(5);
    assert!(dirty(&t, 0, 0));
    assert!(dirty(&t, 0, 1));
    assert!(dirty(&t, 0, 2));
    assert!(!dirty(&t, 0, 3));
    t.print(u32::from('E')).unwrap();
    nl(&mut t);
    assert_eq!("E\n\n\nD", plain(&t));
}

#[test]
fn terminal_delete_lines_with_scroll_region_cursor_outside_of_region() {
    let mut t = init(80, 80);
    abcd_lines(&mut t);
    t.set_top_and_bottom_margin(1, 3);
    t.set_cursor_pos(4, 1);
    t.clear_dirty();
    t.delete_lines(1);
    for y in 0..4 {
        assert!(!dirty(&t, 0, y));
    }
    assert_eq!("A\nB\nC\nD", plain(&t));
}

#[test]
fn terminal_delete_lines_resets_pending_wrap() {
    let mut t = init(5, 5);
    prints(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.delete_lines(1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('B')).unwrap();
    assert_eq!("B", plain(&t));
}

#[test]
fn terminal_delete_lines_resets_wrap() {
    let mut t = init(3, 3);
    t.print(u32::from('1')).unwrap();
    nl(&mut t);
    prints(&mut t, "ABCDEF");
    t.set_top_and_bottom_margin(1, 2);
    t.set_cursor_pos(1, 1);
    t.delete_lines(1);
    t.print(u32::from('X')).unwrap();
    assert_eq!("XBC\n\nDEF", plain(&t));
    for y in 0..u32::from(t.rows) {
        assert!(!row_at(&t, pt_active(0, y)).wrap());
    }
}

fn lr_setup() -> Terminal {
    let mut t = init(10, 10);
    abc123(&mut t);
    t.scrolling_region.left = 1;
    t.scrolling_region.right = 3;
    t
}

fn abc123(t: &mut Terminal) {
    print_str(t, "ABC123");
    nl(t);
    print_str(t, "DEF456");
    nl(t);
    print_str(t, "GHI789");
}

#[test]
fn terminal_delete_lines_left_right_scroll_region() {
    let mut t = lr_setup();
    t.set_cursor_pos(2, 2);
    t.clear_dirty();
    t.delete_lines(1);
    assert!(!dirty(&t, 0, 0));
    for y in 1..3 {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("ABC123\nDHI756\nG   89", plain(&t));
}

#[test]
fn terminal_delete_lines_left_right_scroll_region_from_top() {
    let mut t = lr_setup();
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_lines(1);
    for y in 0..3 {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("AEF423\nDHI756\nG   89", plain(&t));
}

#[test]
fn terminal_delete_lines_left_right_scroll_region_high_count() {
    let mut t = lr_setup();
    t.set_cursor_pos(2, 2);
    t.clear_dirty();
    t.delete_lines(100);
    assert!(!dirty(&t, 0, 0));
    for y in 1..3 {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("ABC123\nD   56\nG   89", plain(&t));
}

#[test]
fn terminal_delete_lines_wide_character_spacer_head() {
    let mut t = init(5, 3);
    print_str(&mut t, "AAAAABBBB\u{1F600}CCC");
    t.set_cursor_pos(1, 1);
    t.delete_lines(1);
    assert_eq!("BBBB\n\u{1F600}CCC", plain(&t));
    assert_eq!("BBBB\n\u{1F600}CCC", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_lines_wide_character_spacer_head_left_scroll_margin() {
    let mut t = init(5, 3);
    print_str(&mut t, "AAAAABBBB\u{1F600}CCC");
    t.scrolling_region.left = 2;
    t.set_cursor_pos(1, 3);
    t.delete_lines(1);
    assert_eq!("AABB\nBBCCC\n\u{1F600}", plain(&t));
    assert_eq!("AABB BBCCC\u{1F600}", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_lines_wide_character_spacer_head_right_scroll_margin() {
    let mut t = init(5, 3);
    print_str(&mut t, "AAAAABBBB\u{1F600}CCC");
    t.scrolling_region.right = 3;
    t.set_cursor_pos(1, 1);
    t.delete_lines(1);
    assert_eq!("BBBBA\n\u{1F600}CC\n    C", plain(&t));
    assert_eq!("BBBBA\u{1F600}CC     C", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_lines_wide_character_spacer_head_left_and_right_scroll_margin() {
    let mut t = init(5, 3);
    print_str(&mut t, "AAAAABBBB\u{1F600}CCC");
    t.scrolling_region.right = 3;
    t.scrolling_region.left = 2;
    t.set_cursor_pos(1, 3);
    t.delete_lines(1);
    assert_eq!("AABBA\nBBCC\n\u{1F600}  C", plain(&t));
    assert_eq!("AABBABBCC\u{1F600}  C", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_lines_wide_character_spacer_head_left_lt_2_and_right_scroll_margin() {
    let mut t = init(5, 3);
    print_str(&mut t, "AAAAABBBB\u{1F600}CCC");
    t.scrolling_region.right = 3;
    t.scrolling_region.left = 1;
    t.set_cursor_pos(1, 2);
    t.delete_lines(1);
    assert_eq!("ABBBA\nB CC\n    C", plain(&t));
    assert_eq!("ABBBAB CC     C", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_lines_wide_characters_split_by_left_right_scroll_region_boundaries() {
    let mut t = init(5, 2);
    print_str(&mut t, "AAAAA\n\u{1F600}B\u{1F600}");
    t.scrolling_region.right = 3;
    t.scrolling_region.left = 1;
    t.set_cursor_pos(1, 2);
    t.delete_lines(1);
    assert_eq!("A B A", plain(&t));
}

#[test]
fn terminal_delete_lines_zero() {
    let mut t = init(2, 5);
    t.set_cursor_pos(1, 1);
    t.delete_lines(0);
}

#[test]
fn terminal_default_style_is_empty() {
    let mut t = init(5, 5);
    t.print(u32::from('A')).unwrap();
    let c = cell_at(&t, pt_screen(0, 0));
    assert_eq!(u32::from('A'), c.codepoint());
    assert_eq!(0, c.style_id());
}

#[test]
fn terminal_bold_style() {
    let mut t = init(5, 5);
    t.set_attribute(Attribute::Bold).unwrap();
    t.print(u32::from('A')).unwrap();
    let c = cell_at(&t, pt_screen(0, 0));
    assert_eq!(u32::from('A'), c.codepoint());
    assert!(c.style_id() != 0);
    let id = t.screen().cursor.style_id;
    assert!(t.screen().cursor_page().styles.ref_count(id) > 1);
}

#[test]
fn terminal_garbage_collect_overwritten() {
    let mut t = init(5, 5);
    t.set_attribute(Attribute::Bold).unwrap();
    t.print(u32::from('A')).unwrap();
    t.set_cursor_pos(1, 1);
    t.set_attribute(Attribute::Unset).unwrap();
    t.print(u32::from('B')).unwrap();
    let c = cell_at(&t, pt_screen(0, 0));
    assert_eq!(u32::from('B'), c.codepoint());
    assert_eq!(0, c.style_id());
    // verify we have no styles in our style map
    assert_eq!(0, t.screen().cursor_page().styles.count());
}

#[test]
fn terminal_do_not_garbage_collect_old_styles_in_use() {
    let mut t = init(5, 5);
    t.set_attribute(Attribute::Bold).unwrap();
    t.print(u32::from('A')).unwrap();
    t.set_attribute(Attribute::Unset).unwrap();
    t.print(u32::from('B')).unwrap();
    let c = cell_at(&t, pt_screen(1, 0));
    assert_eq!(u32::from('B'), c.codepoint());
    assert_eq!(0, c.style_id());
    assert_eq!(1, t.screen().cursor_page().styles.count());
}

#[test]
fn terminal_print_with_style_marks_the_row_as_styled() {
    let mut t = init(5, 5);
    t.set_attribute(Attribute::Bold).unwrap();
    t.print(u32::from('A')).unwrap();
    t.set_attribute(Attribute::Unset).unwrap();
    t.print(u32::from('B')).unwrap();
    assert!(row_at(&t, pt_screen(0, 0)).styled());
}

#[test]
fn terminal_decaln() {
    let mut t = init(2, 2);
    t.print(u32::from('A')).unwrap();
    nl(&mut t);
    t.print(u32::from('B')).unwrap();
    t.decaln().unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);
    for y in 0..u32::from(t.rows) {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("EE\nEE", plain(&t));
}

#[test]
fn terminal_decaln_reset_margins() {
    let mut t = init(3, 3);
    t.modes.set(Mode::Origin, true);
    t.set_top_and_bottom_margin(2, 3);
    t.decaln().unwrap();
    t.scroll_down(1);
    assert_eq!("\nEEE\nEEE", plain(&t));
}

#[test]
fn terminal_decaln_preserves_color() {
    let mut t = init(3, 3);
    t.set_attribute(Attribute::DirectColorBg(RED)).unwrap();
    t.modes.set(Mode::Origin, true);
    t.set_top_and_bottom_margin(2, 3);
    t.decaln().unwrap();
    t.scroll_down(1);
    assert_eq!("\nEEE\nEEE", plain(&t));
    let c = cell_at(&t, pt_active(0, 0));
    assert_eq!(ContentTag::BgColorRgb, c.content_tag());
    assert_eq!(RED, c.color_rgb());
}

#[test]
fn terminal_decaln_resets_graphemes_with_protected_mode() {
    let mut t = init(3, 3);
    t.set_protected_mode(ProtectedMode::Iso);
    t.modes.set(Mode::GraphemeCluster, true);
    for cp in [0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467] {
        t.print(cp).unwrap();
    }
    t.decaln().unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);
    assert!(t.screen().cursor.protected);
    assert_eq!(ProtectedMode::Iso, t.screen().protected_mode);
    for y in 0..u32::from(t.rows) {
        assert!(dirty(&t, 0, y));
    }
    assert_eq!("EEE\nEEE\nEEE", plain(&t));
}

#[test]
fn terminal_insert_blanks_zero() {
    let mut t = init(5, 2);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.insert_blanks(0);
    assert_eq!("ABC", plain(&t));
}

#[test]
fn terminal_insert_blanks() {
    let mut t = init(5, 2);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(dirty(&t, 0, 0));
    assert!(!dirty(&t, 0, 1));
    assert_eq!("  ABC", plain(&t));
}

#[test]
fn terminal_insert_blanks_pushes_off_end() {
    let mut t = init(3, 2);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(dirty(&t, 0, 0));
    assert_eq!("  A", plain(&t));
}

#[test]
fn terminal_insert_blanks_more_than_size() {
    let mut t = init(3, 2);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(5);
    assert!(dirty(&t, 0, 0));
    assert_eq!("", plain(&t));
}

#[test]
fn terminal_insert_blanks_no_scroll_region_fits() {
    let mut t = init(10, 10);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(dirty(&t, 0, 0));
    assert_eq!("  ABC", plain(&t));
}

#[test]
fn terminal_insert_blanks_preserves_background_sgr() {
    let mut t = init(10, 10);
    prints(&mut t, "ABC");
    t.set_cursor_pos(1, 1);
    t.set_attribute(Attribute::DirectColorBg(RED)).unwrap();
    t.insert_blanks(2);
    assert_eq!("  ABC", plain(&t));
    let c = cell_at(&t, pt_active(0, 0));
    assert_eq!(ContentTag::BgColorRgb, c.content_tag());
    assert_eq!(RED, c.color_rgb());
}
