//! The embedded surface flow driven through the exported C functions, on
//! Linux, using the `test-platform` seam: a headless EGL context supplies
//! OpenGL (Zig itself only supports surfaces on Darwin). A real shell runs
//! on a real pty behind the surface.
#![allow(
    unsafe_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::undocumented_unsafe_blocks,
    clippy::items_after_statements,
    clippy::float_cmp,
    clippy::assigning_clones
)]

use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_void},
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};

use ghostty::{
    config_capi::*,
    embedded::{test_platform, *},
    inspector::*,
    main_c::*,
    types::*,
};
use ghostty_renderer::headless::Headless;

const TIMEOUT: Duration = Duration::from_secs(30);

const GHOSTTY_MODS_SHIFT: i32 = 1;
const GHOSTTY_MODS_CTRL: i32 = 2;
const GHOSTTY_MODS_ALT: i32 = 4;
const GHOSTTY_BINDING_FLAGS_CONSUMED: i32 = 1;
const GHOSTTY_SURFACE_CONTEXT_TAB: i32 = 1;
const GHOSTTY_COLOR_SCHEME_DARK: i32 = 1;

// ---- recorded callbacks --------------------------------------------------

#[derive(Default)]
struct Rec {
    /// (action tag, target tag, surface pointer)
    actions: Vec<(i32, i32, usize)>,
    titles: Vec<String>,
    cell_size: Option<(u32, u32)>,
    writes: Vec<(i32, String, bool)>,
    reads: Vec<(i32, usize)>,
    confirms: Vec<(String, usize, i32)>,
    closes: Vec<bool>,
    child_exit: Option<u32>,
    mouse_shapes: Vec<i32>,
    pwds: Vec<String>,
    open_urls: Vec<String>,
    other_mimes: Vec<String>,
}

thread_local! {
    static REC: RefCell<Rec> = RefCell::new(Rec::default());
}

fn rec<R>(f: impl FnOnce(&mut Rec) -> R) -> R {
    REC.with(|r| f(&mut r.borrow_mut()))
}

static USERDATA: u8 = 7;

unsafe extern "C" fn on_wakeup(_: *mut c_void) {}

unsafe extern "C" fn on_action(
    _: ghostty_app_t,
    target: ghostty_target_s,
    action: ghostty_action_s,
) -> bool {
    let surf = if target.tag == target_tag::SURFACE {
        unsafe { target.target.surface as usize }
    } else {
        0
    };
    rec(|r| {
        r.actions.push((action.tag, target.tag, surf));
        match action.tag {
            32 => {
                // set_title
                let t = unsafe { CStr::from_ptr(action.action.set_title.title) };
                r.titles.push(t.to_string_lossy().into_owned());
            }
            25 => {
                let c = unsafe { action.action.cell_size };
                r.cell_size = Some((c.width, c.height));
            }
            36 => r.mouse_shapes.push(unsafe { action.action.mouse_shape }),
            35 => {
                let p = unsafe { CStr::from_ptr(action.action.pwd.pwd) };
                r.pwds.push(p.to_string_lossy().into_owned());
            }
            53 => {
                // open_url
                let u = unsafe { action.action.open_url };
                let b = unsafe { std::slice::from_raw_parts(u.url.cast::<u8>(), u.len) };
                r.open_urls.push(String::from_utf8_lossy(b).into_owned());
            }
            55 => r.child_exit = Some(unsafe { action.action.child_exited.exit_code }),
            _ => {}
        }
    });
    // The embedder handles show_child_exited itself.
    true
}

unsafe extern "C" fn on_read_clipboard(_: *mut c_void, c: i32, state: *mut c_void) -> bool {
    rec(|r| r.reads.push((c, state as usize)));
    true
}

unsafe extern "C" fn on_confirm_read(
    _: *mut c_void,
    text: *const c_char,
    state: *mut c_void,
    ty: i32,
) {
    let t = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    rec(|r| r.confirms.push((t, state as usize, ty)));
}

unsafe extern "C" fn on_write_clipboard(
    _: *mut c_void,
    c: i32,
    content: *const ghostty_clipboard_content_s,
    n: usize,
    confirm: bool,
) {
    let items = unsafe { std::slice::from_raw_parts(content, n) };
    for it in items {
        let mime = unsafe { CStr::from_ptr(it.mime) }.to_string_lossy();
        let data = unsafe { CStr::from_ptr(it.data) }
            .to_string_lossy()
            .into_owned();
        // Only the plain text flavor is recorded (copy also sends HTML).
        if mime == "text/plain" {
            rec(|r| r.writes.push((c, data, confirm)));
        } else {
            rec(|r| r.other_mimes.push(mime.into_owned()));
        }
    }
}

unsafe extern "C" fn on_close(_: *mut c_void, alive: bool) {
    rec(|r| r.closes.push(alive));
}

// ---- harness -----------------------------------------------------------

struct Host {
    _gl: Rc<Headless>,
    cfg: ghostty_config_t,
    app: ghostty_app_t,
    _conf_file: tempfile::NamedTempFile,
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

impl Host {
    fn new(extra_config: &str) -> Option<Host> {
        rec(|r| *r = Rec::default());
        let gl = match Headless::new() {
            Ok(g) => Rc::new(g),
            Err(e) => {
                eprintln!("skipping: no headless GL: {e}");
                return None;
            }
        };
        let g = Rc::clone(&gl);
        test_platform::set_gl_loader(Some(Rc::new(move |n| g.proc_address(n))));
        // SAFETY: no argv.
        assert_eq!(unsafe { ghostty_init(0, ptr::null_mut()) }, 0);

        let mut conf = tempfile::NamedTempFile::new().unwrap();
        use std::io::Write as _;
        write!(
            conf,
            "shell-integration = none\nwindow-padding-x = 0\nwindow-padding-y = 0\n\
             font-size = 12\nscrollback-limit = 100000\n{extra_config}\n"
        )
        .unwrap();
        let cfg = ghostty_config_new();
        let path = cs(conf.path().to_str().unwrap());
        unsafe {
            ghostty_config_load_file(cfg, path.as_ptr());
            ghostty_config_finalize(cfg);
        }
        assert_eq!(unsafe { ghostty_config_diagnostics_count(cfg) }, 0);

        let rt = ghostty_runtime_config_s {
            userdata: &raw const USERDATA as *mut c_void,
            supports_selection_clipboard: true,
            wakeup_cb: Some(on_wakeup),
            action_cb: Some(on_action),
            read_clipboard_cb: Some(on_read_clipboard),
            confirm_read_clipboard_cb: Some(on_confirm_read),
            write_clipboard_cb: Some(on_write_clipboard),
            close_surface_cb: Some(on_close),
        };
        let app = unsafe { ghostty_app_new(&rt, cfg) };
        assert!(!app.is_null());
        Some(Host {
            _gl: gl,
            cfg,
            app,
            _conf_file: conf,
        })
    }

    fn surface(&self, command: &str) -> ghostty_surface_t {
        self.surface_with(command, |_| {})
    }

    fn surface_with(
        &self,
        command: &str,
        f: impl FnOnce(&mut ghostty_surface_config_s),
    ) -> ghostty_surface_t {
        let mut sc = ghostty_surface_config_new();
        sc.platform_tag = test_platform::PLATFORM_TAG;
        sc.userdata = &raw const USERDATA as *mut c_void;
        sc.scale_factor = 2.0;
        let command = cs(command);
        sc.command = command.as_ptr();
        f(&mut sc);
        let s = unsafe { ghostty_surface_new(self.app, &sc) };
        assert!(!s.is_null(), "ghostty_surface_new failed");
        unsafe { ghostty_surface_set_size(s, 1000, 600) };
        s
    }

    fn pump(&self) {
        unsafe { ghostty_app_tick(self.app) };
    }

    fn wait(&self, what: &str, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if cond() {
                return;
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "timed out waiting for {what}\nscreen:\n{}",
                SCREEN.with(|c| c.borrow().clone())
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_screen(&self, s: ghostty_surface_t, needle: &str) {
        self.wait(&format!("screen to contain {needle:?}"), || {
            screen_text(s).contains(needle)
        });
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            ghostty_app_free(self.app);
            ghostty_config_free(self.cfg);
        }
        test_platform::set_gl_loader(None);
    }
}

macro_rules! host {
    ($($arg:expr),*) => {
        match Host::new($($arg),*) {
            Some(h) => h,
            None => return,
        }
    };
}

fn point(tag: i32, coord: i32) -> ghostty_point_s {
    ghostty_point_s {
        tag,
        coord,
        x: 0,
        y: 0,
    }
}

thread_local! {
    /// The last screen contents read (for failure messages).
    static SCREEN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The whole screen (history + active) as text.
fn screen_text(s: ghostty_surface_t) -> String {
    let sel = ghostty_selection_s {
        top_left: point(2, 1),
        bottom_right: point(2, 2),
        rectangle: false,
    };
    let mut text = unsafe { std::mem::zeroed::<ghostty_text_s>() };
    let ok = unsafe { ghostty_surface_read_text(s, sel, &mut text) };
    assert!(ok);
    let out = unsafe { CStr::from_ptr(text.text) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(out.len(), text.text_len);
    unsafe { ghostty_surface_free_text(s, &mut text) };
    SCREEN.with(|c| *c.borrow_mut() = out.clone());
    out
}

fn key(action: i32, keycode: u32, mods: i32, text: Option<&CStr>) -> ghostty_input_key_s {
    ghostty_input_key_s {
        action,
        mods,
        consumed_mods: 0,
        keycode,
        text: text.map_or(ptr::null(), CStr::as_ptr),
        unshifted_codepoint: 0,
        composing: false,
    }
}

// xkb keycodes (evdev + 8)
const KEY_A: u32 = 38;
const KEY_B: u32 = 56;
const KEY_C: u32 = 54;
const KEY_ENTER: u32 = 36;

const READ_ECHO: &str =
    "printf 'READY\\n'; while IFS= read -r line; do printf 'ECHO:%s\\n' \"$line\"; done";

// ---- tests -------------------------------------------------------------

#[test]
fn surface_new_reports_and_frees() {
    let h = host!("");
    let s = h.surface(READ_ECHO);
    h.wait_screen(s, "READY");

    // userdata and app round trip
    assert_eq!(
        unsafe { ghostty_surface_userdata(s) },
        &raw const USERDATA as *mut c_void
    );
    assert_eq!(unsafe { ghostty_surface_app(s) }, h.app);

    // The actions fired during init target the surface by its handle.
    let (cell, limit) = rec(|r| {
        (
            r.actions
                .iter()
                .any(|a| a.0 == 25 && a.1 == 1 && a.2 == s as usize),
            r.actions
                .iter()
                .any(|a| a.0 == 22 && a.1 == 1 && a.2 == s as usize),
        )
    });
    assert!(cell, "cell_size action for the surface");
    assert!(limit, "size_limit action for the surface");
    assert!(rec(|r| r.cell_size).unwrap().0 > 0);
    // The quit timer is stopped when the first surface is added.
    assert!(rec(|r| r.actions.iter().any(|a| a.0 == 41 && a.1 == 0)));

    // size
    let size = unsafe { ghostty_surface_size(s) };
    assert_eq!(size.width_px, 1000);
    assert_eq!(size.height_px, 600);
    assert!(
        size.columns > 10 && size.rows > 5,
        "{}x{}",
        size.columns,
        size.rows
    );
    assert!(size.cell_width_px > 0 && size.cell_height_px > 0);
    // 2x content scale doubles the cell size relative to 1x
    unsafe { ghostty_surface_set_content_scale(s, 1.0, 1.0) };
    let size1 = unsafe { ghostty_surface_size(s) };
    assert!(size1.cell_width_px < size.cell_width_px);
    // garbage scales are clamped to 1
    unsafe { ghostty_surface_set_content_scale(s, f64::NAN, -3.0) };
    assert_eq!(
        unsafe { ghostty_surface_size(s) }.cell_width_px,
        size1.cell_width_px
    );

    assert!(!unsafe { ghostty_surface_process_exited(s) });
    assert!(!unsafe { ghostty_surface_has_selection(s) });

    unsafe { ghostty_surface_free(s) };
    // The last surface going away starts the quit timer.
    assert!(rec(|r| r.actions.last().is_some_and(|a| a.0 == 41)));
}

#[test]
fn text_and_key_input_reach_the_child() {
    let h = host!("");
    let s = h.surface(READ_ECHO);
    h.wait_screen(s, "READY");

    // Raw text is treated like a paste.
    let t = b"hello\n";
    unsafe { ghostty_surface_text(s, t.as_ptr().cast(), t.len()) };
    h.wait_screen(s, "ECHO:hello");

    // Key events: press a, b, enter.
    let a = cs("a");
    let b = cs("b");
    let nl = cs("\r");
    assert!(unsafe { ghostty_surface_key(s, key(1, KEY_A, 0, Some(&a))) });
    let _ = unsafe { ghostty_surface_key(s, key(0, KEY_A, 0, Some(&a))) };
    assert!(unsafe { ghostty_surface_key(s, key(1, KEY_B, 0, Some(&b))) });
    assert!(unsafe { ghostty_surface_key(s, key(1, KEY_ENTER, 0, Some(&nl))) });
    h.wait_screen(s, "ECHO:ab");

    // ctrl+shift+c is bound by default on Linux (copy_to_clipboard).
    let c = cs("C");
    let ev = key(1, KEY_C, GHOSTTY_MODS_CTRL | GHOSTTY_MODS_SHIFT, Some(&c));
    let mut flags: u32 = 0xffff_ff00;
    assert!(unsafe { ghostty_surface_key_is_binding(s, ev, (&raw mut flags).cast()) });
    // Only the low byte is written (Zig writes the `u8` Flags.C).
    assert_eq!(flags & 0xffff_ff00, 0xffff_ff00);
    assert_eq!(
        flags & GHOSTTY_BINDING_FLAGS_CONSUMED as u32,
        GHOSTTY_BINDING_FLAGS_CONSUMED as u32
    );
    assert!(!unsafe {
        ghostty_surface_key_is_binding(s, key(1, KEY_A, 0, Some(&a)), ptr::null_mut())
    });

    // The invalid action is rejected.
    assert!(!unsafe { ghostty_surface_key(s, key(9, KEY_A, 0, Some(&a))) });

    // Mod translation is the identity on Linux.
    let m = GHOSTTY_MODS_CTRL | GHOSTTY_MODS_ALT;
    assert_eq!(unsafe { ghostty_surface_key_translation_mods(s, m) }, m);

    unsafe { ghostty_surface_free(s) };
}

#[test]
fn selection_and_clipboard_round_trip() {
    let h = host!("");
    let s = h.surface(READ_ECHO);
    h.wait_screen(s, "READY");

    // select_all + read_selection
    assert!(!unsafe { ghostty_surface_has_selection(s) });
    let mut text = unsafe { std::mem::zeroed::<ghostty_text_s>() };
    assert!(!unsafe { ghostty_surface_read_selection(s, &mut text) });
    let act = b"select_all";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    assert!(unsafe { ghostty_surface_has_selection(s) });
    assert!(unsafe { ghostty_surface_read_selection(s, &mut text) });
    let sel = unsafe { CStr::from_ptr(text.text) }
        .to_string_lossy()
        .into_owned();
    assert!(sel.contains("READY"), "{sel:?}");
    assert!(text.tl_px_x >= 0.0 || text.tl_px_x == -1.0);
    unsafe { ghostty_surface_free_text(s, &mut text) };

    // copy goes to the embedder's write callback
    let act = b"copy_to_clipboard";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    let (clip, data, _confirm) = rec(|r| r.writes.last().cloned()).expect("clipboard write");
    assert_eq!(clip, 0);
    assert!(data.contains("READY"));

    // invalid binding action strings are rejected
    let bad = b"no_such_action";
    assert!(!unsafe { ghostty_surface_binding_action(s, bad.as_ptr().cast(), bad.len()) });

    // paste: read callback -> complete
    let act = b"paste_from_clipboard";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    let (clip, state) = rec(|r| r.reads.last().copied()).expect("clipboard read");
    assert_eq!(clip, 0);
    // (A newline in the text would make the paste unsafe.)
    let pasted = cs("pasted-line");
    unsafe {
        ghostty_surface_complete_clipboard_request(s, pasted.as_ptr(), state as *mut c_void, false);
    };
    let nl = cs("\r");
    assert!(unsafe { ghostty_surface_key(s, key(1, KEY_ENTER, 0, Some(&nl))) });
    h.wait_screen(s, "ECHO:pasted-line");
    assert!(rec(|r| r.confirms.is_empty()));

    // an unsafe paste asks for confirmation and the state stays valid
    let act = b"paste_from_clipboard";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    let (_, state) = rec(|r| r.reads.last().copied()).unwrap();
    let nasty = cs("one\ntwo\n");
    unsafe {
        ghostty_surface_complete_clipboard_request(s, nasty.as_ptr(), state as *mut c_void, false);
    };
    let (ctext, cstate, cty) = rec(|r| r.confirms.last().cloned()).expect("confirmation requested");
    assert_eq!(ctext, "one\ntwo\n");
    assert_eq!(cstate, state);
    assert_eq!(cty, 0);
    // confirmed now
    unsafe {
        ghostty_surface_complete_clipboard_request(s, nasty.as_ptr(), state as *mut c_void, true);
    };
    h.wait_screen(s, "ECHO:one");
    h.wait_screen(s, "ECHO:two");

    unsafe { ghostty_surface_free(s) };
}

/// A child that prints the hex of the first two bytes it receives raw.
const HEX_TWO: &str = "stty raw -echo; dd bs=1 count=2 2>/dev/null | od -An -tx1 | tr -d ' \\n'; printf '\\nHEX-DONE\\n'; sleep 5";

#[test]
fn clipboard_completion_and_binding_action_keep_raw_bytes() {
    // Clipboard contents are bytes: 0xFF must reach the child as 0xFF.
    let h = host!("");
    let s = h.surface(HEX_TWO);
    let act = b"paste_from_clipboard";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    let (_, state) = rec(|r| r.reads.last().copied()).expect("clipboard read");
    let pasted = CString::new(&b"a\xff"[..]).unwrap();
    unsafe {
        ghostty_surface_complete_clipboard_request(s, pasted.as_ptr(), state as *mut c_void, false);
    };
    h.wait_screen(s, "61ff");
    unsafe { ghostty_surface_free(s) };

    // Binding action text is bytes too: `text:` followed by 0xFF.
    let s = h.surface(HEX_TWO);
    let act = b"text:b\xff";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    h.wait_screen(s, "62ff");
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn osc_52_and_title_and_pwd_actions() {
    let h = host!("clipboard-write = allow\n");
    let s = h.surface(
        "printf '\\033]0;my title\\007\\033]7;file://localhost/tmp\\007\\033]52;c;aGVsbG8=\\007DONE\\n'; sleep 30",
    );
    h.wait_screen(s, "DONE");
    h.wait("title action", || {
        rec(|r| r.titles.iter().any(|t| t == "my title"))
    });
    h.wait("pwd action", || {
        rec(|r| r.pwds.iter().any(|p| p.contains("/tmp")))
    });
    h.wait("osc52 write", || {
        rec(|r| r.writes.iter().any(|w| w.1 == "hello"))
    });

    // The inherited config carries the pwd (tab-inherit-working-directory).
    let inherited = unsafe { ghostty_surface_inherited_config(s, GHOSTTY_SURFACE_CONTEXT_TAB) };
    assert_eq!(inherited.context, GHOSTTY_SURFACE_CONTEXT_TAB);
    assert!(!inherited.working_directory.is_null());
    let wd = unsafe { CStr::from_ptr(inherited.working_directory) }.to_string_lossy();
    assert_eq!(wd, "/tmp");
    assert_eq!(inherited.font_size, 12.0);
    // a new surface can be created with the inherited config
    let mut cfg = inherited;
    cfg.platform_tag = test_platform::PLATFORM_TAG;
    cfg.userdata = &raw const USERDATA as *mut c_void;
    let command = cs("pwd; sleep 30");
    cfg.command = command.as_ptr();
    let s2 = unsafe { ghostty_surface_new(h.app, &cfg) };
    assert!(!s2.is_null());
    unsafe { ghostty_surface_set_size(s2, 800, 400) };
    h.wait_screen(s2, "/tmp");

    unsafe {
        ghostty_surface_free(s2);
        ghostty_surface_free(s);
    }
}

#[test]
fn surface_options_apply() {
    let h = host!("");
    let vars = [
        ghostty_env_var_s {
            key: c"GHOSTTY_LIB_TEST".as_ptr(),
            value: c"env-value".as_ptr(),
        },
        ghostty_env_var_s {
            key: c"GHOSTTY_LIB_TEST2".as_ptr(),
            value: c"second".as_ptr(),
        },
    ];
    let mut vars = vars;
    let s = h.surface_with(
        "printf 'ENV=%s,%s PWD=%s\\n' \"$GHOSTTY_LIB_TEST\" \"$GHOSTTY_LIB_TEST2\" \"$PWD\"; \
         IFS= read -r line; printf 'INPUT=%s\\n' \"$line\"; sleep 30",
        |sc| {
            sc.env_vars = vars.as_mut_ptr();
            sc.env_var_count = 2;
            sc.working_directory = c"/usr".as_ptr();
            sc.initial_input = c"typed \"quoted\" \\ text\n".as_ptr();
            sc.font_size = 20.0;
        },
    );
    h.wait_screen(s, "ENV=env-value,second PWD=/usr");
    h.wait_screen(s, r#"INPUT=typed "quoted" \ text"#);
    // The font size override took effect: bigger cells than the default.
    let big = unsafe { ghostty_surface_size(s) };
    let s2 = h.surface("sleep 30");
    let normal = unsafe { ghostty_surface_size(s2) };
    assert!(big.cell_height_px > normal.cell_height_px);
    unsafe {
        ghostty_surface_free(s2);
        ghostty_surface_free(s);
    }
}

#[test]
fn working_directory_that_does_not_exist_is_ignored() {
    let h = host!("");
    let s = h.surface_with("printf 'CWD=%s\\n' \"${PWD##*/}\"; sleep 30", |sc| {
        sc.working_directory = c"/definitely/not/a/dir".as_ptr();
    });
    // Falls back to the default working directory (inherit, or home when the
    // process looks desktop launched), the shell still starts.
    h.wait_screen(s, "CWD=");
    assert!(!screen_text(s).contains("CWD=dir"));
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn mouse_capture_and_events() {
    let h = host!("");
    let s = h.surface("printf '\\033[?1000h\\033[?1006hMOUSEON\\n'; cat");
    h.wait_screen(s, "MOUSEON");
    assert!(unsafe { ghostty_surface_mouse_captured(s) });

    // Mouse reports reach the child (SGR encoding) and are echoed by cat's
    // tty as ^[[<...
    unsafe { ghostty_surface_mouse_pos(s, 10.0, 10.0, 0) };
    let _ = unsafe { ghostty_surface_mouse_button(s, 1, 1, 0) };
    unsafe { ghostty_surface_mouse_button(s, 0, 1, 0) };
    h.wait_screen(s, "[<0;");

    // invalid enum values are rejected
    assert!(!unsafe { ghostty_surface_mouse_button(s, 7, 1, 0) });
    assert!(!unsafe { ghostty_surface_mouse_button(s, 1, 99, 0) });

    // scroll and pressure don't crash
    unsafe {
        ghostty_surface_mouse_scroll(s, 0.0, 3.0, 0);
        ghostty_surface_mouse_scroll(s, 0.0, -3.0, 1);
        ghostty_surface_mouse_pressure(s, 1, 0.5);
        ghostty_surface_mouse_pressure(s, 9, 0.5);
    }
    let (mut x, mut y, mut w, mut hh) = (0.0, 0.0, 0.0, 0.0);
    unsafe { ghostty_surface_ime_point(s, &mut x, &mut y, &mut w, &mut hh) };
    assert!(w >= 0.0 && hh > 0.0);
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn focus_occlusion_preedit_scheme_and_splits() {
    let h = host!("");
    let s = h.surface("printf 'READY\\n'; sleep 30");
    h.wait_screen(s, "READY");
    unsafe {
        ghostty_surface_set_focus(s, false);
        ghostty_surface_set_focus(s, true);
        ghostty_surface_set_occlusion(s, false);
        ghostty_surface_set_occlusion(s, true);
        ghostty_surface_refresh(s);
        ghostty_surface_set_color_scheme(s, GHOSTTY_COLOR_SCHEME_DARK);
        ghostty_surface_set_color_scheme(s, 5);
        let p = b"compose";
        ghostty_surface_preedit(s, p.as_ptr().cast(), p.len());
        ghostty_surface_preedit(s, ptr::null(), 0);
    }

    // splits are actions to the embedder, targeting the surface
    unsafe {
        ghostty_surface_split(s, 0);
        ghostty_surface_split_focus(s, 1);
        ghostty_surface_split_resize(s, 3, 10);
        ghostty_surface_split_equalize(s);
        // invalid directions are ignored
        ghostty_surface_split(s, 42);
    }
    let tags: Vec<_> = rec(|r| {
        r.actions
            .iter()
            .filter(|a| a.2 == s as usize)
            .map(|a| a.0)
            .collect()
    });
    for want in [4, 16, 18, 19] {
        assert!(tags.contains(&want), "missing action {want} in {tags:?}");
    }
    assert_eq!(tags.iter().filter(|t| **t == 4).count(), 1);

    // binding actions that go to the embedder
    let act = b"new_window";
    assert!(unsafe { ghostty_surface_binding_action(s, act.as_ptr().cast(), act.len()) });
    assert!(rec(|r| r
        .actions
        .iter()
        .any(|a| a.0 == 1 && a.2 == s as usize)));

    // request_close goes through the normal close path -> close callback
    unsafe { ghostty_surface_request_close(s) };
    h.wait("close callback", || rec(|r| !r.closes.is_empty()));
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn child_exit_is_reported() {
    let h = host!("");
    let s = h.surface("exit 3");
    h.wait("child exit action", || rec(|r| r.child_exit.is_some()));
    assert!(unsafe { ghostty_surface_process_exited(s) });
    assert_eq!(rec(|r| r.child_exit), Some(3));
    assert!(!unsafe { ghostty_surface_needs_confirm_quit(s) });
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn update_config_reaches_surfaces() {
    let h = host!("");
    let s = h.surface("printf 'READY\\n'; sleep 30");
    h.wait_screen(s, "READY");
    let before = unsafe { ghostty_surface_size(s) };

    let mut f = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write as _;
    write!(
        f,
        "font-size = 24\nwindow-padding-x = 0\nwindow-padding-y = 0\n"
    )
    .unwrap();
    let c2 = ghostty_config_new();
    let path = cs(f.path().to_str().unwrap());
    unsafe {
        ghostty_config_load_file(c2, path.as_ptr());
        ghostty_config_finalize(c2);
        ghostty_app_update_config(h.app, c2);
    }
    h.pump();
    assert!(rec(|r| r.actions.iter().any(|a| a.0 == 48 && a.1 == 0)));
    let after = unsafe { ghostty_surface_size(s) };
    assert!(
        after.cell_height_px > before.cell_height_px,
        "{} -> {}",
        before.cell_height_px,
        after.cell_height_px
    );

    // surface-only update
    let c3 = ghostty_config_new();
    unsafe {
        ghostty_surface_update_config(s, c3);
        ghostty_config_free(c3);
        ghostty_config_free(c2);
        ghostty_surface_free(s);
    }
}

#[test]
fn inspector_api() {
    let h = host!("");
    let s = h.surface("printf 'READY\\n'; sleep 30");
    h.wait_screen(s, "READY");

    let i = unsafe { ghostty_surface_inspector(s) };
    assert!(!i.is_null());
    // one inspector per surface
    assert_eq!(unsafe { ghostty_surface_inspector(s) }, i);
    unsafe {
        ghostty_inspector_set_content_scale(i, 2.0, 2.0);
        ghostty_inspector_set_size(i, 800, 600);
        ghostty_inspector_set_focus(i, true);
        ghostty_inspector_mouse_pos(i, 10.0, 10.0);
        ghostty_inspector_mouse_button(i, 1, 1, 0);
        ghostty_inspector_mouse_button(i, 0, 1, 0);
        ghostty_inspector_mouse_button(i, 1, 9, 0);
        ghostty_inspector_mouse_scroll(i, 0.0, 1.0, 1);
        ghostty_inspector_key(i, 1, 18, GHOSTTY_MODS_CTRL);
        ghostty_inspector_text(i, c"abc".as_ptr());
    }
    // input queues renders of the inspector
    assert!(rec(|r| r
        .actions
        .iter()
        .any(|a| a.0 == 30 && a.2 == s as usize)));
    unsafe {
        ghostty_inspector_free(s);
    }
    // a new one can be created afterwards
    let i2 = unsafe { ghostty_surface_inspector(s) };
    assert!(!i2.is_null());
    // and it is freed with the surface
    unsafe { ghostty_surface_free(s) };
}

#[test]
fn surface_without_platform_is_rejected() {
    let h = host!("");
    let mut sc = ghostty_surface_config_new();
    // no platform
    assert!(unsafe { ghostty_surface_new(h.app, &sc) }.is_null());
    sc.platform_tag = platform::MACOS;
    // Zig's Platform.init rejects Darwin platforms off Darwin.
    assert!(unsafe { ghostty_surface_new(h.app, &sc) }.is_null());
}

#[test]
fn selection_clipboard_support_is_reported() {
    // supports_selection_clipboard = true in this host: an OSC 52 write to
    // the primary selection goes out with clipboard=1.
    let h = host!("clipboard-write = allow\n");
    let s = h.surface("printf '\\033]52;p;c2VsZWN0aW9u\\007DONE\\n'; sleep 30");
    h.wait_screen(s, "DONE");
    h.wait("selection write", || {
        rec(|r| {
            r.writes
                .iter()
                .any(|w| w.1 == "selection" && (w.0 == 1 || w.0 == 2))
        })
    });
    unsafe { ghostty_surface_free(s) };
}
