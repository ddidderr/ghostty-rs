//! Keyboard input end to end: key events -> core Surface -> encoder -> pty ->
//! a real `cat` child that records the exact bytes it received.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use ghostty_input::Key;

fn rec_args(setup: &str, args: &[&str]) -> Option<Harness> {
    Harness::recorder(setup, args)
}

fn rec(setup: &str) -> Option<Harness> {
    Harness::recorder(setup, &[])
}

#[test]
fn printable_text_enter_backspace_tab() {
    let h = harness!(rec(""));
    h.type_text("Hello, World! 123");
    h.key(Key::Enter, none(), "");
    h.key(Key::Backspace, none(), "");
    h.key(Key::Tab, none(), "");
    h.key(Key::Tab, shift(), "");
    h.key(Key::Escape, none(), "");
    h.key(Key::Space, none(), " ");
    h.expect_input(b"Hello, World! 123\r\x7f\t\x1b[Z\x1b ");
}

#[test]
fn utf8_text_is_passed_through() {
    let h = harness!(rec(""));
    h.key(Key::KeyE, none(), "\u{e9}");
    h.key(Key::Unidentified, none(), "\u{1f600}");
    h.expect_input("\u{e9}\u{1f600}".as_bytes());
}

#[test]
fn ctrl_keys() {
    let h = harness!(rec(""));
    h.key(Key::KeyC, ctrl(), "c");
    h.key(Key::KeyD, ctrl(), "d");
    h.key(Key::KeyA, ctrl(), "a");
    h.key(Key::KeyZ, ctrl(), "z");
    // ctrl+[ is deliberately not in Ghostty's ctrl table (kitty behaviour): CSI u.
    h.key(Key::BracketLeft, ctrl(), "[");
    h.key(Key::Space, ctrl(), " ");
    h.expect_input(b"\x03\x04\x01\x1a\x1b[91;5u\x00");
}

#[test]
fn alt_prefix() {
    let h = harness!(rec(""));
    h.key(Key::KeyX, alt(), "x");
    h.key(Key::KeyB, alt(), "b");
    h.key(Key::Enter, alt(), "");
    h.key(Key::Backspace, alt(), "");
    h.expect_input(b"\x1bx\x1bb\x1b\r\x1b\x7f");
}

#[test]
fn ctrl_alt() {
    let h = harness!(rec(""));
    h.key(
        Key::KeyC,
        ghostty_input::Mods {
            ctrl: true,
            alt: true,
            ..Default::default()
        },
        "c",
    );
    h.expect_input(b"\x1b\x03");
}

#[test]
fn arrows_normal_mode() {
    let h = harness!(rec(""));
    h.special(Key::ArrowUp, none());
    h.special(Key::ArrowDown, none());
    h.special(Key::ArrowRight, none());
    h.special(Key::ArrowLeft, none());
    h.special(Key::Home, none());
    h.special(Key::End, none());
    h.special(Key::PageUp, none());
    h.special(Key::PageDown, none());
    h.special(Key::Insert, none());
    h.special(Key::Delete, none());
    h.expect_input(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1b[5~\x1b[6~\x1b[2~\x1b[3~");
}

#[test]
fn arrows_decckm_application_mode() {
    let h = harness!(rec(r"\033[?1h"));
    h.special(Key::ArrowUp, none());
    h.special(Key::ArrowDown, none());
    h.special(Key::ArrowRight, none());
    h.special(Key::ArrowLeft, none());
    h.special(Key::Home, none());
    h.special(Key::End, none());
    // Modified arrows always use CSI form.
    h.special(Key::ArrowUp, shift());
    h.special(Key::ArrowLeft, ctrl());
    h.expect_input(b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF\x1b[1;2A\x1b[1;5D");
}

#[test]
fn function_keys() {
    let h = harness!(rec(""));
    for k in [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ] {
        h.special(k, none());
    }
    h.special(Key::F1, shift());
    h.special(Key::F5, ctrl());
    h.expect_input(
        b"\x1bOP\x1bOQ\x1bOR\x1bOS\x1b[15~\x1b[17~\x1b[18~\x1b[19~\x1b[20~\x1b[21~\x1b[23~\x1b[24~\x1b[1;2P\x1b[15;5~",
    );
}

#[test]
fn keypad_normal_and_application() {
    // numlock on, normal keypad mode: plain digits
    let h = harness!(rec(""));
    h.key(Key::Numpad1, none(), "1");
    h.key(Key::NumpadAdd, none(), "+");
    h.key(Key::NumpadEnter, none(), "");
    h.expect_input(b"1+\r");

    // DECKPAM alone is ignored (mode 1035 ignore_keypad_with_numlock defaults on).
    let h = harness!(rec(r"\033="));
    h.key(Key::Numpad1, none(), "1");
    h.expect_input(b"1");

    // DECKPAM + mode 1035 off: application keypad sequences
    let h = harness!(rec(r"\033=\033[?1035l"));
    h.key(Key::Numpad1, none(), "1");
    h.key(Key::NumpadAdd, none(), "+");
    h.key(Key::NumpadEnter, none(), "");
    h.expect_input(b"\x1bOq\x1bOk\x1bOM");
}

#[test]
fn kitty_keyboard_disambiguate() {
    let h = harness!(rec(r"\033[>1u"));
    h.wait("kitty flags", |h| {
        h.surface
            .lock_terminal()
            .screen()
            .kitty_keyboard
            .current()
            .int()
            == 1
    });
    h.key(Key::KeyC, ctrl(), "c");
    h.key(Key::Escape, none(), "");
    h.key(Key::KeyX, alt(), "x");
    h.key(Key::KeyA, none(), "a");
    h.key(Key::Enter, none(), "");
    h.key(Key::Enter, shift(), "");
    h.special(Key::ArrowUp, none());
    h.special(Key::F1, none());
    h.expect_input(b"\x1b[99;5u\x1b[27u\x1b[120;3ua\r\x1b[13;2u\x1b[A\x1b[P");
}

#[test]
fn kitty_keyboard_report_events_sends_release() {
    // flags 1|2: disambiguate + report event types
    let h = harness!(rec(r"\033[>3u"));
    h.wait("kitty flags", |h| {
        h.surface
            .lock_terminal()
            .screen()
            .kitty_keyboard
            .current()
            .int()
            == 3
    });
    h.key(Key::KeyA, none(), "a");
    h.key(Key::KeyC, ctrl(), "c");
    // press "a" is plain text, release "a" is reported
    h.expect_input(b"a\x1b[97;1:3u\x1b[99;5u\x1b[99;5:3u");
}

#[test]
fn kitty_keyboard_pop_restores_legacy() {
    let h = harness!(rec(r"\033[>1u\033[<1u"));
    h.wait("kitty popped", |h| {
        h.surface
            .lock_terminal()
            .screen()
            .kitty_keyboard
            .current()
            .int()
            == 0
    });
    h.key(Key::KeyC, ctrl(), "c");
    h.expect_input(b"\x03");
}

#[test]
fn modify_other_keys_state_2() {
    let h = harness!(rec(r"\033[>4;2m"));
    h.wait("mok2", |h| {
        h.surface.lock_terminal().flags.modify_other_keys_2
    });
    // Plain ctrl+letter stays legacy, other ctrl combos use CSI 27.
    h.key(Key::KeyA, ctrl(), "a");
    h.key(Key::Semicolon, ctrl(), ";");
    h.key(Key::Digit1, ctrl(), "1");
    h.key(Key::KeyA, none(), "a");
    h.expect_input(b"\x01\x1b[27;5;59~1a");
}

#[test]
fn no_input_while_disable_keyboard_mode() {
    // KAM (CSI 2 h) disables keyboard input.
    let h = harness!(rec_args(r"\033[2h", &["--vt-kam-allowed=true"]));
    h.wait("kam", |h| {
        h.surface
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::DisableKeyboard)
    });
    h.type_text("zz");
    h.settle(100);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn key_release_without_press_is_ignored_in_legacy() {
    let h = harness!(rec(""));
    // Only a release event: nothing should be sent.
    h.key_event(ghostty_input::KeyEvent {
        action: ghostty_input::key::Action::Release,
        key: Key::KeyA,
        utf8: "a",
        ..Default::default()
    });
    h.type_text("b");
    h.expect_input(b"b");
}
