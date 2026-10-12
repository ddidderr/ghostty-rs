//! Debug-only input replay (`GHOSTTY_DEBUG_INPUT_SCRIPT=<file>`).
//!
//! Compositors under test (niri) do not let us inject keys into a window, so
//! this hook synthesizes the events inside the process, at the same entry
//! points the GTK handlers use: key events go through
//! `Surface::debug_key_event` (the keyval/keycode/state translation of
//! `Surface::key_event`, i.e. `gtk/key.rs`, then `key_callback`) and text goes
//! through the IM context's `commit` signal (`im_commit`).
//!
//! Script format, one command per line (`#` starts a comment):
//!
//! ```text
//! key ctrl+shift+t        # press+release; mods: ctrl shift alt super
//! text hello world        # IM commit of the rest of the line
//! wait 200                # milliseconds
//! screenshot /abs/out.png # dump the renderer's last frame (PNG)
//! inspector-screenshot /abs/out.png # dump the next frame of the terminal
//!                                   # inspector window (PNG)
//! inspector-click 100 20  # left click in the inspector (logical px)
//! inspector-scroll 100 200 -3  # mouse wheel in the inspector
//! mouse-move 100 50 [ctrl+shift]   # pointer motion (logical px)
//! mouse-press 1 [mods]    # button 1/2/3 at the last position
//! mouse-release 1 [mods]
//! click 1 [mods]          # press+release; repeat quickly for multi-click
//! drag 10 10 200 10 [mods]  # move, press left, move in steps, release
//! scroll 0 -3             # wheel (dx dy, in ticks)
//! hscroll 130             # horizontal touchpad scroll (surface units); +-120 switches tabs
//! tabs-dump               # log the tab count and selected tab of the active window
//! resize 600 400         # set the window default size (logical px)
//! clipboard-set a\nb       # set the GDK clipboard (\n and \\ escapes)
//! window-screenshot /abs/out.png  # GTK render of the whole window (dialogs included)
//! windows-dump            # log every toplevel window / dialog
//! clipboard-dump          # log the GDK clipboard and primary text
//! ime-preedit ´a          # IM: preedit-start (if needed) + preedit-changed with this preedit
//! ime-preedit-end         # IM: preedit-end
//! ime-commit á            # IM: commit signal (outside any key event)
//! ime-key <mode> <key> [text]  # key press+release through the IM filter, mode:
//!                         # preedit | commit | commit-simple | handled-commit | pass
//! ime-state               # log whether the surface is composing
//! animations off          # disable GTK/libadwaita animations (so dialogs render at once)
//! quick-focus-out         # emit notify::is-active on the quick terminal window (it is not
//!                         # compositor-focused here), running the autohide handler
//! dialog-info             # log the open AdwAlertDialog (heading, body, responses, contents)
//! dialog-remember         # turn on the "remember choice" switch of the open dialog
//! dialog-click Allow      # click the dialog button with this label (Allow/Deny/OK/Cancel...)
//! quit                    # quit the application
//! ```
//!
//! Key names are GDK keysym names (`t`, `Return`, `Tab`, `equal`, `Up`, `F5`)
//! with a few lowercase aliases (`enter`, `esc`, `up`, ...). Commands act on
//! the active surface of the active window at the time they run.

use std::{cell::RefCell, rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::{gdk, glib, glib::translate::IntoGlib};

use crate::class::{
    application::Application,
    surface::{DebugIm, DebugImKey, Surface},
    window::Window,
};

/// One parsed command.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Key(String),
    Text(String),
    Wait(u64),
    Screenshot(String),
    InspectorScreenshot(String),
    InspectorClick(f64, f64),
    InspectorScroll(f64, f64, f64),
    MouseMove(f64, f64, String),
    MousePress(u32, String),
    MouseRelease(u32, String),
    Click(u32, String),
    Drag(f64, f64, f64, f64, String),
    Scroll(f64, f64),
    HScroll(f64),
    TabsDump,
    ClipboardDump,
    Resize(i32, i32),
    ClipboardSet(String),
    WindowsDump,
    WindowScreenshot(String),
    ImePreedit(String),
    ImePreeditEnd,
    ImeCommit(String),
    ImeKey(String, String, String),
    ImeState,
    Animations(bool),
    DialogInfo,
    QuickFocusOut,
    DialogRemember,
    DialogClick(String),
    Quit,
}

/// Parse a script. Errors carry the 1-based line number.
pub fn parse(script: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (i, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim_start();
        let err = |m: &str| format!("line {}: {m}", i + 1);
        steps.push(match cmd {
            "key" if !rest.is_empty() => Step::Key(rest.to_owned()),
            "text" if !rest.is_empty() => Step::Text(rest.to_owned()),
            "wait" => Step::Wait(rest.parse().map_err(|_| err("wait needs milliseconds"))?),
            "screenshot" if !rest.is_empty() => Step::Screenshot(rest.to_owned()),
            "inspector-screenshot" if !rest.is_empty() => {
                Step::InspectorScreenshot(rest.to_owned())
            }
            "inspector-click" => {
                let v: Vec<f64> = rest
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| err("inspector-click needs x y"))?;
                let [x, y] = v[..] else {
                    return Err(err("inspector-click needs x y"));
                };
                Step::InspectorClick(x, y)
            }
            "inspector-scroll" => {
                let v: Vec<f64> = rest
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| err("inspector-scroll needs x y dy"))?;
                let [x, y, dy] = v[..] else {
                    return Err(err("inspector-scroll needs x y dy"));
                };
                Step::InspectorScroll(x, y, dy)
            }
            "mouse-move" | "drag" => {
                let n = if cmd == "drag" { 4 } else { 2 };
                let w: Vec<&str> = rest.split_whitespace().collect();
                if w.len() < n || w.len() > n + 1 {
                    return Err(err(&format!("{cmd} needs {n} numbers [mods]")));
                }
                let v: Vec<f64> = w[..n]
                    .iter()
                    .map(|x| x.parse())
                    .collect::<Result<_, _>>()
                    .map_err(|_| err("bad number"))?;
                let mods = w.get(n).unwrap_or(&"").to_string();
                if cmd == "drag" {
                    Step::Drag(v[0], v[1], v[2], v[3], mods)
                } else {
                    Step::MouseMove(v[0], v[1], mods)
                }
            }
            "mouse-press" | "mouse-release" | "click" => {
                let w: Vec<&str> = rest.split_whitespace().collect();
                let b = w
                    .first()
                    .and_then(|x| x.parse().ok())
                    .ok_or_else(|| err("needs a button number"))?;
                let mods = w.get(1).unwrap_or(&"").to_string();
                match cmd {
                    "mouse-press" => Step::MousePress(b, mods),
                    "mouse-release" => Step::MouseRelease(b, mods),
                    _ => Step::Click(b, mods),
                }
            }
            "scroll" => {
                let v: Vec<f64> = rest
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| err("scroll needs dx dy"))?;
                let [dx, dy] = v[..] else {
                    return Err(err("scroll needs dx dy"));
                };
                Step::Scroll(dx, dy)
            }
            "hscroll" => Step::HScroll(rest.parse().map_err(|_| err("hscroll needs dx"))?),
            "tabs-dump" => Step::TabsDump,
            "clipboard-dump" => Step::ClipboardDump,
            "windows-dump" => Step::WindowsDump,
            "window-screenshot" if !rest.is_empty() => Step::WindowScreenshot(rest.to_owned()),
            "clipboard-set" => Step::ClipboardSet(rest.replace("\\n", "\n")),
            "resize" => {
                let v: Vec<i32> = rest
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| err("resize needs w h"))?;
                let [w, h] = v[..] else {
                    return Err(err("resize needs w h"));
                };
                Step::Resize(w, h)
            }
            "ime-preedit" if !rest.is_empty() => Step::ImePreedit(rest.to_owned()),
            "ime-preedit-end" => Step::ImePreeditEnd,
            "ime-commit" if !rest.is_empty() => Step::ImeCommit(rest.to_owned()),
            "ime-state" => Step::ImeState,
            "animations" => Step::Animations(match rest {
                "on" => true,
                "off" => false,
                _ => return Err(err("animations needs on|off")),
            }),
            "dialog-info" => Step::DialogInfo,
            "quick-focus-out" => Step::QuickFocusOut,
            "dialog-remember" => Step::DialogRemember,
            "dialog-click" if !rest.is_empty() => Step::DialogClick(rest.to_owned()),
            "ime-key" => {
                let mut it = rest.splitn(3, char::is_whitespace);
                let (Some(mode), Some(key)) = (it.next(), it.next()) else {
                    return Err(err("ime-key needs <mode> <key> [text]"));
                };
                if ![
                    "preedit",
                    "commit",
                    "commit-simple",
                    "handled-commit",
                    "pass",
                ]
                .contains(&mode)
                {
                    return Err(err("bad ime-key mode"));
                }
                let text = it.next().unwrap_or("").trim_start().to_owned();
                Step::ImeKey(mode.to_owned(), key.to_owned(), text)
            }
            "quit" => Step::Quit,
            _ => return Err(err(&format!("bad command {line:?}"))),
        });
    }
    Ok(steps)
}

/// Parse `ctrl+shift+t` into (keyval, state).
pub fn parse_key(spec: &str) -> Result<(gdk::Key, gdk::ModifierType), String> {
    let mut state = gdk::ModifierType::empty();
    let parts: Vec<&str> = spec.split('+').collect();
    let (name, mods) = parts.split_last().ok_or("empty key")?;
    // "ctrl++" style: the key itself is "+"
    let (name, mods) = if name.is_empty() {
        ("plus", &mods[..mods.len().saturating_sub(1)])
    } else {
        (*name, mods)
    };
    for m in mods {
        state |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => gdk::ModifierType::CONTROL_MASK,
            "shift" => gdk::ModifierType::SHIFT_MASK,
            "alt" => gdk::ModifierType::ALT_MASK,
            "super" | "meta" => gdk::ModifierType::SUPER_MASK,
            other => return Err(format!("unknown modifier {other:?}")),
        };
    }
    let alias = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" => "space",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" => "Page_Up",
        "pagedown" | "page_down" => "Page_Down",
        "insert" => "Insert",
        _ => name,
    };
    let key = gdk::Key::from_name(alias)
        .or_else(|| gdk::Key::from_name(name))
        .ok_or_else(|| format!("unknown key {name:?}"))?;
    Ok((key, state))
}

/// Start the replay if `GHOSTTY_DEBUG_INPUT_SCRIPT` is set.
pub fn start(app: &Application) {
    let Some(path) = std::env::var_os("GHOSTTY_DEBUG_INPUT_SCRIPT") else {
        return;
    };
    let script = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            log::error!("debug input: cannot read {}: {err}", path.to_string_lossy());
            return;
        }
    };
    let steps = match parse(&script) {
        Ok(s) => s,
        Err(err) => {
            log::error!("debug input: {err}");
            return;
        }
    };
    log::warn!("debug input: replaying {} steps", steps.len());
    let runner = Rc::new(Runner {
        app: app.clone(),
        steps,
        next: RefCell::new(0),
        pos: std::cell::Cell::new((0.0, 0.0)),
        mock_im: mock_im::MockIm::default(),
    });
    // Wait for the first surface to be ready (realized, core initialized).
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if runner.active_surface().is_some_and(|s| s.core().is_some()) {
            runner.run();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

struct Runner {
    app: Application,
    steps: Vec<Step>,
    next: RefCell<usize>,
    /// Last pointer position (logical px).
    pos: std::cell::Cell<(f64, f64)>,
    /// Stands in for the input method context as the signal emitter.
    mock_im: mock_im::MockIm,
}

/// Render the whole widget tree of `win` (GL area content may be blank, but
/// dialogs and popovers are included) to a PNG.
fn window_screenshot(win: &impl IsA<gtk::Widget>, path: &str) {
    let (w, h) = (win.width(), win.height());
    let paintable = gtk::WidgetPaintable::new(Some(win.as_ref()));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        snapshot.upcast_ref::<gdk::Snapshot>(),
        f64::from(w),
        f64::from(h),
    );
    let Some(node) = snapshot.to_node() else {
        log::warn!("debug input: empty window snapshot");
        return;
    };
    let Some(renderer) = win.native().and_then(|n| n.renderer()) else {
        log::warn!("debug input: no renderer");
        return;
    };
    let texture = renderer.render_texture(node, None);
    match texture.save_to_png(path) {
        Ok(()) => log::warn!("debug input: wrote window screenshot {path}"),
        Err(err) => log::error!("debug input: window screenshot failed: {err}"),
    }
}

fn dump_tree(w: &gtk::Widget, depth: usize) {
    log::warn!(
        "debug input: tree {:indent$}{} mapped={}",
        "",
        w.type_().name(),
        w.is_mapped(),
        indent = depth * 2
    );
    let mut c = w.first_child();
    while let Some(child) = c {
        dump_tree(&child, depth + 1);
        c = child.next_sibling();
    }
}

/// Log what the compositor-facing state of a window is (layer-shell role for
/// the quick terminal).
fn dump_window_state(win: &Window) {
    use gtk4_layer_shell::LayerShell;
    let gw: &gtk::Window = win.upcast_ref();
    let layer = gw.is_layer_window().then(|| {
        let edges = [
            gtk4_layer_shell::Edge::Left,
            gtk4_layer_shell::Edge::Right,
            gtk4_layer_shell::Edge::Top,
            gtk4_layer_shell::Edge::Bottom,
        ]
        .map(|e| gw.is_anchor(e));
        format!(
            "layer={:?} keyboard={:?} namespace={:?} anchors(l,r,t,b)={edges:?} exclusive={}",
            gw.layer(),
            gw.keyboard_mode(),
            gw.namespace(),
            gw.exclusive_zone()
        )
    });
    log::warn!(
        "debug input: window quick_terminal={} visible={} mapped={} active={} size={}x{} layer_shell={:?}",
        win.is_quick_terminal(),
        gw.is_visible(),
        gw.is_mapped(),
        gw.is_active(),
        gw.width(),
        gw.height(),
        layer
    );
}

fn collect_descendants<T: IsA<gtk::Widget>>(w: &gtk::Widget, out: &mut Vec<T>) {
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Ok(t) = child.clone().downcast::<T>() {
            out.push(t);
        }
        collect_descendants(&child, out);
        c = child.next_sibling();
    }
}

fn find_descendant<T: IsA<gtk::Widget>>(w: &gtk::Widget) -> Option<T> {
    if let Ok(t) = w.clone().downcast::<T>() {
        return Some(t);
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(t) = find_descendant(&child) {
            return Some(t);
        }
        c = child.next_sibling();
    }
    None
}

fn parse_mods(spec: &str) -> gdk::ModifierType {
    let mut state = gdk::ModifierType::empty();
    for m in spec.split('+').filter(|m| !m.is_empty()) {
        state |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => gdk::ModifierType::CONTROL_MASK,
            "shift" => gdk::ModifierType::SHIFT_MASK,
            "alt" => gdk::ModifierType::ALT_MASK,
            "super" | "meta" => gdk::ModifierType::SUPER_MASK,
            _ => gdk::ModifierType::empty(),
        };
    }
    state
}

/// Modifier state GDK reports while a button is held.
fn button_mask(button: u32) -> gdk::ModifierType {
    match button {
        1 => gdk::ModifierType::BUTTON1_MASK,
        2 => gdk::ModifierType::BUTTON2_MASK,
        _ => gdk::ModifierType::BUTTON3_MASK,
    }
}

impl Runner {
    /// The window the replay acts on: the application's active window, or,
    /// if that is hidden (e.g. an autohidden quick terminal that GTK still
    /// reports as active), the first visible Ghostty window.
    fn active_window(&self) -> Option<Window> {
        let active = self
            .app
            .active_window()
            .and_then(|w| w.downcast::<Window>().ok());
        if active
            .as_ref()
            .is_some_and(gtk::prelude::WidgetExt::is_visible)
        {
            return active;
        }
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<Window>().ok())
            .find(gtk::prelude::WidgetExt::is_visible)
            .or(active)
    }

    fn active_surface(&self) -> Option<Surface> {
        self.active_window()?.get_active_surface()
    }

    /// Run steps until the next `wait`, which reschedules us.
    fn run(self: &Rc<Self>) {
        loop {
            let idx = *self.next.borrow();
            let Some(step) = self.steps.get(idx).cloned() else {
                log::warn!("debug input: script finished");
                return;
            };
            *self.next.borrow_mut() += 1;
            log::warn!("debug input: {step:?}");
            match step {
                Step::Wait(ms) => {
                    let this = Rc::clone(self);
                    glib::timeout_add_local_once(Duration::from_millis(ms), move || this.run());
                    return;
                }
                Step::Quit => {
                    self.app.quit();
                    return;
                }
                Step::Key(spec) => self.key(&spec),
                Step::MouseMove(x, y, m) => {
                    self.pos.set((x, y));
                    if let Some(s) = self.active_surface() {
                        s.debug_mouse_move(x, y, parse_mods(&m));
                    }
                }
                Step::MousePress(b, m) => self.button(true, b, &m),
                Step::MouseRelease(b, m) => self.button(false, b, &m),
                Step::Click(b, m) => {
                    self.button(true, b, &m);
                    self.button(false, b, &m);
                }
                Step::Drag(x1, y1, x2, y2, m) => {
                    if let Some(s) = self.active_surface() {
                        let mods = parse_mods(&m);
                        self.pos.set((x1, y1));
                        s.debug_mouse_move(x1, y1, mods);
                        s.debug_mouse_button(true, 1, x1, y1, mods);
                        for i in 1..=10 {
                            let t = f64::from(i) / 10.0;
                            let (x, y) = (x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
                            self.pos.set((x, y));
                            s.debug_mouse_move(x, y, mods | gdk::ModifierType::BUTTON1_MASK);
                        }
                        s.debug_mouse_button(false, 1, x2, y2, mods);
                    }
                }
                Step::Scroll(_dx, dy) => {
                    if let Some(s) = self.active_surface() {
                        s.debug_scroll(dy);
                    }
                }
                Step::HScroll(dx) => {
                    if let Some(s) = self.active_surface() {
                        s.debug_hscroll(dx);
                    }
                }
                Step::TabsDump => {
                    if let Some(win) = self
                        .app
                        .active_window()
                        .and_then(|w| w.downcast::<Window>().ok())
                    {
                        let tv = win.tab_view();
                        let selected = tv.selected_page().map(|p| tv.page_position(&p));
                        log::warn!("debug input: tabs n={} selected={selected:?}", tv.n_pages());
                    }
                }
                Step::Resize(w, h) => {
                    if let Some(win) = self.app.active_window() {
                        win.set_default_size(w, h);
                    }
                }
                Step::ClipboardSet(text) => {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&text);
                    }
                }
                Step::WindowScreenshot(path) => {
                    if let Some(win) = self.app.active_window() {
                        window_screenshot(&win, &path);
                    }
                }
                Step::WindowsDump => {
                    if let Some(win) = self.app.active_window() {
                        dump_tree(win.upcast_ref(), 0);
                    }
                    for w in gtk::Window::list_toplevels() {
                        if let Some(win) = w.downcast_ref::<Window>() {
                            dump_window_state(win);
                        }
                        log::warn!(
                            "debug input: toplevel {} title={:?} visible={}",
                            w.type_().name(),
                            w.downcast_ref::<gtk::Window>()
                                .and_then(gtk::prelude::GtkWindowExt::title),
                            w.is_visible()
                        );
                    }
                }
                Step::ClipboardDump => {
                    if let Some(s) = self.active_surface() {
                        s.debug_clipboard_dump();
                    }
                }
                Step::ImePreedit(text) => {
                    if let Some(s) = self.active_surface() {
                        self.mock_im.set_preedit(&text);
                        if !s.debug_im_composing() {
                            s.debug_im(self.mock_im.upcast_ref(), DebugIm::PreeditStart);
                        }
                        s.debug_im(self.mock_im.upcast_ref(), DebugIm::PreeditChanged);
                    }
                }
                Step::ImePreeditEnd => {
                    if let Some(s) = self.active_surface() {
                        self.mock_im.set_preedit("");
                        s.debug_im(self.mock_im.upcast_ref(), DebugIm::PreeditEnd);
                    }
                }
                Step::ImeCommit(text) => {
                    if let Some(s) = self.active_surface() {
                        s.debug_im(self.mock_im.upcast_ref(), DebugIm::Commit(text));
                    }
                }
                Step::ImeState => {
                    if let Some(s) = self.active_surface() {
                        log::warn!("debug input: composing={}", s.debug_im_composing());
                    }
                }
                Step::Animations(on) => {
                    if let Some(settings) = gtk::Settings::default() {
                        settings.set_gtk_enable_animations(on);
                    }
                }
                Step::DialogInfo => self.dialog_info(),
                Step::QuickFocusOut => {
                    let quick = gtk::Window::list_toplevels()
                        .into_iter()
                        .filter_map(|w| w.downcast::<Window>().ok())
                        .find(Window::is_quick_terminal);
                    match quick {
                        // The window is not active, so this is exactly what
                        // GTK emits when a quick terminal loses focus.
                        Some(w) => w.notify("is-active"),
                        None => log::warn!("debug input: no quick terminal window"),
                    }
                }
                Step::DialogRemember => match self.find_dialog() {
                    Some(d) => {
                        if let Some(row) = find_descendant::<adw::SwitchRow>(d.upcast_ref()) {
                            row.set_active(true);
                        } else {
                            log::warn!("debug input: the dialog has no switch");
                        }
                    }
                    None => log::warn!("debug input: no dialog is open"),
                },
                Step::DialogClick(label) => self.dialog_click(&label),
                Step::ImeKey(mode, spec, text) => self.ime_key(&mode, &spec, &text),
                Step::Text(text) => match self.active_surface() {
                    Some(s) => s.debug_commit_text(&text),
                    None => log::warn!("debug input: no active surface"),
                },
                Step::Screenshot(path) => match self.active_surface() {
                    Some(s) => s.debug_screenshot(&path),
                    None => log::warn!("debug input: no active surface"),
                },
                Step::InspectorClick(x, y) => match self.active_surface() {
                    Some(s) => {
                        if !s.debug_inspector_click(x, y) {
                            log::warn!("debug input: the surface has no inspector");
                        }
                    }
                    None => log::warn!("debug input: no active surface"),
                },
                Step::InspectorScroll(x, y, dy) => match self.active_surface() {
                    Some(s) => {
                        if !s.debug_inspector_scroll(x, y, dy) {
                            log::warn!("debug input: the surface has no inspector");
                        }
                    }
                    None => log::warn!("debug input: no active surface"),
                },
                Step::InspectorScreenshot(path) => match self.active_surface() {
                    Some(s) => {
                        if !s.debug_inspector_screenshot(&path) {
                            log::warn!("debug input: the surface has no inspector");
                        }
                    }
                    None => log::warn!("debug input: no active surface"),
                },
            }
        }
    }

    fn button(&self, press: bool, button: u32, mods: &str) {
        let Some(s) = self.active_surface() else {
            return;
        };
        let (x, y) = self.pos.get();
        let mut m = parse_mods(mods);
        if !press {
            m |= button_mask(button);
        }
        s.debug_mouse_button(press, button, x, y, m);
    }

    /// The open alert dialog of the active window, if any.
    fn find_dialog(&self) -> Option<adw::AlertDialog> {
        find_descendant::<adw::AlertDialog>(self.app.active_window()?.upcast_ref())
    }

    fn dialog_info(&self) {
        let Some(d) = self.find_dialog() else {
            log::warn!("debug input: no dialog is open");
            return;
        };
        log::warn!(
            "debug input: dialog {} heading={:?} body={:?} remember_switch={}",
            d.type_().name(),
            d.heading(),
            d.body(),
            find_descendant::<adw::SwitchRow>(d.upcast_ref()).is_some()
        );
        let mut chain = Vec::new();
        let mut w = Some(d.clone().upcast::<gtk::Widget>());
        while let Some(x) = w {
            chain.push(format!("{}(mapped={})", x.type_().name(), x.is_mapped()));
            w = x.parent();
        }
        log::warn!(
            "debug input: dialog visible={} realized={} chain={}",
            d.is_visible(),
            d.is_realized(),
            chain.join(" < ")
        );
        for id in ["cancel", "ok", "close", "confirm", "deny"] {
            if d.has_response(id) {
                log::warn!(
                    "debug input: dialog response {id} label={:?}",
                    d.response_label(id)
                );
            }
        }
        if let Some(view) = find_descendant::<gtk::TextView>(d.upcast_ref()) {
            let buf = view.buffer();
            log::warn!(
                "debug input: dialog contents={:?}",
                buf.text(&buf.start_iter(), &buf.end_iter(), false)
            );
        }
    }

    /// Click the dialog's button labelled `label` (the real GtkButton, so the
    /// AdwAlertDialog response machinery runs as for a user click).
    fn dialog_click(&self, label: &str) {
        let Some(d) = self.find_dialog() else {
            log::warn!("debug input: no dialog is open");
            return;
        };
        let mut buttons = Vec::new();
        collect_descendants::<gtk::Button>(d.upcast_ref(), &mut buttons);
        match buttons
            .iter()
            .find(|b| b.label().is_some_and(|l| l.eq_ignore_ascii_case(label)))
        {
            Some(b) => b.emit_clicked(),
            None => log::warn!(
                "debug input: no dialog button {label:?}; have {:?}",
                buttons.iter().map(gtk::Button::label).collect::<Vec<_>>()
            ),
        }
    }

    fn ime_key(&self, mode: &str, spec: &str, text: &str) {
        let Some(surface) = self.active_surface() else {
            log::warn!("debug input: no active surface");
            return;
        };
        let (key, state) = match parse_key(spec) {
            Ok(v) => v,
            Err(err) => {
                log::error!("debug input: {err}");
                return;
            }
        };
        let Some(keycode) = surface
            .display()
            .map_keyval(key)
            .and_then(|keys| keys.first().map(gdk::KeymapKey::keycode))
        else {
            log::error!("debug input: no keycode for {spec:?}");
            return;
        };
        let sim = match mode {
            "preedit" => {
                self.mock_im.set_preedit(text);
                DebugImKey::Preedit
            }
            "commit" => {
                self.mock_im.set_preedit("");
                DebugImKey::Commit(text.to_owned())
            }
            "commit-simple" => {
                self.mock_im.set_preedit("");
                DebugImKey::CommitSimple(text.to_owned())
            }
            "handled-commit" => DebugImKey::HandledCommit(text.to_owned()),
            _ => DebugImKey::Pass,
        };
        let consumed = surface.debug_im_key(
            self.mock_im.upcast_ref(),
            &sim,
            key.into_glib(),
            keycode,
            state,
        );
        log::warn!(
            "debug input: ime-key consumed={consumed} composing={}",
            surface.debug_im_composing()
        );
    }

    fn key(&self, spec: &str) {
        let Some(surface) = self.active_surface() else {
            log::warn!("debug input: no active surface");
            return;
        };
        let (key, state) = match parse_key(spec) {
            Ok(v) => v,
            Err(err) => {
                log::error!("debug input: {err}");
                return;
            }
        };
        let display = surface.display();
        let Some(keycode) = display
            .map_keyval(key)
            .and_then(|keys| keys.first().map(gdk::KeymapKey::keycode))
        else {
            log::error!("debug input: no keycode for {spec:?}");
            return;
        };
        // What GTK reports as `keyval` to the key controller is the
        // translated keyval; the encode path translates again from keycode.
        let keyval = key.into_glib();
        surface.debug_key_event(ghostty_input::key::Action::Press, keyval, keycode, state);
        surface.debug_key_event(ghostty_input::key::Action::Release, keyval, keycode, state);
    }
}

/// A stand-in `GtkIMContext` whose preedit string and attributes the replay
/// controls; the surface's real signal handlers read it through
/// `preedit_string()` exactly as they would from an input method.
pub mod mock_im {
    use gtk::{glib, subclass::prelude::*};

    mod imp {
        use std::cell::RefCell;

        use gtk::{glib, pango, subclass::prelude::*};

        #[derive(Default)]
        pub struct MockIm {
            pub preedit: RefCell<String>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for MockIm {
            const NAME: &'static str = "GhosttyDebugMockIm";
            type Type = super::MockIm;
            type ParentType = gtk::IMContext;
        }

        impl ObjectImpl for MockIm {}

        impl IMContextImpl for MockIm {
            fn preedit_string(&self) -> (glib::GString, pango::AttrList, i32) {
                let text = self.preedit.borrow().clone();
                // What a real input method sends: an underline over the
                // whole preedit and the cursor at its end.
                let attrs = pango::AttrList::new();
                let mut underline = pango::AttrInt::new_underline(pango::Underline::Single);
                underline.set_start_index(0);
                underline.set_end_index(text.len() as u32);
                attrs.insert(underline);
                let cursor = text.chars().count() as i32;
                (text.as_str().into(), attrs, cursor)
            }
        }
    }

    glib::wrapper! {
        pub struct MockIm(ObjectSubclass<imp::MockIm>) @extends gtk::IMContext;
    }

    impl Default for MockIm {
        fn default() -> Self {
            glib::Object::new()
        }
    }

    impl MockIm {
        pub fn set_preedit(&self, text: &str) {
            self.imp().preedit.replace(text.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_script() {
        let s = "# c\n\nkey ctrl+shift+t\ntext hello world\nwait 200\nscreenshot /tmp/a.png\ninspector-screenshot /tmp/i.png\ninspector-click 10 20\ninspector-scroll 1 2 -3\nquit\n";
        assert_eq!(
            parse(s).expect("valid script"),
            vec![
                Step::Key("ctrl+shift+t".into()),
                Step::Text("hello world".into()),
                Step::Wait(200),
                Step::Screenshot("/tmp/a.png".into()),
                Step::InspectorScreenshot("/tmp/i.png".into()),
                Step::InspectorClick(10.0, 20.0),
                Step::InspectorScroll(1.0, 2.0, -3.0),
                Step::Quit
            ]
        );
        assert!(parse("wait x").is_err());
        assert!(parse("bogus").is_err());
    }

    #[test]
    fn parse_ime_steps() {
        let s = "ime-preedit ´a\nime-preedit-end\nime-commit á\nime-key preedit apostrophe ´\nime-key pass a\nime-state\n";
        assert_eq!(
            parse(s).expect("valid script"),
            vec![
                Step::ImePreedit("´a".into()),
                Step::ImePreeditEnd,
                Step::ImeCommit("á".into()),
                Step::ImeKey("preedit".into(), "apostrophe".into(), "´".into()),
                Step::ImeKey("pass".into(), "a".into(), String::new()),
                Step::ImeState,
            ]
        );
        assert_eq!(
            parse("dialog-info\ndialog-remember\ndialog-click Allow\n").expect("valid"),
            vec![
                Step::DialogInfo,
                Step::DialogRemember,
                Step::DialogClick("Allow".into())
            ]
        );
        assert!(parse("dialog-click").is_err());
        assert!(parse("ime-key bogus a").is_err());
        assert!(parse("ime-key commit").is_err());
    }

    #[test]
    fn parse_mouse_steps() {
        let s = "mouse-move 1 2\nmouse-move 1.5 2 ctrl+shift\nmouse-press 1\nmouse-release 3 alt\nclick 2\ndrag 1 2 3 4\nscroll 0 -3\nresize 600 400\nclipboard-set a\\nb\nclipboard-dump\nwindows-dump\nwindow-screenshot /tmp/w.png\n";
        assert_eq!(
            parse(s).expect("valid script"),
            vec![
                Step::MouseMove(1.0, 2.0, String::new()),
                Step::MouseMove(1.5, 2.0, "ctrl+shift".into()),
                Step::MousePress(1, String::new()),
                Step::MouseRelease(3, "alt".into()),
                Step::Click(2, String::new()),
                Step::Drag(1.0, 2.0, 3.0, 4.0, String::new()),
                Step::Scroll(0.0, -3.0),
                Step::Resize(600, 400),
                Step::ClipboardSet("a\nb".into()),
                Step::ClipboardDump,
                Step::WindowsDump,
                Step::WindowScreenshot("/tmp/w.png".into()),
            ]
        );
        assert_eq!(
            parse("hscroll 130\ntabs-dump").expect("valid"),
            vec![Step::HScroll(130.0), Step::TabsDump]
        );
        assert!(parse("hscroll").is_err());
        assert!(parse("mouse-move 1").is_err());
        assert!(parse("drag 1 2 3").is_err());
        assert!(parse("click x").is_err());
        assert!(parse("scroll 1").is_err());
    }
}
