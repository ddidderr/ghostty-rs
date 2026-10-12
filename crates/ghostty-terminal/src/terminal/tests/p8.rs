use ghostty_vt::{
    color::Rgb,
    osc::{
        SemanticPrompt as Cmd,
        semantic_prompt::{Action as A, Click},
    },
    sgr::Attribute,
};

use super::*;
use crate::{
    page::{ContentTag, SemanticContent, SemanticPrompt as RowPrompt},
    screen::SemanticClick,
};

fn sp(a: A) -> Cmd {
    Cmd::init(a)
}

fn sp_opts(a: A, o: &str) -> Cmd {
    Cmd {
        action: a,
        options_unvalidated: o.as_bytes().to_vec(),
    }
}

fn print_chars(t: &mut Terminal, s: &str) {
    for c in s.chars() {
        t.print(u32::from(c)).unwrap();
    }
}

fn row_prompt(t: &Terminal, x: u16, y: u32) -> RowPrompt {
    row_at(t, pt_active(x, y)).semantic_prompt()
}

fn at_cursor(t: &Terminal) -> crate::page::Cell {
    let c = &t.screen().cursor;
    cell_at(t, pt_active(c.x, u32::from(c.y)))
}

#[test]
fn terminal_semantic_prompt_continuations() {
    let mut t = init(10, 5);

    // Prompt
    t.semantic_prompt(&sp(A::FreshLineNewPrompt)).unwrap();
    print_chars(&mut t, "hello");
    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(5, t.screen().cursor.x);
    {
        let cell = cell_at(&t, pt_active(t.screen().cursor.x - 1, 0));
        assert_eq!(SemanticContent::Prompt, cell.semantic_content());
        assert_eq!(RowPrompt::Prompt, row_prompt(&t, 0, 0));
    }

    // Start input but end it on EOL
    t.carriage_return();
    t.linefeed().unwrap();
    t.semantic_prompt(&sp_opts(A::PromptStart, "k=c")).unwrap();

    // Write some output
    assert_eq!(1, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);
    print_chars(&mut t, "world");
    {
        let cell = cell_at(&t, pt_active(t.screen().cursor.x - 1, 1));
        assert_eq!(SemanticContent::Prompt, cell.semantic_content());
        assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));
    }
}

#[test]
fn terminal_index_in_prompt_mode_marks_new_row_as_prompt_continuation() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "hello");
    assert_eq!(RowPrompt::Prompt, row_prompt(&t, 0, 0));

    t.carriage_return();
    t.linefeed().unwrap();

    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));
    assert_eq!(SemanticContent::Prompt, t.screen().cursor.semantic_content);
}

#[test]
fn terminal_index_in_input_mode_does_not_mark_new_row_as_prompt() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "$ ");
    t.semantic_prompt(&sp(A::EndPromptStartInput)).unwrap();
    print_chars(&mut t, "echo \\");

    t.carriage_return();
    t.linefeed().unwrap();

    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));
    assert_eq!(SemanticContent::Input, t.screen().cursor.semantic_content);
}

#[test]
fn terminal_index_in_output_mode_does_not_mark_new_row_as_prompt() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "$ ");
    t.semantic_prompt(&sp(A::EndPromptStartInput)).unwrap();
    print_chars(&mut t, "ls");
    t.semantic_prompt(&sp(A::EndInputStartOutput)).unwrap();

    t.carriage_return();
    t.linefeed().unwrap();

    assert_eq!(RowPrompt::None, row_prompt(&t, 0, 1));
}

#[test]
fn terminal_osc133c_at_x0_on_prompt_row_clears_prompt_mark() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "$ echo \\");

    t.carriage_return();
    t.linefeed().unwrap();
    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));

    assert_eq!(0, t.screen().cursor.x);
    t.semantic_prompt(&sp(A::EndInputStartOutput)).unwrap();

    assert_eq!(RowPrompt::None, row_prompt(&t, 0, 1));
}

#[test]
fn terminal_osc133c_at_x_gt0_on_prompt_row_does_not_clear_prompt_mark() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "$ ");

    t.carriage_return();
    t.linefeed().unwrap();
    t.semantic_prompt(&sp_opts(A::PromptStart, "k=c")).unwrap();
    print_chars(&mut t, "> ");

    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));

    assert!(t.screen().cursor.x > 0);
    t.semantic_prompt(&sp(A::EndInputStartOutput)).unwrap();

    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));
}

#[test]
fn terminal_multiple_newlines_in_prompt_mode_marks_all_rows() {
    let mut t = init(10, 5);

    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    print_chars(&mut t, "line1");

    t.carriage_return();
    t.linefeed().unwrap();
    print_chars(&mut t, "line2");
    t.carriage_return();
    t.linefeed().unwrap();
    print_chars(&mut t, "line3");

    assert_eq!(RowPrompt::Prompt, row_prompt(&t, 0, 0));
    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 1));
    assert_eq!(RowPrompt::PromptContinuation, row_prompt(&t, 0, 2));
}

#[test]
fn terminal_osc133a_click_events_1_sets_click_to_click_events() {
    let mut t = init(10, 5);
    assert_eq!(SemanticClick::None, t.screen().semantic_prompt.click);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "click_events=1"))
        .unwrap();
    assert_eq!(SemanticClick::ClickEvents, t.screen().semantic_prompt.click);
}

#[test]
fn terminal_osc133a_click_events_0_does_not_set_click_events() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "click_events=0"))
        .unwrap();
    assert_eq!(SemanticClick::None, t.screen().semantic_prompt.click);
}

#[test]
fn terminal_osc133a_cl_option_sets_click_to_cl_value() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "cl=m"))
        .unwrap();
    assert_eq!(
        SemanticClick::Cl(Click::Multiple),
        t.screen().semantic_prompt.click
    );
}

#[test]
fn terminal_osc133a_cl_line_sets_click_to_line() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "cl=line"))
        .unwrap();
    assert_eq!(
        SemanticClick::Cl(Click::Line),
        t.screen().semantic_prompt.click
    );
}

#[test]
fn terminal_osc133a_click_events_1_takes_priority_over_cl() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "click_events=1;cl=m"))
        .unwrap();
    assert_eq!(SemanticClick::ClickEvents, t.screen().semantic_prompt.click);
}

#[test]
fn terminal_osc133a_click_events_0_falls_back_to_cl() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "click_events=0;cl=v"))
        .unwrap();
    assert_eq!(
        SemanticClick::Cl(Click::ConservativeVertical),
        t.screen().semantic_prompt.click
    );
}

#[test]
fn terminal_osc133a_no_click_options_leaves_click_as_none() {
    let mut t = init(10, 5);
    t.semantic_prompt(&sp_opts(A::FreshLineNewPrompt, "aid=123"))
        .unwrap();
    assert_eq!(SemanticClick::None, t.screen().semantic_prompt.click);
}

#[test]
fn terminal_cursor_is_at_prompt() {
    let mut t = init(10, 3);

    assert!(!t.cursor_is_at_prompt());
    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    assert!(t.cursor_is_at_prompt());
    print_chars(&mut t, "$ ");

    // Input is also a prompt
    t.semantic_prompt(&sp(A::EndPromptStartInput)).unwrap();
    assert!(t.cursor_is_at_prompt());
    print_chars(&mut t, "ls");

    // (cursor is not at x=0, so the Fish heuristic doesn't trigger)
    t.semantic_prompt(&sp(A::EndInputStartOutput)).unwrap();
    // Still a prompt because this line has a prompt
    assert!(t.cursor_is_at_prompt());
    t.linefeed().unwrap();
    assert!(!t.cursor_is_at_prompt());

    // Until we know we're at a prompt again
    t.linefeed().unwrap();
    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    assert!(t.cursor_is_at_prompt());
}

#[test]
fn terminal_cursor_is_at_prompt_alternate_screen() {
    let mut t = init(3, 2);

    assert!(!t.cursor_is_at_prompt());
    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    assert!(t.cursor_is_at_prompt());

    // Secondary screen is never a prompt
    t.switch_screen_mode(SwitchScreenMode::M1049, true).unwrap();
    assert!(!t.cursor_is_at_prompt());
    t.semantic_prompt(&sp(A::PromptStart)).unwrap();
    assert!(!t.cursor_is_at_prompt());
}

#[test]
fn terminal_full_reset_with_a_non_empty_pen() {
    let mut t = init(80, 80);

    t.set_attribute(Attribute::DirectColorFg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0x7F,
    }))
    .unwrap();
    t.set_attribute(Attribute::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0x7F,
    }))
    .unwrap();
    t.screen_mut().cursor.semantic_content = SemanticContent::Input;
    t.full_reset();

    assert_eq!(0, at_cursor(&t).style_id());
    assert_eq!(0, t.screen().cursor.style_id);
    assert_eq!(SemanticContent::Output, t.screen().cursor.semantic_content);
}

#[test]
fn terminal_full_reset_hyperlink() {
    let mut t = init(80, 80);

    t.screen_mut()
        .start_hyperlink(b"http://example.com", None)
        .unwrap();
    t.full_reset();
    assert_eq!(0, t.screen().cursor.hyperlink_id);
}

#[test]
fn terminal_full_reset_with_a_non_empty_saved_cursor() {
    let mut t = init(80, 80);

    t.set_attribute(Attribute::DirectColorFg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0x7F,
    }))
    .unwrap();
    t.set_attribute(Attribute::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0x7F,
    }))
    .unwrap();
    t.save_cursor();
    t.full_reset();

    assert_eq!(0, at_cursor(&t).style_id());
    assert_eq!(0, t.screen().cursor.style_id);
}

#[test]
fn terminal_full_reset_origin_mode() {
    let mut t = init(10, 10);

    t.set_cursor_pos(3, 5);
    t.modes.set(Mode::Origin, true);
    t.full_reset();

    assert_eq!(0, t.screen().cursor.y);
    assert_eq!(0, t.screen().cursor.x);
    assert!(!t.modes.get(Mode::Origin));
}

#[test]
fn terminal_full_reset_status_display() {
    let mut t = init(10, 10);

    t.status_display = StatusDisplay::StatusLine;
    t.full_reset();
    assert!(t.status_display == StatusDisplay::Main);
}

// https://github.com/mitchellh/ghostty/issues/1607
#[test]
fn terminal_full_reset_clears_alt_screen_kitty_keyboard_state() {
    let mut t = init(10, 10);

    t.switch_screen_mode(SwitchScreenMode::M1049, true).unwrap();
    t.screen_mut()
        .kitty_keyboard
        .push(ghostty_vt::kitty::key::Flags {
            disambiguate: true,
            report_events: false,
            report_alternates: true,
            report_all: true,
            report_associated: true,
        });
    t.switch_screen_mode(SwitchScreenMode::M1049, false)
        .unwrap();

    t.full_reset();
    assert!(t.screens.get(ScreenKey::Alternate).is_none());
}

#[test]
fn terminal_full_reset_default_modes() {
    let mut o = Options::new(10, 10);
    let mut dm = ModePacked::default();
    dm.set(Mode::GraphemeCluster, true);
    o.default_modes = dm;
    let mut t = Terminal::new(o).unwrap();
    assert!(t.modes.get(Mode::GraphemeCluster));
    t.full_reset();
    assert!(t.modes.get(Mode::GraphemeCluster));
}

#[test]
fn terminal_full_reset_tracked_pins() {
    let mut t = init(80, 80);

    // Create a tracked pin
    let pin = t.screen().cursor_pin();
    let h = t.screen_mut().pages.track_pin(pin).unwrap();
    t.full_reset();
    let p = t.screen().pages.tracked_pin(h).unwrap();
    assert!(t.screen().pages.pin_is_valid(p));
}

// https://github.com/mitchellh/ghostty/issues/272
#[test]
fn terminal_resize_less_cols_with_wide_char_then_print() {
    let mut t = init(3, 3);

    t.print(u32::from('x')).unwrap();
    t.print(0x1F600).unwrap();
    t.resize(2, 3).unwrap();
    t.set_cursor_pos(1, 2);
    t.print(0x1F600).unwrap();
}

// https://github.com/mitchellh/ghostty/issues/723
#[test]
fn terminal_resize_with_left_and_right_margin_set() {
    let cols = 70;
    let rows = 23;
    let mut t = init(cols, rows);

    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.print(u32::from('0')).unwrap();
    t.modes.set(Mode::EnableMode3, true);
    t.resize(cols, rows).unwrap();
    t.set_left_and_right_margin(2, 0);
    t.print_repeat(1850).unwrap();
    let _ = t.modes.restore(Mode::EnableMode3);
    t.resize(cols, rows).unwrap();
}

// https://github.com/mitchellh/ghostty/issues/1343
#[test]
fn terminal_resize_with_wraparound_off() {
    let mut t = init(4, 2);

    t.modes.set(Mode::Wraparound, false);
    print_chars(&mut t, "0123");
    t.resize(2, 2).unwrap();

    assert_eq!("01", plain(&t));
}

#[test]
fn terminal_resize_with_wraparound_on() {
    let mut t = init(4, 2);

    t.modes.set(Mode::Wraparound, true);
    print_chars(&mut t, "0123");
    t.resize(2, 2).unwrap();

    assert_eq!("01\n23", plain(&t));
}

#[test]
fn terminal_resize_with_high_unique_style_per_cell() {
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
            t.print(u32::from('x')).unwrap();
        }
    }

    t.resize(60, 30).unwrap();
}

#[test]
fn terminal_resize_with_high_unique_style_per_cell_with_wrapping() {
    let mut t = init(30, 30);

    let cell_count: u16 = t.rows * t.cols;
    for i in 0..cell_count {
        let r = (i >> 8) as u8;
        let g = (i & 0xFF) as u8;

        t.set_attribute(Attribute::DirectColorBg(Rgb { r, g, b: 0 }))
            .unwrap();
        t.print(u32::from('x')).unwrap();
    }

    t.resize(60, 30).unwrap();
}

#[test]
fn terminal_resize_with_reflow_and_saved_cursor() {
    let mut t = init(2, 3);
    print_str(&mut t, "1A2B");
    t.set_cursor_pos(2, 2);
    assert_eq!(u32::from('B'), at_cursor(&t).codepoint());

    assert_eq!("1A\n2B", plain(&t));

    t.save_cursor();
    t.resize(5, 3).unwrap();
    t.restore_cursor();

    assert_eq!("1A2B", plain(&t));

    // Verify our cursor is still in the same place
    assert_eq!(u32::from('B'), at_cursor(&t).codepoint());
}

#[test]
fn terminal_resize_with_reflow_and_saved_cursor_pending_wrap() {
    let mut t = init(2, 3);
    print_str(&mut t, "1A2B");
    assert_eq!(u32::from('B'), at_cursor(&t).codepoint());

    assert_eq!("1A\n2B", plain(&t));

    t.save_cursor();
    t.resize(5, 3).unwrap();
    t.restore_cursor();

    assert_eq!("1A2B", plain(&t));

    // Pending wrap should be reset
    t.print(u32::from('X')).unwrap();
    assert_eq!("1A2BX", plain(&t));
}

#[test]
fn terminal_deccolm_without_dec_mode_40() {
    let mut t = init(5, 5);

    t.modes.set(Mode::M132Column, true);
    t.deccolm(DeccolmMode::Cols132).unwrap();
    assert_eq!(5, t.cols);
    assert_eq!(5, t.rows);
    assert!(!t.modes.get(Mode::M132Column));
}

#[test]
fn terminal_deccolm_unset() {
    let mut t = init(5, 5);

    t.modes.set(Mode::EnableMode3, true);
    t.deccolm(DeccolmMode::Cols80).unwrap();
    assert_eq!(80, t.cols);
    assert_eq!(5, t.rows);
}

#[test]
fn terminal_deccolm_resets_pending_wrap() {
    let mut t = init(5, 5);

    print_chars(&mut t, "ABCDE");
    assert!(t.screen().cursor.pending_wrap);

    t.modes.set(Mode::EnableMode3, true);
    t.deccolm(DeccolmMode::Cols80).unwrap();
    assert_eq!(80, t.cols);
    assert_eq!(5, t.rows);
    assert!(!t.screen().cursor.pending_wrap);
}

#[test]
fn terminal_deccolm_preserves_sgr_bg() {
    let mut t = init(5, 5);

    t.set_attribute(Attribute::DirectColorBg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0,
    }))
    .unwrap();
    t.modes.set(Mode::EnableMode3, true);
    t.deccolm(DeccolmMode::Cols80).unwrap();

    let cell = cell_at(&t, pt_active(0, 0));
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

#[test]
fn terminal_deccolm_resets_scroll_region() {
    let mut t = init(5, 5);

    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_top_and_bottom_margin(2, 3);
    t.set_left_and_right_margin(3, 5);

    t.modes.set(Mode::EnableMode3, true);
    t.deccolm(DeccolmMode::Cols80).unwrap();

    assert!(t.modes.get(Mode::EnableLeftAndRightMargin));
    assert_eq!(0, t.scrolling_region.top);
    assert_eq!(4, t.scrolling_region.bottom);
    assert_eq!(0, t.scrolling_region.left);
    assert_eq!(79, t.scrolling_region.right);
}

#[test]
fn terminal_mode_47_alt_screen_plain() {
    let mut t = init(5, 5);

    print_str(&mut t, "1A");

    t.switch_screen_mode(SwitchScreenMode::M47, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);

    assert_eq!("", plain(&t));

    // Off center because we copy the cursor over from the primary screen
    print_str(&mut t, "2B");
    assert_eq!("  2B", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M47, false).unwrap();
    assert_eq!(ScreenKey::Primary, t.screens.active_key);
    assert_eq!("1A", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M47, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);

    // Screen should retain content
    assert_eq!("  2B", plain(&t));
}

fn style_checks(t: &Terminal) {
    assert!(t.screen().cursor.style_id != style::DEFAULT_ID);
    let page = t.screen().cursor_page();
    assert_eq!(1, page.styles.count());
    assert!(page.styles.ref_count(t.screen().cursor.style_id) > 0);
}

fn copies_cursor_both_directions(mode: SwitchScreenMode) {
    let mut t = init(5, 5);

    // Color our cursor red
    t.set_attribute(Attribute::DirectColorFg(Rgb {
        r: 0xFF,
        g: 0,
        b: 0x7F,
    }))
    .unwrap();

    t.switch_screen_mode(mode, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);
    style_checks(&t);

    // Set a new style
    t.set_attribute(Attribute::DirectColorFg(Rgb {
        r: 0,
        g: 0xFF,
        b: 0,
    }))
    .unwrap();

    t.switch_screen_mode(mode, false).unwrap();
    assert_eq!(ScreenKey::Primary, t.screens.active_key);
    style_checks(&t);
}

#[test]
fn terminal_mode_47_copies_cursor_both_directions() {
    copies_cursor_both_directions(SwitchScreenMode::M47);
}

#[test]
fn terminal_mode_1047_alt_screen_plain() {
    let mut t = init(5, 5);

    print_str(&mut t, "1A");

    t.switch_screen_mode(SwitchScreenMode::M1047, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);
    assert_eq!("", plain(&t));

    print_str(&mut t, "2B");
    assert_eq!("  2B", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M1047, false)
        .unwrap();
    assert_eq!(ScreenKey::Primary, t.screens.active_key);
    assert_eq!("1A", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M1047, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);
    assert_eq!("", plain(&t));
}

#[test]
fn terminal_mode_1047_copies_cursor_both_directions() {
    copies_cursor_both_directions(SwitchScreenMode::M1047);
}

#[test]
fn terminal_mode_1049_alt_screen_plain() {
    let mut t = init(5, 5);

    print_str(&mut t, "1A");

    t.switch_screen_mode(SwitchScreenMode::M1049, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);
    assert_eq!("", plain(&t));

    print_str(&mut t, "2B");
    assert_eq!("  2B", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M1049, false)
        .unwrap();
    assert_eq!(ScreenKey::Primary, t.screens.active_key);
    assert_eq!("1A", plain(&t));

    // Write, our cursor should be restored back.
    print_str(&mut t, "C");
    assert_eq!("1AC", plain(&t));

    t.switch_screen_mode(SwitchScreenMode::M1049, true).unwrap();
    assert_eq!(ScreenKey::Alternate, t.screens.active_key);
    assert_eq!("", plain(&t));
}

// Reproduces a crash found by AFL++ fuzzer: orphaned spacer_tail when
// deleteLines clears all rows without shifting.
#[test]
fn terminal_delete_lines_wide_char_at_right_margin_with_full_clear() {
    let mut t = init(80, 24);

    t.set_cursor_pos(10, 39);
    t.print(0x4E2D).unwrap();

    t.modes.set(Mode::EnableLeftAndRightMargin, true);
    t.set_left_and_right_margin(5, 39);

    let rows = usize::from(t.rows);
    t.scroll_up(rows).unwrap();
}
