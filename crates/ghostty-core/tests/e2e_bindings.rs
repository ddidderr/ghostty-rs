//! Keybinding actions end to end (default and configured bindings).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;

mod common;
use common::*;
use ghostty_apprt::{
    action::{Action, GotoSplit, SplitDirection},
    structs::Clipboard,
};
use ghostty_input::Key;

#[test]
fn select_all_copy_paste_roundtrip() {
    let h = harness!(Harness::with_pre("printf 'hello world\\n'", &[]));
    assert!(h.bind("select_all"));
    assert!(h.surface.has_selection());
    assert!(h.bind("copy_to_clipboard"));
    let c = h.clipboard(Clipboard::Standard).expect("clipboard set");
    assert!(c.starts_with("hello world"), "{c:?}");
    // Mixed format also writes html
    assert!(
        h.rt.writes
            .borrow()
            .iter()
            .any(|w| w.contents.iter().any(|(m, _)| m == "text/html"))
    );
    // paste back: goes to the child
    h.set_clipboard_text(Clipboard::Standard, "pasted");
    assert!(h.bind("paste_from_clipboard"));
    h.expect_input(b"pasted");
}

#[test]
fn select_all_twice_with_copy_on_select_does_not_panic() {
    // Regression: replacing a selection compared against released pins.
    let h = harness!(Harness::with_pre(
        "printf 'hello\\n'",
        &["--copy-on-select=clipboard"]
    ));
    assert!(h.bind("select_all"));
    assert!(h.bind("select_all"));
    assert!(h.surface.has_selection());
    // copy-on-select=clipboard wrote both clipboards.
    assert_eq!(
        h.clipboard(Clipboard::Standard),
        h.clipboard(Clipboard::Selection)
    );
    assert!(
        h.clipboard(Clipboard::Standard)
            .unwrap()
            .starts_with("hello")
    );
}

#[test]
fn copy_without_selection_is_not_performed() {
    let h = harness!(Harness::recorder("", &[]));
    assert!(!h.bind("copy_to_clipboard"));
    assert!(h.clipboard(Clipboard::Standard).is_none());
}

#[test]
fn default_ctrl_shift_c_and_v() {
    let h = harness!(Harness::with_pre("printf 'copyme\\n'", &[]));
    h.bind("select_all");
    h.key(Key::KeyC, ctrl_shift(), "C");
    assert!(
        h.clipboard(Clipboard::Standard)
            .unwrap()
            .starts_with("copyme")
    );
    h.set_clipboard_text(Clipboard::Standard, "xyz");
    h.key(Key::KeyV, ctrl_shift(), "V");
    h.expect_input(b"xyz");
}

#[test]
fn bracketed_paste() {
    let h = harness!(Harness::recorder(r"\033[?2004h", &[]));
    h.wait("bracketed", |h| {
        h.surface
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::BracketedPaste)
    });
    h.set_clipboard_text(Clipboard::Standard, "line1\nline2");
    assert!(h.bind("paste_from_clipboard"));
    h.expect_input(b"\x1b[200~line1\nline2\x1b[201~");
}

#[test]
fn unbracketed_paste_converts_newlines_and_requires_confirmation_when_unsafe() {
    let h = harness!(Harness::recorder("", &[]));
    h.set_clipboard_text(Clipboard::Standard, "safe text");
    assert!(h.bind("paste_from_clipboard"));
    h.expect_input(b"safe text");
    h.mark();

    // A newline is unsafe without bracketed paste: confirmation required.
    h.set_clipboard_text(Clipboard::Standard, "rm -rf x\nls");
    assert!(h.bind("paste_from_clipboard"));
    h.wait("confirmation", |h| !h.rt.confirmations.borrow().is_empty());
    h.settle(100);
    assert!(h.recorded().is_empty(), "paste must wait for confirmation");
    // Confirm it.
    let c = h.rt.confirmations.borrow()[0].clone();
    h.surface
        .complete_clipboard_request(c.request, &c.text, true)
        .unwrap();
    h.expect_input(b"rm -rf x\rls");
}

#[test]
fn paste_without_text_on_clipboard_is_not_performed() {
    let h = harness!(Harness::recorder("", &[]));
    assert!(!h.bind("paste_from_clipboard"));
}

#[test]
fn paste_protection_can_be_disabled() {
    let h = harness!(Harness::recorder(
        "",
        &["--clipboard-paste-protection=false"]
    ));
    h.set_clipboard_text(Clipboard::Standard, "a\nb");
    assert!(h.bind("paste_from_clipboard"));
    h.expect_input(b"a\rb");
}

#[test]
fn font_size_actions() {
    let h = harness!(Harness::recorder("", &[]));
    let (c0, r0) = h.grid();
    let cell0 = h.surface.size().cell;
    h.clear_actions();

    assert!(h.bind("increase_font_size:4"));
    let cell1 = h.surface.size().cell;
    assert!(cell1.height > cell0.height && cell1.width > cell0.width);
    let (c1, r1) = h.grid();
    assert!(
        c1 < c0 && r1 < r0,
        "grid must shrink: {c0}x{r0} -> {c1}x{r1}"
    );
    assert!(h.has_action(|a| matches!(a, Action::CellSize(c) if c.width == cell1.width)));
    // The pty sees the new size eventually (TIOCSWINSZ via stty in other test).

    assert!(h.bind("decrease_font_size:8"));
    let cell2 = h.surface.size().cell;
    assert!(cell2.height < cell0.height);

    assert!(h.bind("reset_font_size"));
    assert_eq!(h.surface.size().cell, cell0);
    assert_eq!(h.grid(), (c0, r0));
}

#[test]
fn default_ctrl_equal_and_minus_change_font() {
    let h = harness!(Harness::recorder("", &[]));
    let cell0 = h.surface.size().cell;
    h.key(Key::Equal, ctrl(), "=");
    assert!(h.surface.size().cell.height > cell0.height);
    h.key(Key::Digit0, ctrl(), "0");
    assert_eq!(h.surface.size().cell, cell0);
    h.key(Key::Minus, ctrl(), "-");
    assert!(h.surface.size().cell.height < cell0.height);
    // none of these keys reach the child
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn tab_split_close_actions() {
    let h = harness!(Harness::recorder("", &[]));
    h.clear_actions();
    assert!(h.bind("new_tab"));
    assert!(h.has_action(|a| matches!(a, Action::NewTab)));
    assert!(h.bind("new_split:right"));
    assert!(h.has_action(|a| matches!(a, Action::NewSplit(SplitDirection::Right))));
    assert!(h.bind("new_split:down"));
    assert!(h.has_action(|a| matches!(a, Action::NewSplit(SplitDirection::Down))));
    assert!(h.bind("goto_split:left"));
    assert!(h.has_action(|a| matches!(a, Action::GotoSplit(GotoSplit::Left))));
    assert!(h.bind("goto_split:next"));
    assert!(h.has_action(|a| matches!(a, Action::GotoSplit(GotoSplit::Next))));
    assert!(h.bind("new_window"));
    assert!(h.has_action(|a| matches!(a, Action::NewWindow)));
    assert!(h.bind("toggle_split_zoom"));
    assert!(h.has_action(|a| matches!(a, Action::ToggleSplitZoom)));
    assert!(h.bind("equalize_splits"));
    assert!(h.has_action(|a| matches!(a, Action::EqualizeSplits)));
    assert!(h.bind("toggle_fullscreen"));
    assert!(h.has_action(|a| matches!(a, Action::ToggleFullscreen(_))));

    assert!(h.rt.closes.borrow().is_empty());
    assert!(h.bind("close_surface"));
    assert_eq!(h.rt.closes.borrow().len(), 1);
}

#[test]
fn default_keybinds_for_tabs_and_splits() {
    let h = harness!(Harness::recorder("", &[]));
    h.clear_actions();
    h.key(Key::KeyT, ctrl_shift(), "T");
    h.key(Key::KeyO, ctrl_shift(), "O");
    h.key(Key::KeyE, ctrl_shift(), "E");
    h.key(Key::KeyN, ctrl_shift(), "N");
    h.key(Key::KeyW, ctrl_shift(), "W");
    assert!(h.has_action(|a| matches!(a, Action::NewTab)));
    assert!(h.has_action(|a| matches!(a, Action::NewSplit(SplitDirection::Right))));
    assert!(h.has_action(|a| matches!(a, Action::NewSplit(SplitDirection::Down))));
    assert!(h.has_action(|a| matches!(a, Action::NewWindow)));
    assert!(h.has_action(|a| matches!(a, Action::CloseTab(_))));
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn scroll_page_up_down_and_clear_screen() {
    // Viewport scrolling goes through the IO thread, so wait for each change.
    let h = harness!(Harness::with_pre("seq 1 300", &[]));
    let before = h.screen();
    assert!(before.contains("300"), "{before}");
    assert!(h.bind("scroll_page_up"));
    h.wait("scrolled up", |h| h.screen() != before);
    assert!(!h.screen().contains("READY"));
    assert!(h.bind("scroll_page_down"));
    h.wait("scrolled down", |h| h.screen() == before);
    assert!(h.bind("scroll_to_top"));
    h.wait("top", |h| h.screen().starts_with("1\n"));
    assert!(h.bind("scroll_to_bottom"));
    h.wait("bottom", |h| h.screen() == before);

    // Keyboard: shift+page_up / shift+end are bound by default.
    h.key(Key::PageUp, shift(), "");
    h.wait("shift+pageup", |h| h.screen() != before);
    h.key(Key::End, shift(), "");
    h.wait("shift+end", |h| h.screen() == before);
    h.settle(30);
    assert_eq!(h.recorded(), Vec::<u8>::new());

    // clear_screen: viewport and history cleared.
    assert!(h.bind("clear_screen"));
    h.wait("screen cleared", |h| !h.screen().contains("300"));
    assert!(h.bind("scroll_to_top"));
    h.settle(50);
    assert!(
        !h.screen().contains("150"),
        "history cleared: {}",
        h.screen()
    );
}

#[test]
fn jump_to_prompt_with_osc133() {
    let mut pre = String::new();
    for i in 1..=5 {
        let _ = write!(pre, "printf '\\033]133;A\\033\\\\prompt{i}\\n'; seq 1 40; ");
    }
    pre.push_str("true");
    let h = harness!(Harness::with_pre(&pre, &[]));
    let bottom = h.screen();
    assert!(bottom.contains("40"));
    assert!(h.bind("jump_to_prompt:-1"));
    h.wait("viewport moved to a prompt", |h| h.screen() != bottom);
    let s = h.screen();
    assert!(s.contains("prompt"), "{s}");
    let first = s.lines().next().unwrap().to_owned();
    assert!(first.starts_with("prompt"), "prompt at the top row: {s}");
    assert!(h.bind("jump_to_prompt:-1"));
    h.wait("moved again", |h| {
        h.screen().lines().next() != Some(first.as_str())
    });
    let second = h.screen().lines().next().unwrap().to_owned();
    assert!(second.starts_with("prompt") && second != first);
    assert!(h.bind("jump_to_prompt:1"));
    h.wait("moved forward", |h| {
        h.screen().lines().next() == Some(first.as_str())
    });
}

#[test]
fn key_sequence_binding() {
    let h = harness!(Harness::recorder("", &["--keybind=ctrl+a>n=new_tab"]));
    h.clear_actions();
    let e = h.key(Key::KeyA, ctrl(), "a");
    assert_eq!(e, ghostty_apprt::runtime_api::InputEffect::Consumed);
    assert!(h.has_action(|a| matches!(a, Action::KeySequence(_))));
    assert!(!h.has_action(|a| matches!(a, Action::NewTab)));
    h.key(Key::KeyN, none(), "n");
    assert!(h.has_action(|a| matches!(a, Action::NewTab)));
    assert!(h.has_action(|a| matches!(
        a,
        Action::KeySequence(ghostty_apprt::action::KeySequence::End)
    )));
    h.settle(50);
    assert!(
        h.recorded().is_empty(),
        "sequence keys are not sent to the app"
    );
}

#[test]
fn key_sequence_mismatch_flushes_keys_to_child() {
    let h = harness!(Harness::recorder("", &["--keybind=ctrl+a>n=new_tab"]));
    h.key(Key::KeyA, ctrl(), "a");
    h.key(Key::KeyX, none(), "x");
    h.expect_input(b"\x01x");
    assert!(!h.has_action(|a| matches!(a, Action::NewTab)));
}

#[test]
fn unconsumed_binding_also_sends_key() {
    let h = harness!(Harness::recorder(
        "",
        &[
            "--keybind=unconsumed:ctrl+b=new_tab",
            "--keybind=ctrl+k=new_split:right"
        ]
    ));
    h.clear_actions();
    h.key(Key::KeyB, ctrl(), "b");
    h.key(Key::KeyK, ctrl(), "k");
    assert!(h.has_action(|a| matches!(a, Action::NewTab)));
    assert!(h.has_action(|a| matches!(a, Action::NewSplit(_))));
    h.expect_input(b"\x02"); // only the unconsumed one reaches the child
}

#[test]
fn performable_binding_passes_through_when_not_performed() {
    let h = harness!(Harness::with_pre(
        "printf 'sel\\n'",
        &["--keybind=performable:ctrl+x=copy_to_clipboard"]
    ));
    // no selection: copy cannot be performed so ctrl+x goes to the child
    h.key(Key::KeyX, ctrl(), "x");
    h.expect_input(b"\x18");
    h.mark();
    h.bind("select_all");
    h.key(Key::KeyX, ctrl(), "x");
    assert!(h.clipboard(Clipboard::Standard).unwrap().starts_with("sel"));
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn text_csi_esc_and_ignore_bindings() {
    let h = harness!(Harness::recorder(
        "",
        &[
            "--keybind=ctrl+1=text:hi\\x21",
            "--keybind=ctrl+2=csi:1;5H",
            "--keybind=ctrl+3=esc:x",
            "--keybind=ctrl+4=ignore",
        ]
    ));
    h.key(Key::Digit1, ctrl(), "1");
    h.key(Key::Digit2, ctrl(), "2");
    h.key(Key::Digit3, ctrl(), "3");
    h.key(Key::Digit4, ctrl(), "4");
    h.expect_input(b"hi!\x1b[1;5H\x1bx");
}

#[test]
fn csi_binding_longer_than_the_zig_buffer_is_rejected() {
    let h = harness!(Harness::recorder("", &[]));
    let long = "1".repeat(127);
    let a = ghostty_input::binding::Action::parse(&format!("csi:{long}")).unwrap();
    assert!(h.surface.perform_binding_action(&a).is_err());
    let ok = "1".repeat(126);
    let a = ghostty_input::binding::Action::parse(&format!("csi:{ok}")).unwrap();
    assert!(h.surface.perform_binding_action(&a).is_ok());
    let a = ghostty_input::binding::Action::parse(&format!("esc:{}", "2".repeat(128))).unwrap();
    assert!(h.surface.perform_binding_action(&a).is_err());
    let a = ghostty_input::binding::Action::parse(&format!("esc:{}", "2".repeat(127))).unwrap();
    assert!(h.surface.perform_binding_action(&a).is_ok());
}

#[test]
fn global_new_window_binding_has_no_parent() {
    let h = harness!(Harness::recorder(
        "",
        &["--keybind=global:ctrl+n=new_window"]
    ));
    h.clear_actions();
    h.key(Key::KeyN, ctrl(), "n");
    let found =
        h.rt.actions.borrow().iter().any(|(t, a)| {
            matches!(a, Action::NewWindow) && *t == ghostty_apprt::action::Target::App
        });
    assert!(found, "{:?}", h.action_names());
}

#[test]
fn child_environment_comes_from_the_runtime() {
    // FakeRt removes HOME like GTK removes GDK_DISABLE & co; the core must not
    // restore it from the process environment.
    let h = harness!(Harness::new(Opts::default().script(
        "env | grep -c '^HOME=' >'{out}'; env | grep -c '^TERM_PROGRAM_TEST=1' >>'{out}'; sleep 5"
    )));
    h.wait("env output", |h| h.recorded().len() >= 4);
    assert_eq!(String::from_utf8_lossy(&h.recorded()), "0\n1\n");
}

#[test]
fn write_screen_file_open_failure_is_an_error() {
    // The fake apprt declines open_url and xdg-open may not exist; when the
    // fallback fails the binding must report failure (so performable
    // bindings fall through).
    let h = harness!(Harness::with_pre("printf 'x\\n'", &[]));
    let a = ghostty_input::binding::Action::parse("write_screen_file:open").unwrap();
    let res = h.surface.perform_binding_action(&a);
    let xdg = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v xdg-open")
        .output()
        .is_ok_and(|o| o.status.success());
    if !xdg {
        assert!(res.is_err(), "{res:?}");
    }
}
