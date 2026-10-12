//! Mouse end to end: selection, copy-on-select, multi-click, mouse reporting
//! formats, alternate scroll, shift bypass and links.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use ghostty_apprt::action::Action;
use ghostty_apprt::structs::Clipboard;
use ghostty_input::{Key, MouseButton};

const L: MouseButton = MouseButton::Left;

/// Drag-select from the left edge of `c0` to the right edge of `c1` on `row`.
fn drag(h: &Harness, c0: u32, c1: u32, row: u32, mods: ghostty_input::Mods) {
    h.move_to_pos(h.cell_pos_frac(c0, row, 0.1, 0.5), Some(mods));
    h.press_mouse(L, mods);
    // a couple of intermediate moves like a real drag
    for c in c0..=c1 {
        h.move_to_pos(h.cell_pos_frac(c, row, 0.9, 0.5), Some(mods));
    }
    h.release_mouse(L, mods);
}

fn text_harness(args: &[&str]) -> Option<Harness> {
    Harness::with_pre("printf 'hello world foo\\nsecond line\\n'", args)
}

#[test]
fn drag_selects_and_copy_on_select_fills_selection_clipboard() {
    let h = harness!(text_harness(&[]));
    drag(&h, 0, 4, 0, none());
    assert_eq!(h.surface.selection_string().as_deref(), Some("hello"));
    // copy-on-select = true: the selection clipboard (not the standard one)
    assert_eq!(h.clipboard(Clipboard::Selection).as_deref(), Some("hello"));
    assert_eq!(h.clipboard(Clipboard::Standard), None);

    assert!(h.bind("copy_to_clipboard"));
    assert_eq!(h.clipboard(Clipboard::Standard).as_deref(), Some("hello"));
}

#[test]
fn copy_on_select_clipboard_sets_both_and_false_sets_none() {
    let h = harness!(text_harness(&["--copy-on-select=clipboard"]));
    drag(&h, 6, 10, 0, none());
    assert_eq!(h.clipboard(Clipboard::Standard).as_deref(), Some("world"));
    assert_eq!(h.clipboard(Clipboard::Selection).as_deref(), Some("world"));

    let h = harness!(text_harness(&["--copy-on-select=false"]));
    drag(&h, 6, 10, 0, none());
    assert_eq!(h.surface.selection_string().as_deref(), Some("world"));
    assert_eq!(h.clipboard(Clipboard::Selection), None);
    assert_eq!(h.clipboard(Clipboard::Standard), None);
}

#[test]
fn click_clears_selection() {
    let h = harness!(text_harness(&[]));
    drag(&h, 0, 4, 0, none());
    assert!(h.surface.has_selection());
    h.click(2, 1, L, none());
    assert!(!h.surface.has_selection());
}

#[test]
fn double_click_selects_word_and_triple_click_line() {
    let h = harness!(text_harness(&[]));
    h.move_to(7, 0, None);
    h.press_mouse(L, none());
    h.release_mouse(L, none());
    h.press_mouse(L, none());
    h.release_mouse(L, none());
    assert_eq!(h.surface.selection_string().as_deref(), Some("world"));
    h.press_mouse(L, none());
    h.release_mouse(L, none());
    assert_eq!(
        h.surface.selection_string().as_deref(),
        Some("hello world foo")
    );
}

#[test]
fn reporting_normal_sgr() {
    // 1000 normal + 1006 SGR
    let h = harness!(Harness::recorder(r"\033[?1000h\033[?1006h", &[]));
    h.wait("mouse mode", |h| {
        h.surface.lock_terminal().flags.mouse_event != ghostty_terminal::terminal::MouseEvents::None
    });
    h.move_to(5, 3, None); // motion is not reported in normal mode
    h.press_mouse(L, none());
    h.release_mouse(L, none());
    h.press_mouse(MouseButton::Right, ctrl());
    h.release_mouse(MouseButton::Right, ctrl());
    h.expect_input(b"\x1b[<0;6;4M\x1b[<0;6;4m\x1b[<18;6;4M\x1b[<18;6;4m");
    h.mark();
    // wheel: buttons 4/5
    h.move_to(5, 3, Some(none())); // ctrl is released
    h.scroll(0.0, 1.0);
    h.scroll(0.0, -1.0);
    // One discrete tick is `mouse-scroll-multiplier` (3) rows -> 3 reports per tick.
    h.wait("wheel", |h| h.recorded().len() >= 6 * 9);
    let r = String::from_utf8(h.recorded()).unwrap();
    assert!(r.starts_with("\x1b[<64;6;4M"), "{r:?}");
    assert!(r.contains("\x1b[<65;6;4M"), "{r:?}");
}

#[test]
fn reporting_x10_mode_only_presses() {
    let h = harness!(Harness::recorder(r"\033[?9h", &[]));
    h.wait("x10", |h| {
        h.surface.lock_terminal().flags.mouse_event == ghostty_terminal::terminal::MouseEvents::X10
    });
    h.click(5, 3, L, none());
    // ESC [ M, 32+0, 32+6, 32+4
    h.expect_input(b"\x1b[M &$");
}

#[test]
fn reporting_normal_default_format() {
    let h = harness!(Harness::recorder(r"\033[?1000h", &[]));
    h.wait("mode", |h| {
        h.surface.lock_terminal().flags.mouse_event
            == ghostty_terminal::terminal::MouseEvents::Normal
    });
    h.click(5, 3, L, shift_free());
    // press: button 0, release: button 3 in non-SGR formats
    h.expect_input(b"\x1b[M &$\x1b[M#&$");
}

fn shift_free() -> ghostty_input::Mods {
    none()
}

#[test]
fn reporting_button_and_any_motion() {
    // 1002 button-event tracking + SGR
    let h = harness!(Harness::recorder(r"\033[?1002h\033[?1006h", &[]));
    h.wait("mode", |h| {
        h.surface.lock_terminal().flags.mouse_event
            == ghostty_terminal::terminal::MouseEvents::Button
    });
    h.move_to(1, 1, None); // no button: not reported
    h.press_mouse(L, none());
    h.move_to(2, 1, Some(none()));
    h.move_to(2, 1, Some(none())); // same cell: deduplicated
    h.move_to(3, 1, Some(none()));
    h.release_mouse(L, none());
    h.expect_input(b"\x1b[<0;2;2M\x1b[<32;3;2M\x1b[<32;4;2M\x1b[<0;4;2m");

    // 1003 any-event tracking reports motion without buttons (code 35)
    let h = harness!(Harness::recorder(r"\033[?1003h\033[?1006h", &[]));
    h.wait("mode", |h| {
        h.surface.lock_terminal().flags.mouse_event == ghostty_terminal::terminal::MouseEvents::Any
    });
    h.move_to(1, 1, Some(none()));
    h.move_to(2, 1, Some(none()));
    h.expect_input(b"\x1b[<35;2;2M\x1b[<35;3;2M");
}

#[test]
fn reporting_urxvt_and_utf8_formats() {
    let h = harness!(Harness::recorder(r"\033[?1000h\033[?1015h", &[]));
    h.wait("urxvt", |h| {
        h.surface.lock_terminal().flags.mouse_format
            == ghostty_terminal::terminal::MouseFormat::Urxvt
    });
    h.click(5, 3, L, none());
    h.expect_input(b"\x1b[32;6;4M\x1b[35;6;4M");

    // UTF-8 format can encode columns > 95 as 2-byte sequences.
    let h = harness!(Harness::new(
        Opts {
            size: Some((1600, 600)),
            ..Opts::default()
        }
        .script(
            "stty raw -echo; printf '\\033[?1000h\\033[?1005h'; printf READY; exec cat >'{out}'"
        )
    ));
    h.wait_screen("READY");
    h.wait("utf8", |h| {
        h.surface.lock_terminal().flags.mouse_format
            == ghostty_terminal::terminal::MouseFormat::Utf8
    });
    assert!(h.grid().0 > 130, "grid {:?}", h.grid());
    h.move_to(120, 2, None);
    h.press_mouse(L, none());
    let mut want = b"\x1b[M ".to_vec();
    want.extend("\u{20}".chars().map(|_| 0u8).take(0));
    let col = char::from_u32(32 + 120 + 1).unwrap();
    want.extend(col.to_string().as_bytes());
    want.push(32 + 2 + 1);
    h.expect_input(&want);
}

#[test]
fn reporting_sgr_pixels() {
    let h = harness!(Harness::recorder(r"\033[?1000h\033[?1016h", &[]));
    h.wait("sgr-pixels", |h| {
        h.surface.lock_terminal().flags.mouse_format
            == ghostty_terminal::terminal::MouseFormat::SgrPixels
    });
    let p = h.cell_pos_frac(5, 3, 0.5, 0.5);
    h.move_to_pos(p, None);
    h.press_mouse(L, none());
    // terminal-space coordinates are surface minus padding, rounded
    let pad = h.surface.size().padding;
    let x = (p.x - pad.left as f32).round() as i32;
    let y = (p.y - pad.top as f32).round() as i32;
    h.expect_input(format!("\x1b[<0;{x};{y}M").as_bytes());
}

#[test]
fn shift_bypasses_mouse_reporting_and_selects() {
    let h = harness!(Harness::with_pre(
        "printf 'hello world foo\\n'; printf '\\033[?1000h\\033[?1006h'",
        &[]
    ));
    h.wait("mode", |h| {
        h.surface.lock_terminal().flags.mouse_event != ghostty_terminal::terminal::MouseEvents::None
    });
    // Without shift the app gets the click and nothing is selected.
    drag(&h, 0, 4, 0, none());
    assert!(!h.surface.has_selection());
    h.wait("reports", |h| !h.recorded().is_empty());
    h.mark();
    // With shift the click is ours.
    drag(&h, 0, 4, 0, shift());
    assert_eq!(h.surface.selection_string().as_deref(), Some("hello"));
    h.settle(50);
    assert!(h.recorded().is_empty(), "{:?}", h.recorded());
}

#[test]
fn mouse_reporting_can_be_disabled_by_config() {
    let h = harness!(Harness::recorder(
        r"\033[?1000h\033[?1006h",
        &["--mouse-reporting=false"]
    ));
    h.click(5, 3, L, none());
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn alternate_scroll_sends_cursor_keys() {
    let h = harness!(Harness::recorder(r"\033[?1049h", &[]));
    h.wait("alt screen", |h| {
        h.surface.lock_terminal().screens.active_key == ghostty_terminal::screen_set::Key::Alternate
    });
    h.scroll(0.0, 1.0);
    let n_up = {
        h.wait("up", |h| !h.recorded().is_empty());
        h.settle(50);
        h.recorded()
    };
    assert!(n_up.len() >= 3 && n_up.len() % 3 == 0, "{n_up:?}");
    assert!(n_up.chunks(3).all(|c| c == b"\x1b[A"), "{n_up:?}");
    h.mark();
    h.scroll(0.0, -1.0);
    h.wait("down", |h| !h.recorded().is_empty());
    h.settle(50);
    let down = h.recorded();
    assert!(down.chunks(3).all(|c| c == b"\x1b[B"), "{down:?}");
    assert_eq!(down.len(), n_up.len());
}

#[test]
fn alternate_scroll_application_cursor_keys() {
    let h = harness!(Harness::recorder(r"\033[?1049h\033[?1h", &[]));
    h.wait("alt screen", |h| {
        h.surface.lock_terminal().screens.active_key == ghostty_terminal::screen_set::Key::Alternate
    });
    h.scroll(0.0, -1.0);
    h.wait("down", |h| !h.recorded().is_empty());
    h.settle(50);
    assert!(h.recorded().chunks(3).all(|c| c == b"\x1bOB"));
}

#[test]
fn primary_screen_wheel_scrolls_viewport_not_app() {
    let h = harness!(Harness::with_pre("seq 1 300", &[]));
    let before = h.screen();
    h.scroll(0.0, 3.0);
    h.wait("viewport scrolled", |h| h.screen() != before);
    h.settle(30);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

fn osc8_child() -> Option<Harness> {
    Harness::with_pre(
        "printf 'go \\033]8;;http://osc8.example/x\\033\\\\here\\033]8;;\\033\\\\ and https://ghostty.org/docs/config ok\\n'",
        &[],
    )
}

#[test]
fn osc8_link_hover_and_ctrl_click_opens() {
    let h = harness!(osc8_child());
    // hover with ctrl over "here" (cols 3..6)
    h.move_to(4, 0, Some(ctrl()));
    let url = h.wait_action("mouse over link", |a| match a {
        Action::MouseOverLink(l) if !l.url.is_empty() => Some(l.url.clone()),
        _ => None,
    });
    assert_eq!(url, "http://osc8.example/x");
    h.click(4, 0, L, ctrl());
    let opened = h.wait_action("open url", |a| match a {
        Action::OpenUrl(u) => Some(u.url.clone()),
        _ => None,
    });
    assert_eq!(opened, "http://osc8.example/x");
}

#[test]
fn regex_url_ctrl_click_opens_and_plain_click_does_not() {
    let h = harness!(osc8_child());
    // "go here and https://ghostty.org/docs/config ok": URL starts at col 12
    h.click(20, 0, L, none());
    h.settle(50);
    assert!(!h.has_action(|a| matches!(a, Action::OpenUrl(_))));
    h.move_to(30, 2, Some(none()));
    h.move_to(20, 0, Some(ctrl()));
    h.wait_action("hover", |a| match a {
        Action::MouseOverLink(l) if l.url.contains("ghostty.org") => Some(()),
        _ => None,
    });
    h.click(20, 0, L, ctrl());
    let opened = h.wait_action("open url", |a| match a {
        Action::OpenUrl(u) => Some(u.url.clone()),
        _ => None,
    });
    assert_eq!(opened, "https://ghostty.org/docs/config");
    // moving away clears the hover
    h.clear_actions();
    h.move_to(2, 5, Some(none()));
    h.wait_action("hover cleared", |a| match a {
        Action::MouseOverLink(l) if l.url.is_empty() => Some(()),
        _ => None,
    });
}

#[test]
fn middle_click_pastes_selection_clipboard() {
    let h = harness!(Harness::recorder("", &[]));
    h.set_clipboard_text(Clipboard::Selection, "sel-text");
    h.click(3, 3, MouseButton::Middle, none());
    h.expect_input(b"sel-text");
}

#[test]
fn right_click_shows_context_menu_action_or_nothing_to_child() {
    let h = harness!(Harness::recorder("", &[]));
    h.click(3, 3, MouseButton::Right, none());
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
    let _ = Key::Enter;
}
