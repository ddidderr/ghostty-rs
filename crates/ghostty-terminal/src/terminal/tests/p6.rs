use super::*;
use crate::page::{ContentTag, Wide};
use ghostty_vt::charsets::Slots;
use ghostty_vt::color::Rgb;
use ghostty_vt::sgr::Attribute;

/// Zig `for ("...") |c| try t.print(c)`.
fn pr(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

fn pt(t: &mut Terminal, c: u32) {
    t.print(c).unwrap();
}

fn node_page(t: &Terminal) -> &crate::page::Page {
    t.screen().cursor_page()
}

#[test]
fn terminal_insert_blanks_shift_off_screen() {
    let mut t = init(5, 10);
    pr(&mut t, "  ABC");
    t.set_cursor_pos(1, 3);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(t.is_dirty(pt_active(0, 0)));
    pt(&mut t, 'X' as u32);
    assert_eq!("  X A", plain(&t));
}

#[test]
fn terminal_insert_blanks_split_multi_cell_character() {
    let mut t = init(5, 10);
    pr(&mut t, "123");
    pt(&mut t, '橋' as u32);
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(1);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!(" 123", plain(&t));
}

#[test]
fn terminal_insert_blanks_inside_left_right_scroll_region() {
    let mut t = init(10, 10);
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    t.set_cursor_pos(1, 3);
    pr(&mut t, "ABC");
    t.set_cursor_pos(1, 3);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(t.is_dirty(pt_active(0, 0)));
    pt(&mut t, 'X' as u32);
    assert_eq!("  X A", plain(&t));
}

#[test]
fn terminal_insert_blanks_outside_left_right_scroll_region() {
    let mut t = init(6, 10);
    t.set_cursor_pos(1, 4);
    pr(&mut t, "ABC");
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    assert!(t.screen().cursor.pending_wrap);
    t.clear_dirty();
    t.insert_blanks(2);
    assert!(!t.is_dirty(pt_active(0, 0)));
    assert!(!t.screen().cursor.pending_wrap);
    pt(&mut t, 'X' as u32);
    assert_eq!("   ABX", plain(&t));
}

#[test]
fn terminal_insert_blanks_left_right_scroll_region_large_count() {
    let mut t = init(10, 10);
    t.modes.set(Mode::Origin, true);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(140);
    assert!(t.is_dirty(pt_active(0, 0)));
    pt(&mut t, 'X' as u32);
    assert_eq!("  X", plain(&t));
}

#[test]
fn terminal_insert_blanks_deleting_graphemes() {
    let mut t = init(5, 5);
    t.modes.set(Mode::GraphemeCluster, true);
    t.print_string("ABC").unwrap();

    for cp in [0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467] {
        pt(&mut t, cp);
    }

    assert_eq!(1, node_page(&t).grapheme_count());

    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(4);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("    A", plain(&t));
    assert_eq!(0, node_page(&t).grapheme_count());
}

#[test]
fn terminal_insert_blanks_shift_graphemes() {
    let mut t = init(5, 5);
    t.modes.set(Mode::GraphemeCluster, true);
    t.print_string("A").unwrap();

    for cp in [0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467] {
        pt(&mut t, cp);
    }

    assert_eq!(1, node_page(&t).grapheme_count());

    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.insert_blanks(1);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!(" A👨‍👩‍👧", plain(&t));
    assert_eq!(1, node_page(&t).grapheme_count());
}

#[test]
fn terminal_insert_blanks_split_multi_cell_character_from_tail() {
    let mut t = init(5, 10);
    t.print_string("橋123").unwrap();
    t.set_cursor_pos(1, 2);
    t.insert_blanks(1);
    assert_eq!("   12", plain(&t));
}

#[test]
fn terminal_insert_blanks_shifts_hyperlinks() {
    let mut t = init(10, 2);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.print_string("ABC").unwrap();
    t.set_cursor_pos(1, 1);
    t.insert_blanks(2);

    assert_eq!("  ABC", plain(&t));

    // Verify all our cells have a hyperlink
    for x in 2..5u16 {
        let p = pin_at(&t, pt_screen(x, 0));
        let page = t.screen().page(p.node);
        assert!(page.rows[usize::from(p.y)].hyperlink());
        let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
        assert!(page.cells[i].hyperlink());
        assert_eq!(Some(1), page.lookup_hyperlink(i));
    }
    for x in 0..2u16 {
        let p = pin_at(&t, pt_screen(x, 0));
        let page = t.screen().page(p.node);
        let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
        assert!(!page.cells[i].hyperlink());
        assert_eq!(None, page.lookup_hyperlink(i));
    }
}

#[test]
fn terminal_insert_blanks_pushes_hyperlink_off_end_completely() {
    let mut t = init(3, 2);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.print_string("ABC").unwrap();
    t.set_cursor_pos(1, 1);
    t.insert_blanks(3);

    assert_eq!("", plain(&t));

    for x in 0..3u16 {
        let p = pin_at(&t, pt_screen(x, 0));
        let page = t.screen().page(p.node);
        assert!(!page.rows[usize::from(p.y)].hyperlink());
        let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
        assert!(!page.cells[i].hyperlink());
        assert_eq!(None, page.lookup_hyperlink(i));
    }
}

#[test]
fn terminal_insert_blanks_wide_char_straddling_right_margin() {
    let mut t = init(10, 5);
    t.set_cursor_pos(1, 1);
    pr(&mut t, "ABCD");
    pt(&mut t, '橋' as u32);
    t.scrolling_region.right = 4;
    t.set_cursor_pos(1, 3);
    t.insert_blanks(1);
    assert_eq!("AB CD", plain(&t));
}

#[test]
fn terminal_insert_blanks_wide_char_spacer_tail_orphaned_beyond_right_margin() {
    let mut t = init(10, 5);
    for _ in 0..5 {
        pt(&mut t, 0x4E2D);
    }
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(1, 9);
    pt(&mut t, 'a' as u32);
    t.insert_blanks(8);
    assert_eq!("a", plain(&t));
}

#[test]
fn terminal_insert_mode_with_space() {
    let mut t = init(10, 2);
    pr(&mut t, "hello");
    t.set_cursor_pos(1, 2);
    t.modes.set(Mode::Insert, true);
    pt(&mut t, 'X' as u32);
    assert_eq!("hXello", plain(&t));
}

#[test]
fn terminal_insert_mode_doesnt_wrap_pushed_characters() {
    let mut t = init(5, 2);
    pr(&mut t, "hello");
    t.set_cursor_pos(1, 2);
    t.modes.set(Mode::Insert, true);
    pt(&mut t, 'X' as u32);
    assert_eq!("hXell", plain(&t));
}

#[test]
fn terminal_insert_mode_does_nothing_at_the_end_of_the_line() {
    let mut t = init(5, 2);
    pr(&mut t, "hello");
    t.modes.set(Mode::Insert, true);
    pt(&mut t, 'X' as u32);
    assert_eq!("hello\nX", plain(&t));
}

#[test]
fn terminal_insert_mode_with_wide_characters() {
    let mut t = init(5, 2);
    pr(&mut t, "hello");
    t.set_cursor_pos(1, 2);
    t.modes.set(Mode::Insert, true);
    pt(&mut t, 0x1F600);
    assert_eq!("h😀el", plain(&t));
}

#[test]
fn terminal_insert_mode_with_wide_characters_at_end() {
    let mut t = init(5, 2);
    pr(&mut t, "well");
    t.modes.set(Mode::Insert, true);
    pt(&mut t, 0x1F600);
    assert_eq!("well\n😀", plain(&t));
}

#[test]
fn terminal_insert_mode_pushing_off_wide_character() {
    let mut t = init(5, 2);
    pr(&mut t, "123");
    pt(&mut t, 0x1F600);
    t.modes.set(Mode::Insert, true);
    t.set_cursor_pos(1, 1);
    pt(&mut t, 'X' as u32);
    assert_eq!("X123", plain(&t));
}

#[test]
fn terminal_delete_chars() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_chars(2);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("ADE", plain(&t));
}

#[test]
fn terminal_delete_chars_zero_count() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_chars(0);
    assert!(!t.is_dirty(pt_active(0, 0)));
    assert_eq!("ABCDE", plain(&t));
}

#[test]
fn terminal_delete_chars_more_than_half() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_chars(3);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("AE", plain(&t));
}

#[test]
fn terminal_delete_chars_more_than_line_width() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_chars(10);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("A", plain(&t));
}

#[test]
fn terminal_delete_chars_should_shift_left() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 2);
    t.clear_dirty();
    t.delete_chars(1);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("ACDE", plain(&t));
}

#[test]
fn terminal_delete_chars_resets_pending_wrap() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.delete_chars(1);
    assert!(!t.screen().cursor.pending_wrap);
    pt(&mut t, 'X' as u32);
    assert_eq!("ABCDX", plain(&t));
}

#[test]
fn terminal_delete_chars_resets_wrap() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE123");
    assert!(row_at(&t, pt_active(0, 0)).wrap());
    t.set_cursor_pos(1, 1);
    t.delete_chars(1);
    assert!(!row_at(&t, pt_active(0, 0)).wrap());
    pt(&mut t, 'X' as u32);
    assert_eq!("XCDE\n123", plain(&t));
}

#[test]
fn terminal_delete_chars_simple_operation() {
    let mut t = init(10, 10);
    t.print_string("ABC123").unwrap();
    t.set_cursor_pos(1, 3);
    t.clear_dirty();
    t.delete_chars(2);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("AB23", plain(&t));
}

#[test]
fn terminal_delete_chars_preserves_background_sgr() {
    let mut t = init(10, 10);
    pr(&mut t, "ABC123");
    t.set_cursor_pos(1, 3);
    t.set_attribute(Attribute::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0,
    }))
    .unwrap();
    t.delete_chars(2);

    assert_eq!("AB23", plain(&t));
    for x in t.cols - 2..t.cols {
        let c = cell_at(&t, pt_active(x, 0));
        assert!(c.content_tag() == ContentTag::BgColorRgb);
        assert_eq!(
            Rgb {
                r: 0xFF,
                g: 0,
                b: 0
            },
            c.color_rgb()
        );
    }
}

#[test]
fn terminal_delete_chars_outside_scroll_region() {
    let mut t = init(6, 10);
    t.print_string("ABC123").unwrap();
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    assert!(t.screen().cursor.pending_wrap);
    t.clear_dirty();
    t.delete_chars(2);
    assert!(!t.is_dirty(pt_active(0, 0)));
    assert!(t.screen().cursor.pending_wrap);
    assert_eq!("ABC123", plain(&t));
}

#[test]
fn terminal_delete_chars_inside_scroll_region() {
    let mut t = init(6, 10);
    t.print_string("ABC123").unwrap();
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    t.set_cursor_pos(1, 4);
    t.clear_dirty();
    t.delete_chars(1);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("ABC2 3", plain(&t));
}

#[test]
fn terminal_delete_chars_split_wide_character_from_spacer_tail() {
    let mut t = init(6, 10);
    t.print_string("A橋123").unwrap();
    t.set_cursor_pos(1, 3);
    t.delete_chars(1);
    assert_eq!("A 123", plain(&t));
}

#[test]
fn terminal_delete_chars_split_wide_character_from_wide() {
    let mut t = init(6, 10);
    t.print_string("橋123").unwrap();
    t.set_cursor_pos(1, 1);
    t.delete_chars(1);

    let c = cell_at(&t, pt_screen(0, 0));
    assert_eq!(0, c.codepoint());
    assert_eq!(Wide::Narrow, c.wide());
    let c = cell_at(&t, pt_screen(1, 0));
    assert_eq!('1' as u32, c.codepoint());
    assert_eq!(Wide::Narrow, c.wide());
}

#[test]
fn terminal_delete_chars_split_wide_character_from_end() {
    let mut t = init(6, 10);
    t.print_string("A橋123").unwrap();
    t.set_cursor_pos(1, 1);
    t.delete_chars(1);

    let c = cell_at(&t, pt_screen(0, 0));
    assert_eq!(0x6A4B, c.codepoint());
    assert_eq!(Wide::Wide, c.wide());
    let c = cell_at(&t, pt_screen(1, 0));
    assert_eq!(0, c.codepoint());
    assert_eq!(Wide::SpacerTail, c.wide());
}

#[test]
fn terminal_delete_chars_with_a_spacer_head_at_the_end() {
    let mut t = init(5, 10);
    t.print_string("0123橋123").unwrap();
    {
        let c = cell_at(&t, pt_screen(4, 0));
        assert_eq!(Wide::SpacerHead, c.wide());
        assert!(row_at(&t, pt_screen(4, 0)).wrap());
    }

    t.set_cursor_pos(1, 1);
    t.delete_chars(1);

    let c = cell_at(&t, pt_screen(3, 0));
    assert_eq!(0, c.codepoint());
    assert_eq!(Wide::Narrow, c.wide());
}

#[test]
fn terminal_delete_chars_split_wide_character_tail() {
    let mut t = init(5, 5);
    let cols = usize::from(t.cols);
    t.set_cursor_pos(1, cols - 1);
    pt(&mut t, 0x6A4B);
    t.carriage_return();
    t.delete_chars(cols - 1);
    pt(&mut t, '0' as u32);
    assert_eq!("0", plain(&t));
}

#[test]
fn terminal_delete_chars_wide_char_boundary_conditions() {
    let mut t = init(8, 1);
    t.print_string("😀a😀b😀").unwrap();
    assert_eq!("😀a😀b😀", plain(&t));

    t.set_cursor_pos(1, 2);
    t.delete_chars(3);
    node_page(&t).assert_integrity();

    assert_eq!("  b😀", plain(&t));
}

#[test]
fn terminal_delete_chars_wide_char_wrap_boundary_conditions() {
    let mut t = init(8, 3);
    t.print_string(".......😀abcde😀......").unwrap();
    assert_eq!(".......\n😀abcde\n😀......", plain(&t));
    assert_eq!(".......😀abcde😀......", plain_unwrapped(&t));

    t.set_cursor_pos(2, 2);
    t.delete_chars(3);
    node_page(&t).assert_integrity();

    assert_eq!(".......\n cde\n😀......", plain(&t));
    assert_eq!(".......  cde\n😀......", plain_unwrapped(&t));
}

#[test]
fn terminal_delete_chars_wide_char_across_right_margin() {
    let mut t = init(8, 3);
    t.print_string("123456橋").unwrap();
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(2, 7);
    assert_eq!("123456橋", plain(&t));

    t.set_cursor_pos(1, 2);
    t.delete_chars(1);
    node_page(&t).assert_integrity();

    assert_eq!("13456", plain(&t));
}

#[test]
fn terminal_save_cursor() {
    let mut t = init(3, 3);
    t.set_attribute(Attribute::Bold).unwrap();
    t.screen_mut().charset.gr = Slots::G3;
    t.modes.set(Mode::Origin, true);
    t.save_cursor();
    t.screen_mut().charset.gr = Slots::G0;
    t.set_attribute(Attribute::Unset).unwrap();
    t.modes.set(Mode::Origin, false);
    t.restore_cursor();
    assert!(t.screen().cursor.style.flags.bold);
    assert!(t.screen().charset.gr == Slots::G3);
    assert!(t.modes.get(Mode::Origin));
}

#[test]
fn terminal_save_cursor_position() {
    let mut t = init(10, 5);
    t.set_cursor_pos(1, 5);
    pt(&mut t, 'A' as u32);
    t.save_cursor();
    t.set_cursor_pos(1, 1);
    pt(&mut t, 'B' as u32);
    t.restore_cursor();
    pt(&mut t, 'X' as u32);
    assert_eq!("B   AX", plain(&t));
}

#[test]
fn terminal_save_cursor_pending_wrap_state() {
    let mut t = init(5, 5);
    t.set_cursor_pos(1, 5);
    pt(&mut t, 'A' as u32);
    t.save_cursor();
    t.set_cursor_pos(1, 1);
    pt(&mut t, 'B' as u32);
    t.restore_cursor();
    pt(&mut t, 'X' as u32);
    assert_eq!("B   A\nX", plain(&t));
}

#[test]
fn terminal_save_cursor_origin_mode() {
    let mut t = init(10, 5);
    t.modes.set(Mode::Origin, true);
    t.save_cursor();
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_top_and_bottom_margin(2, 4);
    t.restore_cursor();
    pt(&mut t, 'X' as u32);
    assert_eq!("X", plain(&t));
}

#[test]
fn terminal_save_cursor_resize() {
    let mut t = init(10, 5);
    t.set_cursor_pos(1, 10);
    t.save_cursor();
    t.resize(5, 5).unwrap();
    t.restore_cursor();
    pt(&mut t, 'X' as u32);
    assert_eq!("    X", plain(&t));
}

#[test]
fn terminal_save_cursor_protected_pen() {
    let mut t = init(10, 5);
    t.set_protected_mode(ProtectedMode::Iso);
    assert!(t.screen().cursor.protected);
    t.set_cursor_pos(1, 10);
    t.save_cursor();
    t.set_protected_mode(ProtectedMode::Off);
    assert!(!t.screen().cursor.protected);
    t.restore_cursor();
    assert!(t.screen().cursor.protected);
}

#[test]
fn terminal_save_cursor_doesnt_modify_hyperlink_state() {
    let mut t = init(3, 3);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    let id = t.screen().cursor.hyperlink_id;
    t.save_cursor();
    assert_eq!(id, t.screen().cursor.hyperlink_id);
    t.restore_cursor();
    assert_eq!(id, t.screen().cursor.hyperlink_id);
}

#[test]
fn terminal_restore_cursor_uses_default_style_on_out_of_space() {
    // Use a single row so the page can't be split
    let mut t = init(10, 1);

    // Set a style and save the cursor
    t.set_attribute(Attribute::Bold).unwrap();
    t.save_cursor();

    // Clear the style
    t.set_attribute(Attribute::Unset).unwrap();
    assert!(!t.screen().cursor.style.flags.bold);

    // Fill the style map to max capacity
    let max_styles = size::CellCountInt::MAX;
    while t.screen().cursor_page().capacity.styles < max_styles {
        let node = t.screen().cursor_pin().node;
        if t.screen_mut()
            .increase_capacity(node, Some(IncreaseCapacity::Styles))
            .is_err()
        {
            break;
        }
    }

    assert_eq!(max_styles, t.screen().cursor_page().capacity.styles);

    // Fill all style slots using the StyleSet's capacity.
    {
        let page = t.screen_mut().cursor_page_mut();
        page.pause_integrity_checks(true);
        // Zig `layout.cap` counts the reserved zero slot.
        let max_items = page.styles.capacity() + 1;
        let mut n: usize = 1;
        while n < max_items {
            let bg = Rgb {
                r: ((n >> 16) & 0xFF) as u8,
                g: ((n >> 8) & 0xFF) as u8,
                b: (n & 0xFF) as u8,
            };
            let st = Style {
                bg_color: StyleColor::Rgb(bg),
                ..Style::default()
            };
            if page.styles.add(st).is_err() {
                break;
            }
            n += 1;
        }
        page.pause_integrity_checks(false);
        page.assert_integrity();
    }

    // Restore cursor - should fall back to default style since page
    // can't be split (1 row) and styles are at max capacity
    t.restore_cursor();

    // The style should be reset to default because OutOfSpace occurred
    assert!(!t.screen().cursor.style.flags.bold);
    assert_eq!(style::DEFAULT_ID, t.screen().cursor.style_id);
}

#[test]
fn terminal_set_protected_mode() {
    let mut t = init(3, 3);
    assert!(!t.screen().cursor.protected);
    t.set_protected_mode(ProtectedMode::Off);
    assert!(!t.screen().cursor.protected);
    t.set_protected_mode(ProtectedMode::Iso);
    assert!(t.screen().cursor.protected);
    t.set_protected_mode(ProtectedMode::Dec);
    assert!(t.screen().cursor.protected);
    t.set_protected_mode(ProtectedMode::Off);
    assert!(!t.screen().cursor.protected);
}

#[test]
fn terminal_erase_line_simple_erase_right() {
    let mut t = init(5, 5);
    pr(&mut t, "ABCDE");
    t.set_cursor_pos(1, 3);
    t.clear_dirty();
    t.erase_line(csi::EraseLine::Right, false);
    assert!(t.is_dirty(pt_active(0, 0)));
    assert_eq!("AB", plain(&t));
}
