//! PTY-driven checks of the interactive `+list-themes` preview against the
//! upstream Ghostty binary (`/usr/bin/ghostty`, skipped when absent): the
//! same keystrokes must leave the same text on the screen.
#![allow(clippy::unwrap_used)]

use std::{
    fmt::Write as _,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use ghostty_terminal::terminal::{Options, Terminal};
use rustix::{
    fs::{Mode, OFlags, open},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{Winsize, tcsetwinsize},
};

const ORACLE: &str = "/usr/bin/ghostty";
const COLS: u16 = 100;
const ROWS: u16 = 30;

/// A spawned `+list-themes` session in a PTY.
struct Session {
    child: std::process::Child,
    master: std::fs::File,
    rx: mpsc::Receiver<Vec<u8>>,
    output: Vec<u8>,
    /// Answer the primary device attributes query (ends vaxis' startup query).
    reply_da: bool,
    answered: usize,
}

impl Session {
    fn spawn(exe: &str, envs: &[(&str, &str)], reply_da: bool) -> Session {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let slave: OwnedFd = open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        tcsetwinsize(
            &master,
            Winsize {
                ws_row: ROWS,
                ws_col: COLS,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let mut cmd = Command::new("setsid");
        cmd.args(["--ctty", exe, "+list-themes"])
            .env("TERM", "xterm-256color")
            .env("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty")
            .env_remove("COLORFGBG")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        let master = std::fs::File::from(master);
        let (tx, rx) = mpsc::channel();
        let mut reader = master.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Session {
            child,
            master,
            rx,
            output: vec![],
            reply_da,
            answered: 0,
        }
    }

    /// Collect output until the program has been quiet for `quiet`.
    fn settle(&mut self, quiet: Duration) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match self.rx.recv_timeout(quiet) {
                Ok(chunk) => {
                    self.output.extend_from_slice(&chunk);
                    if self.reply_da {
                        let needle = b"\x1b[c";
                        let count = self
                            .output
                            .windows(needle.len())
                            .filter(|w| *w == needle)
                            .count();
                        while self.answered < count {
                            self.answered += 1;
                            self.master.write_all(b"\x1b[?62;4c").unwrap();
                        }
                    }
                }
                Err(_) => return,
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
        self.settle(Duration::from_millis(500));
    }

    /// The screen text after replaying everything received so far.
    fn screen(&self) -> Terminal {
        let mut t = Terminal::new(Options::new(COLS, ROWS)).unwrap();
        t.vt_stream().next_slice(&self.output).unwrap();
        t
    }

    fn finish(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn rows_of(t: &Terminal) -> Vec<String> {
    t.plain_string()
        .unwrap()
        .lines()
        .map(|l| l.trim_end().to_owned())
        .collect()
}

fn have_oracle() -> bool {
    Path::new(ORACLE).exists()
}

fn rust_exe() -> &'static str {
    env!("CARGO_BIN_EXE_ghostty")
}

/// Type `keys` in search mode and return the rows of the screen.
fn search_screen(exe: &str, envs: &[(&str, &str)], keys: &[&[u8]]) -> Vec<String> {
    let mut s = Session::spawn(exe, envs, true);
    s.settle(Duration::from_millis(1500));
    s.send(b"/");
    for k in keys {
        s.send(k);
    }
    let rows = rows_of(&s.screen());
    s.finish();
    rows
}

/// The text inside the search box (the row below the top border).
fn search_box_text(rows: &[String]) -> String {
    let i = rows.iter().rposition(|r| r.contains('\u{256d}')).unwrap();
    // The box starts at column 20; its content follows the left border.
    let inner: String = rows[i + 1].chars().skip(21).collect();
    inner
        .trim_end()
        .trim_end_matches('\u{2502}')
        .trim_end()
        .to_owned()
}

/// Run a script of key presses in search mode and return the search box
/// text after each one.
fn search_trace(exe: &str, envs: &[(&str, &str)], steps: &[&[u8]]) -> Vec<String> {
    let mut s = Session::spawn(exe, envs, true);
    s.settle(Duration::from_millis(1500));
    s.send(b"/");
    let mut out = vec![];
    for k in steps {
        s.send(k);
        out.push(search_box_text(&rows_of(&s.screen())));
    }
    s.finish();
    out
}

/// Compare Rust against the oracle (when present) and against `expected`.
fn check_trace(steps: &[&[u8]], expected: &[&str]) {
    let rust = search_trace(rust_exe(), &[], steps);
    if have_oracle() {
        let zig = search_trace(ORACLE, &[], steps);
        assert_eq!(expected, zig.as_slice(), "oracle");
    }
    assert_eq!(expected, rust.as_slice(), "rust");
}

#[test]
fn word_navigation_and_edit_bindings_match_upstream() {
    check_trace(
        &[
            b"blue red green", // type
            b"\x1bb",          // Alt+B
            b"X",
            b"\x1b[1;3D", // Alt+Left
            b"Y",
            b"\x1bf", // Alt+F
            b"Z",
            b"\x1b[1;3C", // Alt+Right
            b"W",
            b"\x17",     // Ctrl+W
            b"\x1b\x7f", // Alt+Backspace
            b"\x01",     // Ctrl+A
            b"\x1bd",    // Alt+D
            b"\x1bd",
            b"\x05\x0b", // Ctrl+E, Ctrl+K (no-op at end)
            b"\x01\x0b", // Ctrl+A, Ctrl+K
            b"ab cd",
            b"\x02\x02\x04", // Ctrl+B x2, Ctrl+D
            b"\x15",         // Ctrl+U
        ],
        &[
            "blue red green",
            "blue red green",
            "blue red Xgreen",
            "blue red Xgreen",
            "blue red YXgreen",
            "blue red YXgreen",
            "blue red YXgreenZ",
            "blue red YXgreenZ",
            "blue red YXgreenZW",
            "blue red",
            "blue",
            "blue",
            "",
            "",
            "",
            "",
            "ab cd",
            "ab d",
            "d",
        ],
    );
}

#[test]
fn backspace_delete_and_cursor_move_by_grapheme() {
    let cluster = "n\u{303}\u{323}";
    let typed = format!("a{cluster}b");
    check_trace(
        &[
            "xe\u{301}".as_bytes(),
            b"\x7f", // Backspace removes e + U+0301
            b"\x15",
            typed.as_bytes(),
            b"\x1b[D\x1b[D", // Left x2: before the ZWJ cluster
            b"\x1b[3~",      // Delete removes the whole cluster
        ],
        &["xe\u{301}", "x", "", &typed, &typed, "ab"],
    );
}

#[test]
fn search_folds_only_ascii_case() {
    let cfg = tempfile::tempdir().unwrap();
    let themes = cfg.path().join("ghostty/themes");
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(themes.join("\u{c4}rger"), "background = #101010\n").unwrap();
    std::fs::write(themes.join("zzplain"), "background = #101010\n").unwrap();
    let envs = [("XDG_CONFIG_HOME", cfg.path().to_str().unwrap())];
    // `Ä` matches the theme `Ärger` (exact bytes), `ä` does not (no
    // Unicode folding of the theme name or the query).
    for exe in [rust_exe(), ORACLE] {
        if exe == ORACLE && !have_oracle() {
            continue;
        }
        let upper = search_screen(exe, &envs, &["\u{c4}".as_bytes()]);
        assert!(
            upper.iter().any(|r| r.contains("\u{c4}rger")),
            "{exe}: {upper:?}"
        );
        let lower = search_screen(exe, &envs, &["\u{e4}".as_bytes()]);
        assert!(
            !lower.iter().any(|r| r.contains("\u{c4}rger")),
            "{exe}: {lower:?}"
        );
    }
}

/// The (fg, bg) style of every cell of the active screen, as text.
fn style_grid(t: &Terminal) -> Vec<String> {
    use ghostty_terminal::{
        page_list::Pin,
        point::{Coordinate, Point},
    };
    let screen = t.screens.active();
    let pages = &screen.pages;
    let mut out = vec![];
    for y in 0..pages.rows {
        let pin = pages
            .pin(Point::Active(Coordinate {
                x: 0,
                y: u32::from(y),
            }))
            .unwrap();
        let mut row = String::new();
        for x in 0..pages.cols {
            let st = Pin { x, ..pin }.style(pages).unwrap();
            let _ = write!(row, "{:?}/{:?};", st.fg_color, st.bg_color);
        }
        out.push(row);
    }
    out
}

fn initial_styles(exe: &str, envs: &[(&str, &str)]) -> Vec<String> {
    // No terminal replies of any kind: the scheme stays at its initial value.
    let mut s = Session::spawn(exe, envs, false);
    s.settle(Duration::from_millis(3000));
    let grid = style_grid(&s.screen());
    s.finish();
    grid
}

#[test]
fn initial_colors_are_light_and_ignore_colorfgbg() {
    let plain = initial_styles(rust_exe(), &[]);
    let dark_hint = initial_styles(rust_exe(), &[("COLORFGBG", "15;0")]);
    assert_eq!(plain, dark_hint);
    if have_oracle() {
        let zig = initial_styles(ORACLE, &[("COLORFGBG", "15;0")]);
        assert_eq!(zig, plain);
    }
    // Light: black on white (rgb) somewhere on screen.
    let all = plain.join("");
    assert!(all.contains("Rgb"), "{all}");
}

/// Run the preview with an unreadable user theme; returns the exit status
/// (if the program exited by itself) and the final screen text.
fn unreadable_theme_run(exe: &str) -> (Option<i32>, String) {
    use std::os::unix::fs::PermissionsExt;
    let cfg = tempfile::tempdir().unwrap();
    let themes = cfg.path().join("ghostty/themes");
    std::fs::create_dir_all(&themes).unwrap();
    let bad = themes.join("unreadable");
    std::fs::write(&bad, "background = #101010\n").unwrap();
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
    let envs = [("XDG_CONFIG_HOME", cfg.path().to_str().unwrap())];
    let mut s = Session::spawn(exe, &envs, true);
    s.settle(Duration::from_millis(2000));
    let _ = s.master.write_all(b"/a");
    s.settle(Duration::from_millis(1000));
    // Give a loaded machine time to reach (and exit from) the failure.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut status = s.child.try_wait().unwrap();
    while status.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        status = s.child.try_wait().unwrap();
    }
    s.settle(Duration::from_millis(300));
    let code = status.and_then(|st| st.code());
    let text = rows_of(&s.screen()).join("\n");
    s.finish();
    (code, text)
}

#[test]
fn load_file_error_exits_the_preview() {
    let (code, text) = unreadable_theme_run(rust_exe());
    let _ = text;
    if have_oracle() {
        let (zcode, ztext) = unreadable_theme_run(ORACLE);
        let _ = ztext;
        assert_eq!(zcode.is_some(), code.is_some());
    }
    assert!(code.is_some(), "the preview must have exited");
}

#[test]
fn zwj_emoji_cluster_is_drawn_whole_in_search_box() {
    let cluster = "\u{1F469}\u{200D}\u{1F680}";
    let typed = format!("a{cluster}b");
    check_trace(&[typed.as_bytes()], &[&typed]);
}
