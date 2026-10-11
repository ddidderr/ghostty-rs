use super::*;
use crate::page::{SemanticPrompt as RowPrompt, Wide};

fn hl_id(t: &Terminal, pt: Point) -> Option<u16> {
    let p = pin_at(t, pt);
    let page = t.screen().page(p.node);
    let i = page.cell_index(usize::from(p.x), usize::from(p.y)).unwrap();
    page.lookup_hyperlink(i)
}

fn print_chars(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

#[test]
fn terminal_overwrite_multicodepoint_grapheme_tail_clears_grapheme_data() {
    let mut t = init(10, 10);
    t.modes.set(Mode::GraphemeCluster, true);

    for cp in [0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467] {
        t.print(cp).unwrap();
    }

    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert_eq!(1, t.screen().cursor_page().grapheme_count());

    t.set_cursor_pos(1, 2);
    t.print(u32::from('X')).unwrap();
    assert_eq!(" X", plain(&t));
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor_page().grapheme_count());
}

#[test]
fn terminal_print_breaks_valid_grapheme_cluster_with_prepend_ascii_for_speed() {
    let mut t = init(5, 5);
    t.modes.set(Mode::GraphemeCluster, true);
    t.print(u32::from('_')).unwrap();
    t.print(0x0600).unwrap();
    t.print(u32::from('1')).unwrap();

    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(3, t.screen().cursor.x);
    {
        let cell = cell_at(&t, pt_screen(1, 0));
        assert_eq!(0x0600, cell.codepoint());
        assert!(!cell.has_grapheme());
        assert_eq!(Wide::Narrow, cell.wide());
    }
    {
        let cell = cell_at(&t, pt_screen(2, 0));
        assert_eq!(u32::from('1'), cell.codepoint());
        assert!(!cell.has_grapheme());
        assert_eq!(Wide::Narrow, cell.wide());
    }
}

#[test]
fn terminal_print_writes_to_bottom_if_scrolled() {
    let mut t = init(5, 2);
    print_chars(&mut t, "hello");
    t.set_cursor_pos(0, 0);

    t.index().unwrap();
    t.index().unwrap();
    t.index().unwrap();
    assert_eq!("", plain(&t));

    t.screen_mut().scroll(Scroll::Top);
    assert_eq!("hello", plain(&t));

    t.print(u32::from('A')).unwrap();
    t.screen_mut().scroll(Scroll::Active);
    assert_eq!("\nA", plain(&t));

    let (x, y) = (t.screen().cursor.x, t.screen().cursor.y);
    assert!(t.is_dirty(pt_active(x, u32::from(y))));
}

#[test]
fn terminal_print_charset() {
    let mut t = init(80, 80);
    t.configure_charset(Slots::G1, Charset::DecSpecial);
    t.configure_charset(Slots::G2, Charset::DecSpecial);
    t.configure_charset(Slots::G3, Charset::DecSpecial);

    assert!(!t.is_dirty(pt_screen(0, 0)));

    t.print(u32::from('`')).unwrap();
    t.configure_charset(Slots::G0, Charset::Utf8);
    t.print(u32::from('`')).unwrap();
    t.configure_charset(Slots::G0, Charset::Ascii);
    t.print(u32::from('`')).unwrap();
    t.configure_charset(Slots::G0, Charset::DecSpecial);
    t.print(u32::from('`')).unwrap();
    assert_eq!("```◆", plain(&t));

    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_charset_outside_of_ascii() {
    let mut t = init(80, 80);
    t.configure_charset(Slots::G1, Charset::DecSpecial);
    t.configure_charset(Slots::G2, Charset::DecSpecial);
    t.configure_charset(Slots::G3, Charset::DecSpecial);
    assert!(!t.is_dirty(pt_screen(0, 0)));

    t.configure_charset(Slots::G0, Charset::DecSpecial);
    t.print(u32::from('`')).unwrap();
    t.print(0x1F600).unwrap();
    assert_eq!("◆ ", plain(&t));
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_invoke_charset() {
    let mut t = init(80, 80);
    t.configure_charset(Slots::G1, Charset::DecSpecial);
    t.print(u32::from('`')).unwrap();

    t.clear_dirty();
    t.invoke_charset(charsets::ActiveSlot::GL, Slots::G1, false);
    assert!(!t.is_dirty(pt_screen(0, 0)));
    t.print(u32::from('`')).unwrap();
    assert!(t.is_dirty(pt_screen(0, 0)));
    t.print(u32::from('`')).unwrap();
    t.invoke_charset(charsets::ActiveSlot::GL, Slots::G0, false);
    t.print(u32::from('`')).unwrap();
    assert_eq!("`◆◆`", plain(&t));
}

#[test]
fn terminal_print_invoke_charset_single() {
    let mut t = init(80, 80);
    t.configure_charset(Slots::G1, Charset::DecSpecial);
    t.print(u32::from('`')).unwrap();
    t.invoke_charset(charsets::ActiveSlot::GL, Slots::G1, true);
    t.print(u32::from('`')).unwrap();
    t.print(u32::from('`')).unwrap();
    assert_eq!("`◆`", plain(&t));
}

#[test]
fn terminal_print_kitty_unicode_placeholder() {
    let mut t = init(10, 10);
    t.print(0x10EEEE).unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);
    {
        let cell = cell_at(&t, pt_screen(0, 0));
        assert_eq!(0x10EEEE, cell.codepoint());
        assert!(row_at(&t, pt_screen(0, 0)).kitty_virtual_placeholder());
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_soft_wrap() {
    let mut t = init(3, 80);
    print_chars(&mut t, "hello");
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert_eq!("hel\nlo", plain(&t));
}

#[test]
fn terminal_soft_wrap_with_semantic_prompt() {
    let mut t = init(3, 80);
    t.semantic_prompt(&osc::SemanticPrompt::init(PromptAction::PromptStart))
        .unwrap();
    assert!(!t.is_dirty(pt_screen(0, 0)));

    print_chars(&mut t, "hello");
    assert_eq!(
        RowPrompt::Prompt,
        row_at(&t, pt_screen(0, 0)).semantic_prompt()
    );
    assert_eq!(
        RowPrompt::PromptContinuation,
        row_at(&t, pt_screen(0, 1)).semantic_prompt()
    );
}

#[test]
fn terminal_disabled_wraparound_with_wide_char_and_one_space() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, false);
    print_str(&mut t, "AAAA");
    t.clear_dirty();
    t.print(0x1F6A8).unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);
    assert_eq!("AAAA", plain(&t));
    {
        let cell = cell_at(&t, pt_screen(4, 0));
        assert_eq!(0, cell.codepoint());
        assert_eq!(Wide::Narrow, cell.wide());
    }
    assert!(!t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_disabled_wraparound_with_wide_char_and_no_space() {
    let mut t = init(5, 5);
    t.modes.set(Mode::Wraparound, false);
    print_str(&mut t, "AAAAA");
    t.clear_dirty();
    t.print(0x1F6A8).unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);
    assert_eq!("AAAAA", plain(&t));
    {
        let cell = cell_at(&t, pt_screen(4, 0));
        assert_eq!(u32::from('A'), cell.codepoint());
        assert_eq!(Wide::Narrow, cell.wide());
    }
    assert!(!t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_disabled_wraparound_with_wide_grapheme_and_half_space() {
    let mut t = init(5, 5);
    t.modes.set(Mode::GraphemeCluster, true);
    t.modes.set(Mode::Wraparound, false);
    print_str(&mut t, "AAAA");
    t.print(0x2764).unwrap();
    t.clear_dirty();
    t.print(0xFE0F).unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);
    assert_eq!("AAAA❤", plain(&t));
    {
        let cell = cell_at(&t, pt_screen(4, 0));
        assert_eq!(0x2764, cell.codepoint());
        assert_eq!(Wide::Narrow, cell.wide());
    }
    assert!(!t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_right_margin_wrap() {
    let mut t = init(10, 5);
    print_str(&mut t, "123456789");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 5);
    print_str(&mut t, "XY");
    assert_eq!("1234X6789\n  Y", plain(&t));
    assert!(!row_at(&t, pt_active(0, 0)).wrap());
}

#[test]
fn terminal_print_right_margin_wrap_dirty_tracking() {
    let mut t = init(10, 5);
    print_str(&mut t, "123456789");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 5);

    t.clear_dirty();
    t.print(u32::from('X')).unwrap();
    assert!(t.is_dirty(pt_screen(4, 0)));
    assert!(!t.is_dirty(pt_screen(2, 1)));

    t.clear_dirty();
    t.print(u32::from('Y')).unwrap();
    assert!(t.is_dirty(pt_screen(4, 0)));
    assert!(t.is_dirty(pt_screen(2, 1)));
    assert_eq!("1234X6789\n  Y", plain(&t));
}

#[test]
fn terminal_print_right_margin_outside() {
    let mut t = init(10, 5);
    print_str(&mut t, "123456789");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 6);
    t.clear_dirty();
    print_str(&mut t, "XY");
    assert_eq!("12345XY89", plain(&t));
    assert!(t.is_dirty(pt_screen(5, 0)));
}

#[test]
fn terminal_print_right_margin_outside_wrap() {
    let mut t = init(10, 5);
    print_str(&mut t, "123456789");
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 10);
    print_str(&mut t, "XY");
    assert_eq!("123456789X\n  Y", plain(&t));
}

#[test]
fn terminal_print_wide_char_at_right_margin_does_not_create_spacer_head() {
    let mut t = init(10, 10);
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(3, 5);
    t.set_cursor_pos(1, 5);
    t.print(0x1F600).unwrap();
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);

    assert!(t.is_dirty(pt_screen(4, 0)));
    assert!(t.is_dirty(pt_screen(4, 1)));

    {
        let cell = cell_at(&t, pt_screen(4, 0));
        assert_eq!(0, cell.codepoint());
        assert_eq!(Wide::Narrow, cell.wide());
        assert!(!row_at(&t, pt_screen(4, 0)).wrap());
    }
    {
        let cell = cell_at(&t, pt_screen(2, 1));
        assert_eq!(0x1F600, cell.codepoint());
        assert_eq!(Wide::Wide, cell.wide());
    }
    {
        let cell = cell_at(&t, pt_screen(3, 1));
        assert_eq!(Wide::SpacerTail, cell.wide());
    }
}

#[test]
fn terminal_print_with_hyperlink() {
    let mut t = init(80, 80);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "123456");

    for x in 0..6 {
        assert!(row_at(&t, pt_screen(x, 0)).hyperlink());
        assert!(cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(Some(1), hl_id(&t, pt_screen(x, 0)));
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_over_cell_with_same_hyperlink() {
    let mut t = init(80, 80);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "123456");
    t.set_cursor_pos(1, 1);
    print_str(&mut t, "123456");

    for x in 0..6 {
        assert!(row_at(&t, pt_screen(x, 0)).hyperlink());
        assert!(cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(Some(1), hl_id(&t, pt_screen(x, 0)));
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_and_end_hyperlink() {
    let mut t = init(80, 80);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    print_str(&mut t, "123");
    t.screen_mut().end_hyperlink();
    print_str(&mut t, "456");

    for x in 0..3 {
        assert!(row_at(&t, pt_screen(x, 0)).hyperlink());
        assert!(cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(Some(1), hl_id(&t, pt_screen(x, 0)));
    }
    for x in 3..6 {
        assert!(row_at(&t, pt_screen(x, 0)).hyperlink());
        assert!(!cell_at(&t, pt_screen(x, 0)).hyperlink());
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_and_change_hyperlink() {
    let mut t = init(80, 80);
    t.screen_mut()
        .start_hyperlink(b"http://one.example.com", None)
        .unwrap();
    print_str(&mut t, "123");
    t.screen_mut()
        .start_hyperlink(b"http://two.example.com", None)
        .unwrap();
    print_str(&mut t, "456");

    for x in 0..3 {
        assert!(cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(Some(1), hl_id(&t, pt_screen(x, 0)));
    }
    for x in 3..6 {
        assert!(cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(Some(2), hl_id(&t, pt_screen(x, 0)));
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_overwrite_hyperlink() {
    let mut t = init(80, 80);
    t.screen_mut()
        .start_hyperlink(b"http://one.example.com", None)
        .unwrap();
    print_str(&mut t, "123");
    t.set_cursor_pos(1, 1);
    t.screen_mut().end_hyperlink();
    print_str(&mut t, "456");

    for x in 0..3 {
        let p = pin_at(&t, pt_screen(x, 0));
        assert!(!row_at(&t, pt_screen(x, 0)).hyperlink());
        assert!(!cell_at(&t, pt_screen(x, 0)).hyperlink());
        assert_eq!(None, hl_id(&t, pt_screen(x, 0)));
        assert_eq!(0, t.screen().page(p.node).hyperlink_set.count());
    }
    assert!(t.is_dirty(pt_screen(0, 0)));
}

#[test]
fn terminal_print_wide_char_at_right_edge_with_hyperlink() {
    let mut t = init(10, 5);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.set_cursor_pos(1, 10);
    t.print(0x4E2D).unwrap();

    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    {
        let cell = cell_at(&t, pt_screen(9, 0));
        assert_eq!(Wide::SpacerHead, cell.wide());
        assert!(cell.hyperlink());
        assert!(row_at(&t, pt_screen(9, 0)).wrap());
    }
    {
        let cell = cell_at(&t, pt_screen(0, 1));
        assert_eq!(0x4E2D, cell.codepoint());
        assert_eq!(Wide::Wide, cell.wide());
        assert!(cell.hyperlink());
    }
    {
        let cell = cell_at(&t, pt_screen(1, 1));
        assert_eq!(Wide::SpacerTail, cell.wide());
        assert!(cell.hyperlink());
    }
}

#[test]
fn terminal_linefeed_and_carriage_return() {
    let mut t = init(80, 80);
    print_chars(&mut t, "hello");
    t.clear_dirty();
    t.carriage_return();
    assert!(!t.is_dirty(pt_screen(0, 0)));

    t.linefeed().unwrap();
    assert!(t.is_dirty(pt_screen(0, 0)));
    assert!(t.is_dirty(pt_screen(0, 1)));

    print_chars(&mut t, "world");
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(5, t.screen().cursor.x);
    assert_eq!("hello\nworld", plain(&t));
}

#[test]
fn terminal_linefeed_unsets_pending_wrap() {
    let mut t = init(5, 80);
    print_chars(&mut t, "hello");
    assert!(t.screen().cursor.pending_wrap);
    t.clear_dirty();
    t.linefeed().unwrap();
    assert!(t.is_dirty(pt_screen(0, 0)));
    assert!(t.is_dirty(pt_screen(0, 1)));
    assert!(!t.screen().cursor.pending_wrap);
}

#[test]
fn terminal_linefeed_mode_automatic_carriage_return() {
    let mut t = init(10, 10);
    t.modes.set(Mode::Linefeed, true);
    print_str(&mut t, "123456");
    t.linefeed().unwrap();
    t.print(u32::from('X')).unwrap();
    assert_eq!("123456\nX", plain(&t));
}

#[test]
fn terminal_carriage_return_unsets_pending_wrap() {
    let mut t = init(5, 80);
    print_chars(&mut t, "hello");
    assert!(t.screen().cursor.pending_wrap);
    t.carriage_return();
    assert!(!t.screen().cursor.pending_wrap);
}

#[test]
fn terminal_carriage_return_origin_mode_moves_to_left_margin() {
    let mut t = init(5, 80);
    t.modes.set(Mode::Origin, true);
    t.screen_mut().cursor.x = 0;
    t.scrolling_region.left = 2;
    t.carriage_return();
    assert_eq!(2, t.screen().cursor.x);
}

#[test]
fn terminal_carriage_return_left_of_left_margin_moves_to_zero() {
    let mut t = init(5, 80);
    t.screen_mut().cursor.x = 1;
    t.scrolling_region.left = 2;
    t.carriage_return();
    assert_eq!(0, t.screen().cursor.x);
}

#[test]
fn terminal_carriage_return_right_of_left_margin_moves_to_left_margin() {
    let mut t = init(5, 80);
    t.screen_mut().cursor.x = 3;
    t.scrolling_region.left = 2;
    t.carriage_return();
    assert_eq!(2, t.screen().cursor.x);
}

#[test]
fn terminal_backspace() {
    let mut t = init(80, 80);
    print_chars(&mut t, "hello");
    t.backspace();
    t.print(u32::from('y')).unwrap();
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(5, t.screen().cursor.x);
    assert_eq!("helly", plain(&t));
}

#[test]
fn terminal_horizontal_tabs() {
    let mut t = init(20, 5);
    t.print(u32::from('1')).unwrap();
    t.horizontal_tab();
    assert_eq!(8, t.screen().cursor.x);
    t.horizontal_tab();
    assert_eq!(16, t.screen().cursor.x);
    t.horizontal_tab();
    assert_eq!(19, t.screen().cursor.x);
    t.horizontal_tab();
    assert_eq!(19, t.screen().cursor.x);
}

#[test]
fn terminal_horizontal_tabs_starting_on_tabstop() {
    let mut t = init(20, 5);
    let y = usize::from(t.screen().cursor.y);
    t.set_cursor_pos(y, 9);
    t.print(u32::from('X')).unwrap();
    t.set_cursor_pos(y, 9);
    t.horizontal_tab();
    t.print(u32::from('A')).unwrap();
    assert_eq!("        X       A", plain(&t));
}

#[test]
fn terminal_horizontal_tabs_with_right_margin() {
    let mut t = init(20, 5);
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 5;
    let y = usize::from(t.screen().cursor.y);
    t.set_cursor_pos(y, 1);
    t.print(u32::from('X')).unwrap();
    t.horizontal_tab();
    t.print(u32::from('A')).unwrap();
    assert_eq!("X    A", plain(&t));
}

#[test]
fn terminal_horizontal_tabs_back() {
    let mut t = init(20, 5);
    let y = usize::from(t.screen().cursor.y);
    t.set_cursor_pos(y, 20);
    t.horizontal_tab_back();
    assert_eq!(16, t.screen().cursor.x);
    t.horizontal_tab_back();
    assert_eq!(8, t.screen().cursor.x);
    t.horizontal_tab_back();
    assert_eq!(0, t.screen().cursor.x);
    t.horizontal_tab_back();
    assert_eq!(0, t.screen().cursor.x);
}

#[test]
fn terminal_horizontal_tabs_back_starting_on_tabstop() {
    let mut t = init(20, 5);
    let y = usize::from(t.screen().cursor.y);
    t.set_cursor_pos(y, 9);
    t.print(u32::from('X')).unwrap();
    t.set_cursor_pos(y, 9);
    t.horizontal_tab_back();
    t.print(u32::from('A')).unwrap();
    assert_eq!("A       X", plain(&t));
}

#[test]
fn terminal_horizontal_tabs_with_left_margin_in_origin_mode() {
    let mut t = init(20, 5);
    t.modes.set(Mode::Origin, true);
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 5;
    t.set_cursor_pos(1, 2);
    t.print(u32::from('X')).unwrap();
    t.horizontal_tab_back();
    t.print(u32::from('A')).unwrap();
    assert_eq!("  AX", plain(&t));
}

#[test]
fn terminal_horizontal_tab_back_with_cursor_before_left_margin() {
    let mut t = init(20, 5);
    t.modes.set(Mode::Origin, true);
    t.save_cursor();
    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(5, 0);
    t.restore_cursor();
    t.horizontal_tab_back();
    t.print(u32::from('X')).unwrap();
    assert_eq!("X", plain(&t));
}

#[test]
fn terminal_cursor_pos_resets_wrap() {
    let mut t = init(5, 5);
    print_chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);
    t.set_cursor_pos(1, 1);
    assert!(!t.screen().cursor.pending_wrap);
    t.print(u32::from('X')).unwrap();
    assert_eq!("XBCDE", plain(&t));
}

#[test]
fn terminal_cursor_pos_off_the_screen() {
    let mut t = init(5, 5);
    t.set_cursor_pos(500, 500);
    t.print(u32::from('X')).unwrap();
    assert_eq!("\n\n\n\n    X", plain(&t));
}

#[test]
fn terminal_cursor_pos_relative_to_origin() {
    let mut t = init(5, 5);
    t.scrolling_region.top = 2;
    t.scrolling_region.bottom = 3;
    t.modes.set(Mode::Origin, true);
    t.set_cursor_pos(1, 1);
    t.print(u32::from('X')).unwrap();
    assert_eq!("\n\nX", plain(&t));
}

#[test]
fn terminal_cursor_pos_relative_to_origin_with_left_right() {
    let mut t = init(5, 5);
    t.scrolling_region.top = 2;
    t.scrolling_region.bottom = 3;
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    t.modes.set(Mode::Origin, true);
    t.set_cursor_pos(1, 1);
    t.print(u32::from('X')).unwrap();
    assert_eq!("\n\n  X", plain(&t));
}

#[test]
fn terminal_cursor_pos_limits_with_full_scroll_region() {
    let mut t = init(5, 5);
    t.scrolling_region.top = 2;
    t.scrolling_region.bottom = 3;
    t.scrolling_region.left = 2;
    t.scrolling_region.right = 4;
    t.modes.set(Mode::Origin, true);
    t.set_cursor_pos(500, 500);
    t.print(u32::from('X')).unwrap();
    assert_eq!("\n\n\n    X", plain(&t));
}

#[test]
fn terminal_set_cursor_pos_original_test() {
    let mut t = init(80, 80);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor.y);

    t.set_cursor_pos(0, 0);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor.y);

    t.set_cursor_pos(81, 81);
    assert_eq!(79, t.screen().cursor.x);
    assert_eq!(79, t.screen().cursor.y);

    t.set_cursor_pos(0, 80);
    t.print(u32::from('c')).unwrap();
    assert!(t.screen().cursor.pending_wrap);
    t.set_cursor_pos(0, 80);
    assert!(!t.screen().cursor.pending_wrap);

    t.modes.set(Mode::Origin, true);

    t.set_cursor_pos(81, 81);
    assert_eq!(79, t.screen().cursor.x);
    assert_eq!(79, t.screen().cursor.y);

    t.set_top_and_bottom_margin(10, usize::from(t.rows));
    t.set_cursor_pos(0, 0);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(9, t.screen().cursor.y);

    t.set_cursor_pos(1, 1);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(9, t.screen().cursor.y);

    t.set_cursor_pos(100, 0);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(79, t.screen().cursor.y);

    t.set_top_and_bottom_margin(10, 11);
    t.set_cursor_pos(2, 0);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!(10, t.screen().cursor.y);
}

fn abc_def_ghi(t: &mut Terminal) {
    print_str(t, "ABC");
    t.carriage_return();
    t.linefeed().unwrap();
    print_str(t, "DEF");
    t.carriage_return();
    t.linefeed().unwrap();
    print_str(t, "GHI");
}

#[test]
fn terminal_set_top_and_bottom_margin_simple() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(0, 0);

    t.clear_dirty();
    t.scroll_down(1);

    for y in 0..4 {
        assert!(t.is_dirty(pt_active(0, y)));
    }
    assert_eq!("\nABC\nDEF\nGHI", plain(&t));
}

#[test]
fn terminal_set_top_and_bottom_margin_top_only() {
    let mut t = init(5, 5);
    abc_def_ghi(&mut t);
    t.set_top_and_bottom_margin(2, 0);

    t.clear_dirty();
    t.scroll_down(1);

    for y in 0..4 {
        assert!(t.is_dirty(pt_active(0, y)));
    }
    assert_eq!("ABC\n\nDEF\nGHI", plain(&t));
}
