//! Terminal <-> runtime events: resize, focus, title, pwd, bell, OSC 52,
//! notifications, progress, child exit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use ghostty_apprt::{
    action::Action,
    structs::{Clipboard, SurfaceSize},
};
use ghostty_input::Key;
use ghostty_vt::osc::ProgressState;

#[test]
fn resize_reaches_child_via_tiocswinsz() {
    // The child prints `stty size` on every SIGWINCH.
    let h = harness!(Harness::new(Opts::default().script(
        "stty size >'{out}'; trap 'stty size >>\"{out}\"' WINCH; printf READY; while :; do sleep 0.02; done"
    )));
    h.wait_screen("READY");
    let (c0, r0) = h.grid();
    h.wait("initial size", |h| {
        String::from_utf8_lossy(&h.recorded()).starts_with(&format!("{r0} {c0}\n"))
    });

    h.surface
        .size_callback(SurfaceSize {
            width: 1000,
            height: 500,
        })
        .unwrap();
    let (c1, r1) = h.grid();
    assert_ne!((c0, r0), (c1, r1));
    h.wait("new size seen by child", |h| {
        String::from_utf8_lossy(&h.recorded()).contains(&format!("{r1} {c1}\n"))
    });
    // The terminal itself was resized too.
    let t = h.surface.lock_terminal();
    assert_eq!((t.cols, t.rows), (c1, r1));
}

#[test]
fn font_size_change_resizes_pty() {
    let h = harness!(Harness::new(Opts::default().script(
        "trap 'stty size >>\"{out}\"' WINCH; printf READY; while :; do sleep 0.02; done"
    )));
    h.wait_screen("READY");
    h.bind("increase_font_size:6");
    let (c, r) = h.grid();
    h.wait("child saw new grid", |h| {
        String::from_utf8_lossy(&h.recorded()).contains(&format!("{r} {c}\n"))
    });
}

#[test]
fn focus_events_mode_1004() {
    let h = harness!(Harness::recorder(r"\033[?1004h", &[]));
    h.wait("mode 1004", |h| {
        h.surface
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::FocusEvent)
    });
    h.surface.focus_callback(false).unwrap();
    h.surface.focus_callback(true).unwrap();
    // (enabling mode 1004 reports the current focus state first)
    h.expect_input(b"\x1b[I\x1b[O\x1b[I");
}

#[test]
fn no_focus_events_without_mode() {
    let h = harness!(Harness::recorder("", &[]));
    h.surface.focus_callback(false).unwrap();
    h.surface.focus_callback(true).unwrap();
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn title_osc0_osc2_set_title_action() {
    let h = harness!(Harness::recorder(
        r"\033]0;first title\007\033]2;second\033\\",
        &[]
    ));
    h.wait("second title", |h| {
        h.has_action(|a| matches!(a, Action::SetTitle(t) if t.title == "second"))
    });
    assert!(h.has_action(|a| matches!(a, Action::SetTitle(t) if t.title == "first title")));
}

#[test]
fn title_from_config_ignores_osc() {
    let h = harness!(Harness::recorder(r"\033]2;osc\007", &["--title=fixed"]));
    h.settle(100);
    assert!(h.has_action(|a| matches!(a, Action::SetTitle(t) if t.title == "fixed")));
    assert!(!h.has_action(|a| matches!(a, Action::SetTitle(t) if t.title == "osc")));
}

#[test]
fn pwd_osc7() {
    let h = harness!(Harness::recorder(
        r"\033]7;file://localhost/tmp/a%%20b\033\\",
        &[]
    ));
    let pwd = h.wait_action("pwd", |a| match a {
        Action::Pwd(p) if !p.pwd.is_empty() => Some(p.pwd.clone()),
        _ => None,
    });
    assert_eq!(pwd, "/tmp/a b");
    assert_eq!(h.surface.pwd().as_deref(), Some("/tmp/a b"));
}

#[test]
fn bell_rings_once_per_burst() {
    let h = harness!(Harness::recorder(r"\007", &[]));
    h.wait_action("bell", |a| matches!(a, Action::RingBell).then_some(()));
    // Rate limited to one per 100ms: a burst of bells is collapsed.
    let h = harness!(Harness::recorder(r"\007\007\007\007", &[]));
    h.wait_action("bell", |a| matches!(a, Action::RingBell).then_some(()));
    h.settle(60);
    assert!(h.count_actions(|a| matches!(a, Action::RingBell)) <= 2);
}

#[test]
fn osc52_write_allowed_by_default() {
    let h = harness!(Harness::recorder(r"\033]52;c;aGVsbG8=\007", &[]));
    h.wait("clipboard written", |h| {
        h.clipboard(Clipboard::Standard).is_some()
    });
    assert_eq!(h.clipboard(Clipboard::Standard).as_deref(), Some("hello"));
    assert!(!h.rt.writes.borrow()[0].confirm);
}

#[test]
fn osc52_write_ask_requires_confirmation_and_deny_ignores() {
    let h = harness!(Harness::recorder(
        r"\033]52;c;aGVsbG8=\007",
        &["--clipboard-write=ask"]
    ));
    h.wait("write requested", |h| !h.rt.writes.borrow().is_empty());
    assert!(h.rt.writes.borrow()[0].confirm);
    assert_eq!(h.clipboard(Clipboard::Standard), None);
    // The user confirms: the apprt calls complete_clipboard_request.
    let c = h.rt.confirmations.borrow()[0].clone();
    h.surface
        .complete_clipboard_request(c.request, &c.text, true)
        .unwrap();
    assert_eq!(h.rt.writes.borrow().len(), 2);

    let h = harness!(Harness::recorder(
        r"\033]52;c;aGVsbG8=\007",
        &["--clipboard-write=deny"]
    ));
    h.settle(100);
    assert!(h.rt.writes.borrow().is_empty());
}

#[test]
fn osc52_write_base64_acceptance_matches_zig() {
    // "YQ" (unpadded) is rejected, "YR==" (nonzero trailing bits) decodes.
    let h = harness!(Harness::recorder(r"\033]52;c;YQ\007", &[]));
    h.settle(100);
    assert!(h.rt.writes.borrow().is_empty());
    let h = harness!(Harness::recorder(r"\033]52;c;YR==\007", &[]));
    h.wait("clipboard written", |h| {
        h.clipboard(Clipboard::Standard).is_some()
    });
    assert_eq!(h.clipboard(Clipboard::Standard).as_deref(), Some("a"));
}

/// A child that sends an OSC 52 read query after it received one byte.
fn osc52_query_child(args: &[&str]) -> Option<Harness> {
    let h = Harness::new(Opts::default().args(args).script(
        "stty raw -echo; printf READY; head -c1 >/dev/null; printf '\\033]52;c;?\\007'; exec cat >'{out}'",
    ))?;
    h.wait_screen("READY");
    Some(h)
}

#[test]
fn osc52_read_requires_confirmation_by_default() {
    let h = harness!(osc52_query_child(&[]));
    h.set_clipboard_text(Clipboard::Standard, "secret");
    h.type_text("x");
    h.wait("confirmation asked", |h| {
        !h.rt.confirmations.borrow().is_empty()
    });
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
    let c = h.rt.confirmations.borrow()[0].clone();
    h.surface
        .complete_clipboard_request(c.request, &c.text, true)
        .unwrap();
    // base64("secret") = c2VjcmV0
    h.expect_input(b"\x1b]52;c;c2VjcmV0\x1b\\");
}

#[test]
fn osc52_read_allow_and_deny() {
    let h = harness!(osc52_query_child(&["--clipboard-read=allow"]));
    h.set_clipboard_text(Clipboard::Standard, "hi");
    h.type_text("x");
    h.expect_input(b"\x1b]52;c;aGk=\x1b\\");

    let h = harness!(osc52_query_child(&["--clipboard-read=deny"]));
    h.set_clipboard_text(Clipboard::Standard, "hi");
    h.type_text("x");
    h.settle(100);
    assert_eq!(h.recorded(), Vec::<u8>::new());
    assert!(h.rt.confirmations.borrow().is_empty());
}

#[test]
fn desktop_notifications() {
    let h = harness!(Harness::recorder(r"\033]9;just a body\007", &[]));
    let (t, b) = h.wait_action("osc9", |a| match a {
        Action::DesktopNotification(n) => Some((n.title.clone(), n.body.clone())),
        _ => None,
    });
    assert_eq!(b, "just a body");
    let _ = t;

    let h = harness!(Harness::recorder(
        r"\033]777;notify;My Title;My Body\007",
        &[]
    ));
    let (t, b) = h.wait_action("osc777", |a| match a {
        Action::DesktopNotification(n) => Some((n.title.clone(), n.body.clone())),
        _ => None,
    });
    assert_eq!((t.as_str(), b.as_str()), ("My Title", "My Body"));

    let h = harness!(Harness::recorder(
        r"\033]9;x\007",
        &["--desktop-notifications=false"]
    ));
    h.settle(100);
    assert!(!h.has_action(|a| matches!(a, Action::DesktopNotification(_))));
}

#[test]
fn progress_report_osc_9_4() {
    let h = harness!(Harness::recorder(r"\033]9;4;1;50\007", &[]));
    let p = h.wait_action("progress", |a| match a {
        Action::ProgressReport(p) => Some(*p),
        _ => None,
    });
    assert_eq!(p.state, ProgressState::Set);
    assert_eq!(p.progress, Some(50));
}

#[test]
fn child_exit_zero_closes_surface_by_default() {
    let h = harness!(Harness::new(Opts::default().script("sleep 0.4; exit 0")));
    h.wait("surface closed", |h| !h.rt.closes.borrow().is_empty());
    // normal exit after > abnormal runtime: the notice is shown in the terminal
    // when the apprt has no native UI.
    assert!(h.screen().contains("Process exited"), "{}", h.screen());
    assert!(ghostty_apprt::runtime_api::CoreSurface::child_exited(
        &*h.surface
    ));
}

#[test]
fn child_exit_with_wait_after_command_keeps_surface() {
    let h = harness!(Harness::new(
        Opts::default()
            .args(&["--wait-after-command=true"])
            .script("sleep 0.4; exit 0")
    ));
    h.wait("process exited text", |h| {
        h.screen().contains("Process exited")
    });
    h.settle(100);
    assert!(h.rt.closes.borrow().is_empty());
    // "Press any key to close the terminal": a key press closes it.
    h.key(Key::Enter, none(), "");
    assert!(!h.rt.closes.borrow().is_empty());
}

#[test]
fn child_exit_native_ui_gets_show_child_exited() {
    let h = harness!(Harness::new(Opts {
        show_child_exited: Some(true),
        ..Opts::default().script("sleep 0.4; exit 3")
    }));
    let info = h.wait_action("show child exited", |a| match a {
        Action::ShowChildExited(i) => Some(*i),
        _ => None,
    });
    assert_eq!(info.exit_code, 3);
    assert!(!h.screen().contains("Process exited"));
}

#[test]
fn abnormal_exit_shows_error_in_terminal_and_waits() {
    let h = harness!(Harness::new(Opts::default().script("exit 7")));
    h.wait("abnormal message", |h| {
        h.screen()
            .contains("failed to launch the requested command")
    });
    let s = h.screen();
    assert!(s.contains("Exit Code: 7"), "{s}");
    assert!(s.contains("Press any key to close the window"), "{s}");
    // waits for a key, doesn't close by itself
    h.settle(100);
    assert!(h.rt.closes.borrow().is_empty());
    h.key(Key::KeyA, none(), "a");
    assert!(!h.rt.closes.borrow().is_empty());
}

#[test]
fn ctrl_c_interrupts_cooked_child() {
    // A cooked tty turns ctrl+c (0x03) into SIGINT for the foreground group.
    let h = harness!(Harness::new(Opts::default().script(
        "trap 'printf GOT >\"{out}\"' INT; printf READY; while :; do sleep 0.05; done"
    )));
    h.wait_screen("READY");
    h.key(Key::KeyC, ctrl(), "c");
    h.wait("SIGINT delivered", |h| h.recorded() == b"GOT");
}
