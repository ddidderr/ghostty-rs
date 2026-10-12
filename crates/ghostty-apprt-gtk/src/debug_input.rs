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
//! quit                    # quit the application
//! ```
//!
//! Key names are GDK keysym names (`t`, `Return`, `Tab`, `equal`, `Up`, `F5`)
//! with a few lowercase aliases (`enter`, `esc`, `up`, ...). Commands act on
//! the active surface of the active window at the time they run.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::class::application::Application;
use crate::class::surface::Surface;
use crate::class::window::Window;

/// One parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Key(String),
    Text(String),
    Wait(u64),
    Screenshot(String),
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
}

impl Runner {
    fn active_surface(&self) -> Option<Surface> {
        self.app
            .active_window()?
            .downcast::<Window>()
            .ok()?
            .get_active_surface()
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
                Step::Text(text) => match self.active_surface() {
                    Some(s) => s.debug_commit_text(&text),
                    None => log::warn!("debug input: no active surface"),
                },
                Step::Screenshot(path) => match self.active_surface() {
                    Some(s) => s.debug_screenshot(&path),
                    None => log::warn!("debug input: no active surface"),
                },
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_script() {
        let s =
            "# c\n\nkey ctrl+shift+t\ntext hello world\nwait 200\nscreenshot /tmp/a.png\nquit\n";
        assert_eq!(
            parse(s).expect("valid script"),
            vec![
                Step::Key("ctrl+shift+t".into()),
                Step::Text("hello world".into()),
                Step::Wait(200),
                Step::Screenshot("/tmp/a.png".into()),
                Step::Quit
            ]
        );
        assert!(parse("wait x").is_err());
        assert!(parse("bogus").is_err());
    }
}
