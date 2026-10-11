use super::*;
use crate::page::{Cell, Wide};
use ghostty_vt::color::Rgb;
use ghostty_vt::sgr::Attribute;

fn c(t: &Terminal, x: u16, y: u32) -> Cell {
    cell_at(t, pt_screen(x, y))
}

fn dirty(t: &Terminal, x: u16, y: u32) -> bool {
    t.is_dirty(pt_screen(x, y))
}

fn print_all(t: &mut Terminal, cps: &[u32]) {
    for &cp in cps {
        t.print(cp).unwrap();
    }
}

fn cur(t: &Terminal) -> (u16, u16) {
    (t.screen().cursor.x, t.screen().cursor.y)
}

#[test]
fn terminal_input_with_no_control_characters() {
    let mut t = init(40, 40);

    for ch in "hello".chars() {
        t.print(ch as u32).unwrap();
    }
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(5, t.screen().cursor.x);
    assert_eq!("hello", plain(&t));

    // The first row should be dirty
    assert!(dirty(&t, 5, 0));
    assert!(!dirty(&t, 5, 1));
}

#[test]
fn terminal_input_with_basic_wraparound() {
    let mut t = init(5, 40);

    for ch in "helloworldabc12".chars() {
        t.print(ch as u32).unwrap();
    }
    assert_eq!(2, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);
    assert_eq!("hello\nworld\nabc12", plain(&t));
}

#[test]
fn terminal_input_with_basic_wraparound_dirty() {
    let mut t = init(5, 40);

    for ch in "hello".chars() {
        t.print(ch as u32).unwrap();
    }
    assert!(dirty(&t, 4, 0));
    t.clear_dirty();
    t.print('w' as u32).unwrap();

    // Old row is dirty because cursor moved from there
    assert!(dirty(&t, 4, 0));
    assert!(dirty(&t, 0, 1));
}

#[test]
fn terminal_input_that_forces_scroll() {
    let mut t = init(1, 5);

    for ch in "abcdef".chars() {
        t.print(ch as u32).unwrap();
    }
    assert_eq!(4, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);
    assert_eq!("b\nc\nd\ne\nf", plain(&t));
}

#[test]
fn terminal_input_unique_style_per_cell() {
    let mut t = init(30, 30);

    for y in 0..t.rows {
        for x in 0..t.cols {
            t.set_cursor_pos(usize::from(y), usize::from(x));
            t.set_attribute(Attribute::DirectColorBg(Rgb {
                r: x as u8,
                g: y as u8,
                b: 0,
            }))
            .unwrap();
            t.print('x' as u32).unwrap();
        }
    }
}

#[test]
fn terminal_input_glitch_text() {
    let glitch = include_str!("glitch.txt");
    let mut t = init(30, 30);

    // Get our initial grapheme capacity.
    let first = |t: &Terminal| t.screen().pages.first.unwrap();
    let grapheme_cap = t.screen().page(first(&t)).capacity.grapheme_bytes;

    // Print glitch text until our capacity changes
    loop {
        let node = first(&t);
        if t.screen().page(node).capacity.grapheme_bytes != grapheme_cap {
            break;
        }
        t.print_string(glitch).unwrap();
    }

    // We're testing to make sure that grapheme capacity gets increased.
    let node = first(&t);
    assert!(t.screen().page(node).capacity.grapheme_bytes > grapheme_cap);
}

#[test]
fn terminal_zero_width_character_at_start() {
    let mut t = init(80, 80);

    // This used to crash the terminal. This is not allowed so we should
    // just ignore it.
    t.print(0x200D).unwrap();

    assert_eq!((0, 0), cur(&t));

    // Should not be dirty since we changed nothing.
    assert!(!dirty(&t, 0, 0));
}

// https://github.com/mitchellh/ghostty/issues/1400
#[test]
fn terminal_print_single_very_long_line() {
    let mut t = init(5, 5);

    // This would crash for issue 1400. So the assertion here is
    // that we simply do not crash.
    for _ in 0..1000 {
        t.print('x' as u32).unwrap();
    }
}

#[test]
fn terminal_print_wide_char() {
    let mut t = init(80, 80);

    t.print(0x1F600).unwrap(); // Smiley face
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F600, cell.codepoint());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(Wide::SpacerTail, c(&t, 1, 0).wide());

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_wide_char_at_edge_creates_spacer_head() {
    let mut t = init(10, 10);

    t.set_cursor_pos(1, 10);
    t.print(0x1F600).unwrap(); // Smiley face
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    assert_eq!(Wide::SpacerHead, c(&t, 9, 0).wide());
    let cell = c(&t, 0, 1);
    assert_eq!(0x1F600, cell.codepoint());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(Wide::SpacerTail, c(&t, 1, 1).wide());

    // Old row is dirty because cursor moved from there
    assert!(dirty(&t, 0, 0));
    assert!(dirty(&t, 0, 1));
}

#[test]
fn terminal_print_wide_char_with_1_column_width() {
    let mut t = init(1, 2);

    t.print(0x1F600).unwrap();

    // This prints a space so we should be dirty.
    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_wide_char_in_single_width_terminal() {
    let mut t = init(1, 80);

    t.print(0x1F600).unwrap(); // Smiley face
    assert_eq!((0, 0), cur(&t));
    assert!(t.screen().cursor.pending_wrap);

    let cell = c(&t, 0, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_over_wide_char_at_0_0() {
    let mut t = init(80, 80);

    t.print(0x1F600).unwrap(); // Smiley face
    t.set_cursor_pos(0, 0);
    t.print('A' as u32).unwrap();

    assert_eq!((1, 0), cur(&t));

    let cell = c(&t, 0, 0);
    assert_eq!('A' as u32, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());
    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());

    assert!(dirty(&t, 0, 0));
    assert!(!dirty(&t, 0, 1));
}

#[test]
fn terminal_print_over_wide_char_at_col_0_corrupts_previous_row() {
    // Crash found by AFL++ fuzzer (afl-out/stream/default/crashes/id:000002).
    //
    // printCell, when overwriting a wide cell with a narrow cell at x<=1
    // and y>0, sets the last cell of the previous row to .narrow — even
    // when that cell is a .spacer_tail rather than a .spacer_head. This
    // orphans the .wide cell at cols-2.
    let mut t = init(10, 3);

    // Fill rows 0 and 1 with wide chars (5 per row on a 10-col terminal).
    for _ in 0..10 {
        t.print(0x4E2D).unwrap();
    }

    // Move cursor to row 1, col 0 (on top of a wide char) and print a
    // narrow character.
    t.set_cursor_pos(2, 1);
    t.print('A' as u32).unwrap();

    // Row 1, col 0 should be narrow (we just overwrote the wide char).
    assert_eq!(Wide::Narrow, c(&t, 0, 1).wide());
    // Row 0, col 8 should still be .wide (the last wide char on the row).
    assert_eq!(Wide::Wide, c(&t, 8, 0).wide());
    // Row 0, col 9 must remain .spacer_tail to pair with the .wide at col 8.
    assert_eq!(Wide::SpacerTail, c(&t, 9, 0).wide());
}

#[test]
fn terminal_print_over_wide_spacer_tail() {
    let mut t = init(5, 5);

    t.print('橋' as u32).unwrap();
    t.set_cursor_pos(1, 2);
    t.print('X' as u32).unwrap();

    let cell = c(&t, 0, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());
    let cell = c(&t, 1, 0);
    assert_eq!('X' as u32, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());

    assert_eq!(" X", plain(&t));

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_over_wide_char_with_bold() {
    let mut t = init(80, 80);

    t.set_attribute(Attribute::Bold).unwrap();
    t.print(0x1F600).unwrap(); // Smiley face
    // verify we have styles in our style map
    assert_eq!(1, t.screen().cursor_page().styles.count());

    // Go back and overwrite with no style
    t.set_cursor_pos(0, 0);
    t.set_attribute(Attribute::Unset).unwrap();
    t.print('A' as u32).unwrap(); // Smiley face

    // verify our style is gone
    assert_eq!(0, t.screen().cursor_page().styles.count());

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_over_wide_char_with_bg_color() {
    let mut t = init(80, 80);

    t.set_attribute(Attribute::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0,
    }))
    .unwrap();
    t.print(0x1F600).unwrap(); // Smiley face
    // verify we have styles in our style map
    assert_eq!(1, t.screen().cursor_page().styles.count());

    // Go back and overwrite with no style
    t.set_cursor_pos(0, 0);
    t.set_attribute(Attribute::Unset).unwrap();
    t.print('A' as u32).unwrap(); // Smiley face

    // verify our style is gone
    assert_eq!(0, t.screen().cursor_page().styles.count());

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_print_multicodepoint_grapheme_disabled_mode_2027() {
    let mut t = init(80, 80);

    // https://github.com/mitchellh/ghostty/issues/289
    // This is: 👨‍👩‍👧 (which may or may not render correctly)
    print_all(&mut t, &[0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467]);

    // We should have 6 cells taken up
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(6, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F468, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
    assert!(graphemes_at(&t, pt_screen(1, 0)).is_none());

    let cell = c(&t, 2, 0);
    assert_eq!(0x1F469, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(2, 0)).unwrap().len());

    let cell = c(&t, 3, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
    assert!(graphemes_at(&t, pt_screen(3, 0)).is_none());

    let cell = c(&t, 4, 0);
    assert_eq!(0x1F467, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
    assert!(graphemes_at(&t, pt_screen(4, 0)).is_none());

    let cell = c(&t, 5, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
    assert!(graphemes_at(&t, pt_screen(5, 0)).is_none());

    assert!(dirty(&t, 0, 0));
}

#[test]
fn terminal_vs16_doesnt_make_character_with_2027_disabled() {
    let mut t = init(5, 5);

    // Disable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, false);

    t.print(0x2764).unwrap(); // Heart
    t.print(0xFE0F).unwrap(); // VS16 to make wide

    assert_eq!("❤️", plain(&t));

    let cell = c(&t, 0, 0);
    assert_eq!(0x2764, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());
}

#[test]
fn terminal_ignored_vs16_doesnt_mark_dirty() {
    let mut t = init(5, 5);

    // Disable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, false);

    t.print(0x2764).unwrap(); // Heart
    assert!(dirty(&t, 0, 0));

    t.clear_dirty();
    t.print(0xFE0F).unwrap(); // VS16 to make wide
    assert!(!dirty(&t, 0, 0));
}

#[test]
fn terminal_print_invalid_vs16_non_grapheme() {
    let mut t = init(80, 80);

    // https://github.com/mitchellh/ghostty/issues/1482
    t.print('x' as u32).unwrap();
    t.print(0xFE0F).unwrap();

    // We should have 1 narrow cell.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!('x' as u32, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
    assert_eq!(0, c(&t, 1, 0).codepoint());
}

#[test]
fn terminal_invalid_vs16_doesnt_mark_dirty() {
    let mut t = init(5, 5);

    // Disable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, false);

    t.print('x' as u32).unwrap();
    assert!(dirty(&t, 0, 0));

    t.clear_dirty();
    t.print(0xFE0F).unwrap(); // VS16 to make wide
    assert!(!dirty(&t, 0, 0));
}

#[test]
fn terminal_print_multicodepoint_grapheme_mode_2027() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/289
    // This is: 👨‍👩‍👧 (which may or may not render correctly)
    print_all(&mut t, &[0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467]);

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    // Row should be dirty
    assert!(dirty(&t, 0, 0));

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F468, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(4, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_keypad_sequence_vs15() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // This is: "#︎" (number sign with text presentation selector)
    t.print(0x23).unwrap(); // # Number sign (valid base)
    t.print(0xFE0E).unwrap(); // VS15 (text presentation selector)

    // VS15 should combine with the base character into a single grapheme cluster,
    // taking 1 cell (narrow character).
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    // Row should be dirty
    assert!(dirty(&t, 0, 0));

    let cell = c(&t, 0, 0);
    assert_eq!(0x23, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_keypad_sequence_vs16() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // This is: "#️" (number sign with emoji presentation selector)
    t.print(0x23).unwrap(); // # Number sign (valid base)
    t.print(0xFE0F).unwrap(); // VS16 (emoji presentation selector)

    // VS16 should combine with the base character into a single grapheme cluster,
    // taking 2 cells (wide character).
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    // Row should be dirty
    assert!(dirty(&t, 0, 0));

    let cell = c(&t, 0, 0);
    assert_eq!(0x23, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
}

#[test]
fn terminal_fitzpatrick_skin_tone_next_valid_base() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // This is: "👋🏿" (waving hand with dark skin tone)
    t.print(0x1F44B).unwrap(); // 👋 Waving hand (valid base)
    t.print(0x1F3FF).unwrap(); // 🏿 Dark skin tone modifier

    // The skin tone should combine with the base emoji into a single grapheme cluster,
    // taking 2 cells (wide character).
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    // Row should be dirty
    assert!(dirty(&t, 0, 0));

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F44B, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
}

#[test]
fn terminal_fitzpatrick_skin_tone_next_to_non_base() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x22).unwrap(); // "
    t.print(0x1F3FF).unwrap(); // Dark skin tone
    t.print(0x22).unwrap(); // "

    // We should have 4 cells taken up. Importantly, the skin tone
    // should not join with the quotes.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(4, t.screen().cursor.x);

    // Row should be dirty
    assert!(dirty(&t, 0, 0));

    let cell = c(&t, 0, 0);
    assert_eq!(0x22, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!(0x1F3FF, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());

    let cell = c(&t, 3, 0);
    assert_eq!(0x22, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_multicodepoint_grapheme_marks_dirty_on_every_codepoint() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/289
    // This is: 👨‍👩‍👧 (which may or may not render correctly)
    t.print(0x1F468).unwrap();
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0x200D).unwrap();
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0x1F469).unwrap();
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0x200D).unwrap();
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0x1F467).unwrap();
    assert!(dirty(&t, 0, 0));

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
}

#[test]
fn terminal_vs15_to_make_narrow_character() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x2614).unwrap(); // Umbrella with rain drops, width=2
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    t.print(0xFE0E).unwrap(); // VS15 to make narrow
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    // VS15 should send us back a cell since our char is no longer wide.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    assert_eq!("☔︎", plain(&t));

    let cell = c(&t, 0, 0);
    assert_eq!(0x2614, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());
}

#[test]
fn terminal_vs15_on_already_narrow_emoji() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x26C8).unwrap(); // Thunder cloud and rain, width=1
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0xFE0E).unwrap(); // VS15 to make narrow
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    // Character takes up one cell
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    assert_eq!("⛈︎", plain(&t));

    let cell = c(&t, 0, 0);
    assert_eq!(0x26C8, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());
}

#[test]
fn terminal_print_invalid_vs15_following_emoji_is_wide() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x1F9E0).unwrap(); // 🧠
    t.print(0xFE0E).unwrap(); // not valid with U+1F9E0 as base

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F9E0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_print_invalid_vs15_in_emoji_zwj_sequence() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x1F469).unwrap(); // 👩
    t.print(0xFE0E).unwrap(); // not valid with U+1F469 as base
    t.print(0x200D).unwrap(); // ZWJ
    t.print(0x1F466).unwrap(); // 👦

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!(0x1F469, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(
        vec![0x200D, 0x1F466],
        graphemes_at(&t, pt_screen(0, 0)).unwrap()
    );
    assert_eq!(Wide::Wide, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_vs15_to_make_narrow_character_with_pending_wrap() {
    let mut t = init(4, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    assert!(t.modes.get(Mode::Wraparound));

    t.print(0x1F34B).unwrap(); // Lemon, width=2
    t.print(0x2614).unwrap(); // Umbrella with rain drops, width=2
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    // We only move to the end of the line because we're in a pending wrap
    // state.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(3, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);

    t.print(0xFE0E).unwrap(); // VS15 to make narrow
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    // VS15 should clear the pending wrap state
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(3, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    assert_eq!("🍋☔︎", plain(&t));

    let cell = c(&t, 2, 0);
    assert_eq!(0x2614, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(2, 0)).unwrap().len());

    // VS15 should not affect the previous grapheme
    let lemon = c(&t, 0, 0);
    assert_eq!(0x1F34B, lemon.codepoint());
    assert_eq!(Wide::Wide, lemon.wide());
    let spacer = c(&t, 1, 0);
    assert_eq!(0, spacer.codepoint());
    assert_eq!(Wide::SpacerTail, spacer.wide());
}

#[test]
fn terminal_vs16_to_make_wide_character_on_next_line() {
    let mut t = init(3, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.cursor_right(2);
    t.print('#' as u32).unwrap();
    assert_eq!(2, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);
    assert!(dirty(&t, 2, 0));
    t.clear_dirty();

    t.print(0xFE0F).unwrap(); // VS16 to make wide

    assert!(dirty(&t, 2, 0));
    t.clear_dirty();
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    // Previous cell turns into spacer_head
    let cell = c(&t, 2, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerHead, cell.wide());

    // '#' cell is wide
    let cell = c(&t, 0, 1);
    assert_eq!('#' as u32, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0xFE0F], graphemes_at(&t, pt_screen(0, 1)).unwrap());
    assert_eq!(Wide::Wide, cell.wide());

    // spacer_tail
    let cell = c(&t, 1, 1);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_vs16_to_make_wide_character_on_next_line_with_hyperlink() {
    // Regression test for the crash fixed in print's grapheme `.wide` path:
    // writing a spacer_head at the screen edge before row.wrap was set.
    let mut t = init(3, 5);

    // Enable grapheme clustering and activate a hyperlink so printCell
    // calls cursorSetHyperlink (which runs page integrity checks).
    t.modes.set(Mode::GraphemeCluster, true);
    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();

    t.cursor_right(2);
    t.print('#' as u32).unwrap();
    assert_eq!(2, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);

    // Without the fix, this panicked with UnwrappedSpacerHead.
    t.print(0xFE0F).unwrap(); // VS16 to make wide

    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    // Previous cell turns into spacer_head and remains hyperlinked.
    let cell = c(&t, 2, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::SpacerHead, cell.wide());
    assert!(cell.hyperlink());
    assert!(row_at(&t, pt_screen(2, 0)).wrap());

    // '#' cell is now wide and still hyperlinked.
    let cell = c(&t, 0, 1);
    assert_eq!('#' as u32, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0xFE0F], graphemes_at(&t, pt_screen(0, 1)).unwrap());
    assert_eq!(Wide::Wide, cell.wide());
    assert!(cell.hyperlink());

    // spacer_tail inherits hyperlink as part of the same grapheme cell.
    let cell = c(&t, 1, 1);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::SpacerTail, cell.wide());
    assert!(cell.hyperlink());
}

#[test]
fn terminal_vs16_to_make_wide_character_with_pending_wrap() {
    let mut t = init(3, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.cursor_right(1);
    t.print('#' as u32).unwrap();
    assert_eq!(2, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    t.print(0xFE0F).unwrap(); // VS16 to make wide

    assert_eq!(2, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor.y);
    assert!(t.screen().cursor.pending_wrap);

    // '#' cell is wide
    let cell = c(&t, 1, 0);
    assert_eq!('#' as u32, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0xFE0F], graphemes_at(&t, pt_screen(1, 0)).unwrap());
    assert_eq!(Wide::Wide, cell.wide());

    // spacer_tail
    let cell = c(&t, 2, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_vs16_to_make_wide_character_with_mode_2027() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x2764).unwrap(); // Heart
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();
    t.print(0xFE0F).unwrap(); // VS16 to make wide
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    assert_eq!("❤️", plain(&t));

    let cell = c(&t, 0, 0);
    assert_eq!(0x2764, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(1, graphemes_at(&t, pt_screen(0, 0)).unwrap().len());
}

#[test]
fn terminal_vs16_repeated_with_mode_2027() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x2764).unwrap(); // Heart
    t.print(0xFE0F).unwrap(); // VS16 to make wide
    t.print(0x2764).unwrap(); // Heart
    t.print(0xFE0F).unwrap(); // VS16 to make wide

    assert!(dirty(&t, 0, 0));

    assert_eq!("❤️❤️", plain(&t));

    for x in [0, 2] {
        let cell = c(&t, x, 0);
        assert_eq!(0x2764, cell.codepoint());
        assert!(cell.has_grapheme());
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!(1, graphemes_at(&t, pt_screen(x, 0)).unwrap().len());
    }
}

#[test]
fn terminal_print_invalid_vs16_grapheme() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/1482
    t.print('x' as u32).unwrap();
    t.print(0xFE0F).unwrap(); // invalid VS16

    // We should have 1 cells taken up, and narrow.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!('x' as u32, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_print_invalid_vs16_with_second_char() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/1482
    t.print('x' as u32).unwrap();
    t.print(0xFE0F).unwrap();
    t.print('y' as u32).unwrap();

    // We should have 2 cells taken up, from two separate narrow characters.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!('x' as u32, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!('y' as u32, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_print_grapheme_o_with_nonspacing_mark_should_be_narrow() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print('o' as u32).unwrap();
    t.print(0x0300).unwrap(); // combining grave accent

    // We should have 1 cell taken up.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!('o' as u32, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0x0300], graphemes_at(&t, pt_screen(0, 0)).unwrap());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_print_devanagari_grapheme_should_be_wide() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // क्‍ष
    print_all(&mut t, &[0x0915, 0x094D, 0x200D, 0x0937]);

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!(0x0915, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(
        vec![0x094D, 0x200D, 0x0937],
        graphemes_at(&t, pt_screen(0, 0)).unwrap()
    );
    assert_eq!(Wide::Wide, cell.wide());
    assert_eq!(Wide::SpacerTail, c(&t, 1, 0).wide());
}

#[test]
fn terminal_print_devanagari_grapheme_should_be_wide_on_next_line() {
    let mut t = init(3, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.cursor_right(2);

    // क्‍ष
    print_all(&mut t, &[0x0915, 0x094D, 0x200D]);
    assert_eq!(2, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);

    // This one increases the width to wide
    t.print(0x0937).unwrap();

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    // Previous cell turns into spacer_head
    let cell = c(&t, 2, 0);
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerHead, cell.wide());

    // Devanagari grapheme is wide
    let cell = c(&t, 0, 1);
    assert_eq!(0x0915, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(
        vec![0x094D, 0x200D, 0x0937],
        graphemes_at(&t, pt_screen(0, 1)).unwrap()
    );
    assert_eq!(Wide::Wide, cell.wide());

    assert_eq!(Wide::SpacerTail, c(&t, 1, 1).wide());
}

#[test]
fn terminal_print_devanagari_grapheme_should_be_wide_on_next_page() {
    let rows = crate::page::STD_CAPACITY.rows;
    let cols = crate::page::STD_CAPACITY.cols;
    let mut t = init(cols, rows);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.cursor_down(usize::from(rows - 1));

    let first = t.screen().pages.first.unwrap();
    let first_rows = t.screen().page(first).capacity.rows;
    for _ in rows..first_rows {
        t.index().unwrap();
    }

    t.cursor_right(usize::from(cols - 1));

    assert_eq!(cols - 1, t.screen().cursor.x);
    assert_eq!(rows - 1, t.screen().cursor.y);

    // क्‍ष
    print_all(&mut t, &[0x0915, 0x094D, 0x200D]);
    assert_eq!(cols - 1, t.screen().cursor.x);
    assert!(t.screen().cursor.pending_wrap);

    // This one increases the width to wide
    t.print(0x0937).unwrap();

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(rows - 1, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);
    assert!(!t.screen().cursor.pending_wrap);

    // Previous cell turns into spacer_head
    let cell = cell_at(&t, pt_active(cols - 1, u32::from(rows) - 2));
    assert_eq!(0, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::SpacerHead, cell.wide());

    // Devanagari grapheme is wide
    let pt = pt_active(0, u32::from(rows) - 1);
    let cell = cell_at(&t, pt);
    assert_eq!(0x0915, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0x094D, 0x200D, 0x0937], graphemes_at(&t, pt).unwrap());
    assert_eq!(Wide::Wide, cell.wide());

    let cell = cell_at(&t, pt_active(1, u32::from(rows) - 1));
    assert_eq!(Wide::SpacerTail, cell.wide());
}

#[test]
fn terminal_print_invalid_vs16_with_second_char_combining() {
    let mut t = init(80, 80);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/1482
    t.print('n' as u32).unwrap();
    t.print(0xFE0F).unwrap(); // invalid VS16
    t.print(0x0303).unwrap(); // combining tilde

    // We should have 1 cells taken up, and narrow.
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);

    let cell = c(&t, 0, 0);
    assert_eq!('n' as u32, cell.codepoint());
    assert!(cell.has_grapheme());
    assert_eq!(vec![0x0303], graphemes_at(&t, pt_screen(0, 0)).unwrap());
    assert_eq!(Wide::Narrow, cell.wide());

    let cell = c(&t, 1, 0);
    assert_eq!(0, cell.codepoint());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_overwrite_grapheme_should_clear_grapheme_data() {
    let mut t = init(5, 5);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    t.print(0x26C8).unwrap(); // Thunder cloud and rain
    t.print(0xFE0E).unwrap(); // VS15 to make narrow
    assert!(dirty(&t, 0, 0));
    t.clear_dirty();

    t.set_cursor_pos(1, 1);
    t.print('A' as u32).unwrap();
    assert!(dirty(&t, 0, 0));

    assert_eq!("A", plain(&t));

    let cell = c(&t, 0, 0);
    assert_eq!('A' as u32, cell.codepoint());
    assert!(!cell.has_grapheme());
    assert_eq!(Wide::Narrow, cell.wide());
}

#[test]
fn terminal_overwrite_multicodepoint_grapheme_clears_grapheme_data() {
    let mut t = init(10, 10);

    // Enable grapheme clustering
    t.modes.set(Mode::GraphemeCluster, true);

    // https://github.com/mitchellh/ghostty/issues/289
    // This is: 👨‍👩‍👧 (which may or may not render correctly)
    print_all(&mut t, &[0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467]);

    // We should have 2 cells taken up. It is one character but "wide".
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(2, t.screen().cursor.x);

    // We should have one cell with graphemes
    assert_eq!(1, t.screen().cursor_page().grapheme_count());

    // Move back and overwrite wide
    t.set_cursor_pos(1, 1);
    t.clear_dirty();
    t.print('X' as u32).unwrap();
    assert!(dirty(&t, 0, 0));

    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(1, t.screen().cursor.x);
    assert_eq!(0, t.screen().cursor_page().grapheme_count());

    assert_eq!("X", plain(&t));
}
