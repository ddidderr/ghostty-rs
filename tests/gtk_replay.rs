#![allow(clippy::unwrap_used)]
//! Runs the real GTK application on the current Wayland/X11 session with the
//! `GHOSTTY_DEBUG_INPUT_SCRIPT` replay hook and checks what reaches the pty
//! (a `cat > file` child) and the clipboard. Covers the paths a real user
//! drives with an input method or a confirmation dialog: IME preedit/commit
//! through the surface's real signal handlers, the unsafe-paste dialog and
//! the OSC 52 read/write confirmation dialogs (confirm and deny).
//!
//! Skipped (passes) when there is no display or `GHOSTTY_SKIP_GTK_REPLAY` is
//! set. The tests share the session clipboard, so they run one at a time.
//! Child processes explicitly enable debug logs on stderr: replay assertions
//! and layer-shell detection must not depend on the user's logging settings.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn have_display() -> bool {
    std::env::var_os("GHOSTTY_SKIP_GTK_REPLAY").is_none()
        && (std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some())
}

struct Run {
    log: String,
}

impl Run {
    /// Lines of the replay log (`debug input: ...`) with the prefix removed.
    fn lines(&self) -> Vec<&str> {
        self.log
            .lines()
            .filter_map(|l| l.split_once("debug input: ").map(|x| x.1))
            .collect()
    }

    fn has(&self, needle: &str) -> bool {
        self.lines().iter().any(|l| l.contains(needle))
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ghostty-gtk-replay-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// Run the app with `script`, `extra` config args and the child command
/// `sh -c <child>`.
fn run(script: &str, extra: &[&str], child: &str) -> Run {
    let script_path = scratch("script.txt");
    // Distinct file per call so parallel tests never clash.
    let script_path = script_path.with_file_name(format!(
        "script-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&script_path, format!("animations off\n{script}\nquit\n")).unwrap();
    let mut proc = Command::new(env!("CARGO_BIN_EXE_ghostty"))
        .args([
            "--config-default-files=false",
            "--gtk-single-instance=false",
            "--confirm-close-surface=false",
        ])
        .args(extra)
        .args(["-e", "sh", "-c", child])
        .env("GHOSTTY_DEBUG_INPUT_SCRIPT", &script_path)
        // Replay output is warn-level; Wayland capability detection is debug-level.
        .env("RUST_LOG", "debug")
        .env("GHOSTTY_LOG", "stderr")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = proc.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut { stderr }, &mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if proc.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = proc.kill();
            let _ = proc.wait();
            panic!("app did not quit; log:\n{}", reader.join().unwrap());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&script_path);
    Run {
        log: reader.join().unwrap(),
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn ime_preedit_commit_and_dead_key_flow() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let out = scratch("ime.out");
    let _ = std::fs::remove_file(&out);
    let r = run(
        "\
wait 800
ime-preedit かn
ime-state
ime-commit こん
ime-state
ime-key preedit apostrophe ´
ime-key commit a á
ime-key handled-commit x x
ime-key pass y
ime-key preedit apostrophe ´
ime-key commit-simple e é
ime-state
key enter
key ctrl+d
wait 800",
        &[],
        &format!("cat > {}", out.display()),
    );
    // The preedit never reaches the pty, committed text and plain keys do.
    assert_eq!(read(&out), "こんáxyé\n", "log:\n{}", r.log);
    // composing: preedit -> true, commit -> false, ... , end -> false.
    let state: Vec<&str> = r
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("composing=") || l.starts_with("ime-key consumed="))
        .collect();
    assert_eq!(
        state,
        [
            "composing=true",
            "composing=false",
            // dead key: handled while composing -> not encoded
            "ime-key consumed=true composing=true",
            // commit of the composed char: handled, no key encode
            "ime-key consumed=true composing=false",
            // plain translation commit: handled but encoded with the key
            "ime-key consumed=false composing=false",
            // not handled: encoded
            "ime-key consumed=false composing=false",
            "ime-key consumed=true composing=true",
            "ime-key consumed=true composing=false",
            "composing=false",
        ],
        "log:\n{}",
        r.log
    );
}

#[test]
fn unsafe_paste_dialog_deny_then_confirm() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let out = scratch("paste.out");
    let _ = std::fs::remove_file(&out);
    let r = run(
        "\
wait 800
clipboard-set rm -rf a\\nsecond\\n
key ctrl+shift+v
wait 800
dialog-info
dialog-click Deny
wait 500
dialog-info
key ctrl+shift+v
wait 800
dialog-click Allow
wait 500
key ctrl+d
wait 800",
        &[],
        &format!("cat > {}", out.display()),
    );
    assert!(r.has("Warning: Potentially Unsafe Paste"), "{}", r.log);
    assert!(
        r.has("dialog contents=\"rm -rf a\\nsecond\\n\""),
        "{}",
        r.log
    );
    // Deny closed the dialog (the second dialog-info found none).
    assert!(r.has("no dialog is open"), "{}", r.log);
    // Exactly one copy: the denied paste wrote nothing.
    assert_eq!(read(&out), "rm -rf a\nsecond\n", "{}", r.log);
}

#[test]
fn osc52_write_ask_allow_deny_and_remember() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    // Deny: clipboard unchanged.
    let r = run(
        "\
clipboard-set before
wait 1500
dialog-info
dialog-click Deny
wait 500
clipboard-dump
wait 500",
        &["--clipboard-write=ask"],
        "printf '\\033]52;c;aGVsbG8=\\a'; sleep 8",
    );
    assert!(
        r.has("body=\"An application is attempting to write"),
        "{}",
        r.log
    );
    assert!(r.has("dialog contents=\"hello\""), "{}", r.log);
    assert!(r.has("clipboard = Some(\"before\")"), "{}", r.log);

    // Allow: clipboard written.
    let r = run(
        "\
clipboard-set before
wait 1500
dialog-click Allow
wait 500
clipboard-dump
wait 500",
        &["--clipboard-write=ask"],
        "printf '\\033]52;c;aGVsbG8=\\a'; sleep 8",
    );
    assert!(r.has("clipboard = Some(\"hello\")"), "{}", r.log);

    // Allow + remember: the second write does not prompt.
    let r = run(
        "\
wait 1500
dialog-remember
dialog-click Allow
wait 3000
dialog-info
clipboard-dump
wait 500",
        &["--clipboard-write=ask"],
        "printf '\\033]52;c;aGVsbG8=\\a'; sleep 2; printf '\\033]52;c;d29ybGQ=\\a'; sleep 8",
    );
    assert!(r.has("no dialog is open"), "{}", r.log);
    assert!(r.has("clipboard = Some(\"world\")"), "{}", r.log);
}

#[test]
fn osc52_read_ask_allow_and_deny() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let out = scratch("osc52-read.out");
    let child = format!(
        "rm -f {o}; stty -icanon -echo; sleep 2; printf '\\033]52;c;?\\a'; timeout --foreground 6 dd bs=1 count=15 of={o} 2>/dev/null; sleep 1",
        o = out.display()
    );
    for (button, expect) in [("Allow", "\x1b]52;c;c2VjcmV0"), ("Deny", "")] {
        let _ = std::fs::remove_file(&out);
        let r = run(
            &format!(
                "clipboard-set secret\nwait 3500\ndialog-info\ndialog-click {button}\nwait 6000"
            ),
            &["--clipboard-read=ask"],
            &child,
        );
        assert!(
            r.has("body=\"An application is attempting to read"),
            "{}",
            r.log
        );
        assert!(r.has("dialog contents=\"secret\""), "{}", r.log);
        assert_eq!(read(&out), expect, "button {button}\n{}", r.log);
    }
}

/// `libgtk4-layer-shell` must be loaded before `libwayland-client`; if the
/// link order regresses `is_supported()` is false and no layer surface is
/// created on a compositor that does implement wlr-layer-shell.
#[test]
fn quick_terminal_layer_surface_toggle_and_autohide() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let script = "\
wait 800
key ctrl+shift+q
wait 1500
windows-dump
quick-focus-out
wait 1000
windows-dump
key ctrl+shift+q
wait 2000
windows-dump";
    let r = run(
        script,
        &[
            "--keybind=global:ctrl+shift+q=toggle_quick_terminal",
            "--quick-terminal-position=bottom",
            "--quick-terminal-size=50%,30%",
            "--gtk-quick-terminal-layer=overlay",
            "--quick-terminal-autohide=true",
        ],
        "sleep 20",
    );
    if !r.log.contains("found global zwlr_layer_shell_v1") {
        return; // compositor without wlr-layer-shell (or X11)
    }
    assert!(
        !r.log.contains("disabling quick terminal"),
        "layer shell not usable (library load order?)\n{}",
        r.log
    );
    let quick: Vec<&str> = r
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("window quick_terminal=true"))
        .collect();
    assert_eq!(quick.len(), 3, "{}", r.log);
    // Shown as a layer surface anchored to the bottom edge only.
    assert!(quick[0].contains("visible=true"), "{}", quick[0]);
    assert!(
        quick[0].contains("layer=Overlay keyboard=OnDemand")
            && quick[0].contains("anchors(l,r,t,b)=[false, false, false, true]"),
        "{}",
        quick[0]
    );
    // Losing focus autohides (quick-terminal-autohide is false by default on
    // Linux, enabled above). The replay can only simulate a focus loss when
    // the compositor has not actually focused the layer surface (GTK's
    // `is-active` is read-only); if it has, the autohide branch cannot be
    // driven from here, so only the toggle behaviour is checked.
    let compositor_focused = quick[0].contains("active=true");
    if compositor_focused {
        eprintln!("note: quick terminal is compositor-focused; skipping autohide assertion");
    } else {
        assert!(quick[1].contains("visible=false"), "{}", quick[1]);
        // Toggling shows it again.
        assert!(quick[2].contains("visible=true"), "{}", quick[2]);
    }
}

/// Horizontal touchpad scrolling switches tabs at +-120 accumulated units
/// (+ = previous, - = next) and the accumulator resets after 500 ms.
#[test]
fn horizontal_scroll_switches_tabs() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let r = run(
        "\
wait 800
key ctrl+shift+t
wait 1500
key ctrl+shift+t
wait 1500
tabs-dump
hscroll 70
tabs-dump
hscroll 70
tabs-dump
hscroll -130
tabs-dump
hscroll -130
tabs-dump
hscroll 50
wait 700
hscroll 50
tabs-dump
hscroll 100
tabs-dump",
        &[],
        "sleep 20",
    );
    let tabs: Vec<&str> = r
        .lines()
        .into_iter()
        .filter_map(|l| l.strip_prefix("tabs n=3 selected="))
        .collect();
    assert_eq!(
        tabs,
        [
            "Some(2)", // new tab is selected
            "Some(2)", // 70 < 120: accumulating
            "Some(1)", // 140: previous
            "Some(2)", // -130: next
            "Some(2)", // next at the last tab stays
            "Some(2)", // 50, timeout, 50: reset in between
            "Some(1)", // 50 + 100: previous
        ],
        "{}",
        r.log
    );
}

#[test]
fn quick_terminal_stays_open_without_autohide() {
    if !have_display() {
        return;
    }
    let _serial = serial();
    let r = run(
        "wait 800\nkey ctrl+shift+q\nwait 1500\nquick-focus-out\nwait 1000\nwindows-dump",
        &["--keybind=global:ctrl+shift+q=toggle_quick_terminal"],
        "sleep 20",
    );
    if !r.log.contains("found global zwlr_layer_shell_v1") {
        return;
    }
    assert!(
        r.has("window quick_terminal=true visible=true"),
        "{}",
        r.log
    );
}
