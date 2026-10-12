//! End-to-end test harness: the REAL core `Surface` (real termio with a real
//! pty + child process, real `Terminal`, real renderer on a headless EGL
//! context) driven through a fake apprt (`FakeRt`) that records the performed
//! actions and implements the clipboard in memory.
//!
//! The harness is single-threaded like the GTK main loop: the test thread is
//! the "main thread" and calls [`Harness::pump`] (which is `App::tick` plus
//! answering pending clipboard reads) while polling for conditions.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use ghostty_apprt::{
    action::{Action, Target},
    runtime_api::{ClipboardError, CoreApp, InputEffect, RtApp, RtSurface, SurfaceId, Waker},
    structs::{Clipboard, ClipboardRequest, ContentScale, CursorPos, SurfaceSize},
};
use ghostty_config::{command::Command, config::Config};
use ghostty_core::{App, Surface};
use ghostty_input::{
    Key,
    KeyEvent,
    Mods,
    MouseButton,
    MouseButtonState,
    ScrollMods,
    key::Action as KeyAction,
};
use ghostty_renderer::headless::Headless;

/// Generous upper bound for every polled condition.
pub const TIMEOUT: Duration = Duration::from_secs(30);

struct NopWaker;
impl Waker for NopWaker {
    fn wake(&self) {}
}

/// A recorded `set_clipboard` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardWrite {
    pub clipboard: Clipboard,
    pub contents: Vec<(String, Vec<u8>)>,
    pub confirm: bool,
}

/// A confirmation dialog the GTK apprt would show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub request: ClipboardRequest,
    pub text: String,
}

/// The fake apprt (both `RtApp` and `RtSurface`).
pub struct FakeRt {
    pub actions: RefCell<Vec<(Target<SurfaceId>, Action)>>,
    /// In-memory clipboards (plain text).
    pub clipboards: RefCell<HashMap<u8, String>>,
    pub writes: RefCell<Vec<ClipboardWrite>>,
    pending: RefCell<VecDeque<(Clipboard, ClipboardRequest)>>,
    pub confirmations: RefCell<Vec<Confirmation>>,
    pub closes: RefCell<Vec<bool>>,
    pub size: Cell<SurfaceSize>,
    pub scale: Cell<ContentScale>,
    pub cursor: Cell<CursorPos>,
    /// What `perform_action(ShowChildExited)` returns (GTK: true).
    pub show_child_exited: Cell<bool>,
    pub title: RefCell<Option<String>>,
    gl: Rc<Headless>,
}

fn cb_key(c: Clipboard) -> u8 {
    match c {
        Clipboard::Standard => 0,
        Clipboard::Selection => 1,
        Clipboard::Primary => 2,
    }
}

impl RtApp for FakeRt {
    fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool {
        let ret = match &action {
            Action::ShowChildExited(_) => self.show_child_exited.get(),
            Action::SetTitle(t) => {
                *self.title.borrow_mut() = Some(t.title.to_text_lossy());
                true
            }
            _ => true,
        };
        self.actions.borrow_mut().push((target, action));
        ret
    }
}

impl RtSurface for FakeRt {
    fn close(&self, process_active: bool) {
        self.closes.borrow_mut().push(process_active);
    }
    fn cgroup(&self) -> Option<String> {
        None
    }
    fn get_title(&self) -> Option<ghostty_os::osbytes::EncodedOs> {
        self.title.borrow().clone().map(Into::into)
    }
    fn get_content_scale(&self) -> ContentScale {
        self.scale.get()
    }
    fn get_size(&self) -> SurfaceSize {
        self.size.get()
    }
    fn get_cursor_pos(&self) -> CursorPos {
        self.cursor.get()
    }
    fn clipboard_request(&self, clipboard: Clipboard, state: ClipboardRequest) -> bool {
        // Like GTK: paste requests need text on the clipboard.
        if state == ClipboardRequest::Paste
            && !self.clipboards.borrow().contains_key(&cb_key(clipboard))
        {
            return false;
        }
        self.pending.borrow_mut().push_back((clipboard, state));
        true
    }
    fn set_clipboard(&self, clipboard: Clipboard, contents: &[(String, Vec<u8>)], confirm: bool) {
        self.writes.borrow_mut().push(ClipboardWrite {
            clipboard,
            contents: contents.to_vec(),
            confirm,
        });
        if let Some((_, bytes)) = contents.iter().find(|(m, _)| m == "text/plain") {
            // Test harness: the fake clipboard stores text.
            let text = &String::from_utf8_lossy(bytes).into_owned();
            // Like GTK, a confirm=true write shows a dialog first; the
            // harness only stores unconfirmed writes.
            if confirm {
                self.confirmations.borrow_mut().push(Confirmation {
                    request: ClipboardRequest::Osc52Write(clipboard),
                    text: text.clone(),
                });
            } else {
                self.clipboards
                    .borrow_mut()
                    .insert(cb_key(clipboard), text.clone());
            }
        }
    }
    fn default_termio_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        // Like GTK: the process environment minus what the runtime does not
        // want to leak (here HOME stands in for GDK_DISABLE & co), plus extras.
        let mut env: Vec<(std::ffi::OsString, std::ffi::OsString)> =
            std::env::vars_os().filter(|(k, _)| k != "HOME").collect();
        env.push(("TERM_PROGRAM_TEST".into(), "1".into()));
        env
    }
    fn gl_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        self.gl.proc_address(name)
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Options for building a harness.
#[derive(Default, Clone)]
pub struct Opts {
    /// Extra config args (`--key=value`).
    pub args: Vec<String>,
    /// `sh -c` script; `{out}` is replaced by the recording file path.
    pub script: Option<String>,
    /// Surface size in pixels (default 800x600).
    pub size: Option<(u32, u32)>,
    pub scale: Option<f32>,
    pub show_child_exited: Option<bool>,
}

impl Opts {
    pub fn args(mut self, a: &[&str]) -> Opts {
        self.args.extend(a.iter().map(|s| (*s).to_owned()));
        self
    }
    pub fn script(mut self, s: &str) -> Opts {
        self.script = Some(s.to_owned());
        self
    }
}

pub struct Harness {
    pub app: App,
    pub rt: Rc<FakeRt>,
    pub surface: Rc<Surface>,
    pub out: PathBuf,
    pub id: SurfaceId,
    base: Cell<usize>,
    // Dropped last: the GL context must outlive the surface.
    _gl: Rc<Headless>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.out);
    }
}

fn tmp_path(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    dir.join(format!("e2e-{}-{}-{tag}", std::process::id(), n))
}

impl Harness {
    /// Try to build; `None` when no headless GL is available.
    pub fn try_new(opts: &Opts) -> Option<Harness> {
        let gl = match Headless::new() {
            Ok(g) => Rc::new(g),
            Err(e) => {
                eprintln!("SKIP: headless EGL unavailable: {e}");
                return None;
            }
        };
        let (w, h) = opts.size.unwrap_or((800, 600));
        gl.set_viewport(w as i32, h as i32);
        let s = opts.scale.unwrap_or(1.0);
        let rt = Rc::new(FakeRt {
            actions: RefCell::default(),
            clipboards: RefCell::default(),
            writes: RefCell::default(),
            pending: RefCell::default(),
            confirmations: RefCell::default(),
            closes: RefCell::default(),
            size: Cell::new(SurfaceSize {
                width: w,
                height: h,
            }),
            scale: Cell::new(ContentScale { x: s, y: s }),
            cursor: Cell::new(CursorPos { x: 0.0, y: 0.0 }),
            show_child_exited: Cell::new(opts.show_child_exited.unwrap_or(false)),
            title: RefCell::default(),
            gl: Rc::clone(&gl),
        });
        let out = tmp_path("rec");
        let _ = std::fs::remove_file(&out);

        let mut config = Config::new_default();
        let mut args: Vec<String> = vec![
            "--linux-cgroup=never".into(),
            "--shell-integration=none".into(),
            "--config-default-files=false".into(),
            // Keep tests quiet and deterministic.
            "--desktop-notifications=true".into(),
        ];
        args.extend(opts.args.iter().cloned());
        let mut it = ghostty_config::cli_args::VecIterator::new(args);
        config.load_iter(&mut it).expect("config args");
        if let Some(script) = &opts.script {
            let script = script.replace("{out}", out.to_str().unwrap());
            config.command = Some(Command::Direct(vec![
                "sh".into(),
                "-c".into(),
                script.into(),
            ]));
        }
        config.finalize().expect("finalize");

        let app = App::new(Arc::new(NopWaker)).expect("app");
        let id: SurfaceId = 1;
        let rt_app: Rc<dyn RtApp> = rt.clone();
        let rt_surface: Rc<dyn RtSurface> = rt.clone();
        let core = app
            .new_surface(id, config, rt_app, rt_surface)
            .expect("new_surface");
        let surface = app.state().surface(id).expect("surface registered");
        drop(core);
        Some(Harness {
            app,
            rt,
            surface,
            out,
            id,
            base: Cell::new(0),
            _gl: gl,
        })
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn new(opts: Opts) -> Option<Harness> {
        Harness::try_new(&opts)
    }

    /// A recorder child: raw tty, no echo, optional setup escape output, then
    /// prints READY and records every byte it receives into the out file.
    pub fn recorder(setup_printf: &str, args: &[&str]) -> Option<Harness> {
        let script =
            format!("stty raw -echo; printf '{setup_printf}'; printf READY; exec cat >'{{out}}'");
        let h = Harness::new(Opts::default().args(args).script(&script))?;
        h.wait_screen("READY");
        Some(h)
    }

    /// Like [`Harness::recorder`] but runs the shell snippet `pre` (cooked
    /// tty) first, e.g. to print lots of output.
    pub fn with_pre(pre: &str, args: &[&str]) -> Option<Harness> {
        let script = format!("{pre}; stty raw -echo; printf READY; exec cat >'{{out}}'");
        let h = Harness::new(Opts::default().args(args).script(&script))?;
        h.wait_screen("READY");
        Some(h)
    }

    // ------------------------------------------------------------------
    // Event loop

    /// `App::tick` plus answering pending clipboard reads (async in GTK).
    pub fn pump(&self) {
        self.app.tick(self.rt.as_ref()).unwrap();
        loop {
            let next = self.rt.pending.borrow_mut().pop_front();
            let Some((clipboard, req)) = next else { break };
            let text = self
                .rt
                .clipboards
                .borrow()
                .get(&cb_key(clipboard))
                .cloned()
                .unwrap_or_default();
            match self
                .surface
                .complete_clipboard_request(req, text.as_bytes(), false)
            {
                Ok(()) => {}
                Err(ClipboardError::UnsafePaste | ClipboardError::UnauthorizedPaste) => {
                    self.rt
                        .confirmations
                        .borrow_mut()
                        .push(Confirmation { request: req, text });
                }
                Err(e) => panic!("clipboard request failed: {e}"),
            }
        }
    }

    /// Poll `cond` (pumping the app) until true or timeout.
    pub fn wait(&self, what: &str, mut cond: impl FnMut(&Harness) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if cond(self) {
                return;
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "timeout waiting for {what}\nscreen:\n{}\nrecorded: {:?}\nactions: {:#?}",
                self.screen(),
                self.recorded(),
                self.action_names(),
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Assert `cond` stays false for a short period (negative checks). The
    /// duration is a deliberate settle time, used sparingly.
    pub fn settle(&self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            self.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // ------------------------------------------------------------------
    // Observation

    /// The visible viewport as plain text.
    pub fn screen(&self) -> String {
        self.surface.lock_terminal().plain_string().unwrap()
    }

    pub fn wait_screen(&self, needle: &str) {
        self.wait(&format!("screen to contain {needle:?}"), |h| {
            h.screen().contains(needle)
        });
    }

    /// Bytes the recorder child has received so far.
    pub fn recorded(&self) -> Vec<u8> {
        let all = std::fs::read(&self.out).unwrap_or_default();
        all.get(self.base.get()..).unwrap_or_default().to_vec()
    }

    /// Wait for exactly `expected` (as a prefix of what was recorded, then
    /// the full length) and assert equality.
    pub fn expect_input(&self, expected: &[u8]) {
        self.wait(&format!("child input {expected:?}"), |h| {
            h.recorded().len() >= expected.len()
        });
        assert_eq!(
            String::from_utf8_lossy(&self.recorded()),
            String::from_utf8_lossy(expected)
        );
        assert_eq!(self.recorded(), expected);
    }

    /// Start a fresh recording window: later `recorded()`/`expect_input`
    /// only see bytes after this point. Call after an `expect_input`.
    pub fn mark(&self) {
        let len = std::fs::metadata(&self.out).map_or(0, |m| m.len() as usize);
        self.base.set(len);
    }

    pub fn actions(&self) -> Vec<Action> {
        self.rt
            .actions
            .borrow()
            .iter()
            .map(|(_, a)| a.clone())
            .collect()
    }

    pub fn action_names(&self) -> Vec<String> {
        self.rt
            .actions
            .borrow()
            .iter()
            .filter(|(_, a)| !matches!(a, Action::Render | Action::Scrollbar(_)))
            .map(|(t, a)| format!("{t:?} {a:?}"))
            .collect()
    }

    /// Wait for an action matching `f` and return it.
    pub fn wait_action<T>(&self, what: &str, f: impl Fn(&Action) -> Option<T>) -> T {
        let mut found = None;
        self.wait(what, |h| {
            found = h.rt.actions.borrow().iter().find_map(|(_, a)| f(a));
            found.is_some()
        });
        found.unwrap()
    }

    pub fn has_action(&self, f: impl Fn(&Action) -> bool) -> bool {
        self.rt.actions.borrow().iter().any(|(_, a)| f(a))
    }

    pub fn count_actions(&self, f: impl Fn(&Action) -> bool) -> usize {
        self.rt
            .actions
            .borrow()
            .iter()
            .filter(|(_, a)| f(a))
            .count()
    }

    pub fn clear_actions(&self) {
        self.rt.actions.borrow_mut().clear();
    }

    // ------------------------------------------------------------------
    // Input

    pub fn key_event(&self, ev: KeyEvent<'_>) -> InputEffect {
        self.surface.key_callback(ev).unwrap()
    }

    /// Press (and release) a key with the given mods and text.
    pub fn key(&self, key: Key, mods: Mods, utf8: &str) -> InputEffect {
        let unshifted = utf8.chars().next().map_or(0, |c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase() as u32
            } else {
                c as u32
            }
        });
        let eff = self.key_event(KeyEvent {
            action: KeyAction::Press,
            key,
            mods,
            consumed_mods: if mods.shift && !utf8.is_empty() {
                Mods {
                    shift: true,
                    ..Mods::default()
                }
            } else {
                Mods::default()
            },
            composing: false,
            utf8: utf8.as_bytes(),
            unshifted_codepoint: unshifted,
        });
        // GTK reports the same text/codepoint on release.
        self.key_event(KeyEvent {
            action: KeyAction::Release,
            key,
            mods,
            consumed_mods: Mods::default(),
            composing: false,
            utf8: utf8.as_bytes(),
            unshifted_codepoint: unshifted,
        });
        eff
    }

    /// A key without text (arrows, F-keys, ...).
    pub fn special(&self, key: Key, mods: Mods) -> InputEffect {
        self.key(key, mods, "")
    }

    /// Type printable ASCII text char by char.
    pub fn type_text(&self, s: &str) {
        for c in s.chars() {
            let (key, shift) = key_for_char(c);
            let mods = Mods {
                shift,
                ..Mods::default()
            };
            let mut buf = [0u8; 4];
            self.key(key, mods, c.encode_utf8(&mut buf));
        }
    }

    /// Write bytes to the child by pasting-unsafe means: straight into the
    /// pty through the IO mailbox, bypassing paste protection.
    pub fn paste_raw(&self, s: &str) {
        self.surface.text_callback(s).unwrap();
    }

    pub fn press_mouse(&self, button: MouseButton, mods: Mods) -> bool {
        self.surface
            .mouse_button_callback(MouseButtonState::Press, button, mods)
            .unwrap()
    }

    pub fn release_mouse(&self, button: MouseButton, mods: Mods) -> bool {
        self.surface
            .mouse_button_callback(MouseButtonState::Release, button, mods)
            .unwrap()
    }

    /// Pixel position of the center of a cell.
    pub fn cell_pos(&self, col: u32, row: u32) -> CursorPos {
        let s = self.surface.size();
        CursorPos {
            x: (s.padding.left + col * s.cell.width + s.cell.width / 2) as f32,
            y: (s.padding.top + row * s.cell.height + s.cell.height / 2) as f32,
        }
    }

    /// Position inside a cell; `fx`/`fy` in [0,1).
    pub fn cell_pos_frac(&self, col: u32, row: u32, fx: f32, fy: f32) -> CursorPos {
        let s = self.surface.size();
        CursorPos {
            x: s.padding.left as f32 + (col as f32 + fx) * s.cell.width as f32,
            y: s.padding.top as f32 + (row as f32 + fy) * s.cell.height as f32,
        }
    }

    pub fn move_to_pos(&self, p: CursorPos, mods: Option<Mods>) {
        self.rt.cursor.set(p);
        self.surface.cursor_pos_callback(p, mods).unwrap();
    }

    pub fn move_to(&self, col: u32, row: u32, mods: Option<Mods>) {
        let p = self.cell_pos(col, row);
        self.rt.cursor.set(p);
        self.surface.cursor_pos_callback(p, mods).unwrap();
    }

    pub fn click(&self, col: u32, row: u32, button: MouseButton, mods: Mods) {
        self.move_to(col, row, Some(mods));
        self.press_mouse(button, mods);
        self.release_mouse(button, mods);
    }

    pub fn scroll(&self, x: f64, y: f64) {
        self.surface
            .scroll_callback(x, y, ScrollMods::default())
            .unwrap();
    }

    pub fn bind(&self, action: &str) -> bool {
        let a = ghostty_input::binding::Action::parse(action).expect("parse action");
        self.surface.perform_binding_action(&a).unwrap()
    }

    /// Clipboard text.
    pub fn clipboard(&self, c: Clipboard) -> Option<String> {
        self.rt.clipboards.borrow().get(&cb_key(c)).cloned()
    }

    pub fn set_clipboard_text(&self, c: Clipboard, text: &str) {
        self.rt
            .clipboards
            .borrow_mut()
            .insert(cb_key(c), text.to_owned());
    }

    pub fn grid(&self) -> (u16, u16) {
        let g = self.surface.size().grid();
        (g.columns, g.rows)
    }
}

/// Physical key for an ASCII char (US layout) and whether shift is needed.
pub fn key_for_char(c: char) -> (Key, bool) {
    use Key::*;
    let letters = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    let digits = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    match c {
        'a'..='z' => (letters[(c as u8 - b'a') as usize], false),
        'A'..='Z' => (letters[(c as u8 - b'A') as usize], true),
        '0'..='9' => (digits[(c as u8 - b'0') as usize], false),
        ')' => (Digit0, true),
        '!' => (Digit1, true),
        '@' => (Digit2, true),
        '#' => (Digit3, true),
        '$' => (Digit4, true),
        '%' => (Digit5, true),
        '^' => (Digit6, true),
        '&' => (Digit7, true),
        '*' => (Digit8, true),
        '(' => (Digit9, true),
        ' ' => (Space, false),
        '-' => (Minus, false),
        '_' => (Minus, true),
        '=' => (Equal, false),
        '+' => (Equal, true),
        '.' => (Period, false),
        ',' => (Comma, false),
        '/' => (Slash, false),
        ';' => (Semicolon, false),
        ':' => (Semicolon, true),
        '\'' => (Quote, false),
        '"' => (Quote, true),
        '[' => (BracketLeft, false),
        ']' => (BracketRight, false),
        '\\' => (Backslash, false),
        '`' => (Backquote, false),
        _ => (Unidentified, false),
    }
}

pub fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}
pub fn alt() -> Mods {
    Mods {
        alt: true,
        ..Mods::default()
    }
}
pub fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}
pub fn ctrl_shift() -> Mods {
    Mods {
        ctrl: true,
        shift: true,
        ..Mods::default()
    }
}
pub fn none() -> Mods {
    Mods::default()
}

/// Skip the test when headless GL isn't available.
#[macro_export]
macro_rules! harness {
    ($e:expr) => {
        match $e {
            Some(h) => h,
            None => return,
        }
    };
}
