//! GDK key <-> Ghostty input translation (port of `apprt/gtk/key.zig`).

use std::fmt::Write as _;

use ghostty_input::binding::{Trigger, TriggerKey};
use ghostty_input::key::{Action, Key as K};
use ghostty_input::key_mods::{ModSide, Mods};
use gtk::gdk;
use gtk::gdk::prelude::DeviceExt;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;

/// Name of a keyval (`gdk.keyvalName`).
fn keyval_name(keyval: u32) -> Option<String> {
    #[allow(unsafe_code)]
    // SAFETY: `gdk::Key` is a transparent newtype over the guint keyval;
    // every u32 is a valid value (unknown keyvals simply have no name).
    let key = unsafe { <gdk::Key as gtk::glib::translate::FromGlib<u32>>::from_glib(keyval) };
    key.name().map(|s| s.to_string())
}

/// Returns a GTK accelerator string from a trigger, or None if the trigger
/// can't be represented (e.g. catch_all or unmapped key).
pub fn accel_from_trigger(trigger: Trigger) -> Option<String> {
    let mut s = String::new();
    if trigger.mods.shift {
        s.push_str("<Shift>");
    }
    if trigger.mods.ctrl {
        s.push_str("<Ctrl>");
    }
    if trigger.mods.alt {
        s.push_str("<Alt>");
    }
    if trigger.mods.super_ {
        s.push_str("<Super>");
    }
    write_trigger_key(&mut s, trigger).then_some(s)
}

/// Returns a XDG-compliant shortcuts string from a trigger.
/// Spec: <https://specifications.freedesktop.org/shortcuts-spec/latest/>
pub fn xdg_shortcut_from_trigger(trigger: Trigger) -> Option<String> {
    let mut s = String::new();
    if trigger.mods.shift {
        s.push_str("SHIFT+");
    }
    if trigger.mods.ctrl {
        s.push_str("CTRL+");
    }
    if trigger.mods.alt {
        s.push_str("ALT+");
    }
    if trigger.mods.super_ {
        s.push_str("LOGO+");
    }
    // NOTE: the spec expects libxkbcommon keysyms; GTK's are identical to
    // X11's, which works for now (same assumption as upstream).
    write_trigger_key(&mut s, trigger).then_some(s)
}

fn write_trigger_key(w: &mut String, trigger: Trigger) -> bool {
    match trigger.key {
        TriggerKey::Physical(k) => {
            let Some(keyval) = keyval_from_key(k) else {
                return false;
            };
            let Some(name) = keyval_name(keyval) else {
                return false;
            };
            w.push_str(&name);
        }
        TriggerKey::Unicode(cp) => {
            if let Some(name) = keyval_name(cp) {
                w.push_str(&name);
            } else if let Some(c) = char::from_u32(cp) {
                w.push(c);
            }
        }
        TriggerKey::CatchAll => return false,
    }
    true
}

pub fn translate_mods(state: gdk::ModifierType) -> Mods {
    Mods {
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        alt: state.contains(gdk::ModifierType::ALT_MASK),
        super_: state.contains(gdk::ModifierType::SUPER_MASK),
        // Lock is dependent on the X settings but we just assume caps lock.
        caps_lock: state.contains(gdk::ModifierType::LOCK_MASK),
        ..Mods::default()
    }
}

/// Get the unshifted unicode value of the keyval. This is used by the Kitty
/// keyboard protocol. Returns 0 if unknown.
pub fn keyval_unicode_unshifted(
    widget: &impl IsA<gtk::Widget>,
    event: &gdk::KeyEvent,
    keycode: u32,
) -> u32 {
    // We need the currently active keyboard layout so we know what group to
    // look at.
    keyval_unicode_unshifted_for_layout(widget, event.layout(), keycode)
}

/// Like [`keyval_unicode_unshifted`] with an explicit keyboard layout (group)
/// instead of a `GdkKeyEvent`; used by the debug input replay.
pub fn keyval_unicode_unshifted_for_layout(
    widget: &impl IsA<gtk::Widget>,
    layout: u32,
    keycode: u32,
) -> u32 {
    let display = widget.as_ref().display();

    // All possible keyboard mappings for this keycode (the physical key).
    let Some(entries) = display.map_keycode(keycode) else {
        return 0;
    };

    for (key, keyval) in entries {
        if key.group() == layout.cast_signed() && key.level() == 0 {
            return keyval.to_unicode().map_or(0, |c| c as u32);
        }
    }

    0
}

/// Returns the mods to use for a key event from a GTK event. This requires
/// a lot of context because the GdkEvent doesn't contain enough on its own.
///
/// `winproto_event_mods` is `winproto::App::event_mods(device, gtk_mods)`.
pub fn event_mods(
    event: &gdk::Event,
    physical_key: K,
    gtk_mods: gdk::ModifierType,
    action: Action,
    winproto_event_mods: impl FnOnce(Option<&gdk::Device>, gdk::ModifierType) -> Mods,
) -> Mods {
    event_mods_for_device(
        event.device().as_ref(),
        physical_key,
        gtk_mods,
        action,
        winproto_event_mods,
    )
}

/// Like [`event_mods`] but with the (optional) device instead of the event.
pub fn event_mods_for_device(
    device: Option<&gdk::Device>,
    physical_key: K,
    gtk_mods: gdk::ModifierType,
    action: Action,
    winproto_event_mods: impl FnOnce(Option<&gdk::Device>, gdk::ModifierType) -> Mods,
) -> Mods {
    let mut mods = winproto_event_mods(device, gtk_mods);
    mods.num_lock = device.is_some_and(DeviceExt::is_num_locked);

    // We use the physical key to determine sided modifiers. As far as I can
    // tell there's no other way to reliably determine this.
    //
    // We also set the main modifier to true if either side is true, since on
    // both X11/Wayland, GTK doesn't set the main modifier if only the
    // modifier key is pressed, but our core logic relies on it.
    let down = action != Action::Release;
    match physical_key {
        K::ShiftLeft => {
            mods.shift = down;
            mods.sides.shift = ModSide::Left;
        }
        K::ShiftRight => {
            mods.shift = down;
            mods.sides.shift = ModSide::Right;
        }
        K::ControlLeft => {
            mods.ctrl = down;
            mods.sides.ctrl = ModSide::Left;
        }
        K::ControlRight => {
            mods.ctrl = down;
            mods.sides.ctrl = ModSide::Right;
        }
        K::AltLeft => {
            mods.alt = down;
            mods.sides.alt = ModSide::Left;
        }
        K::AltRight => {
            mods.alt = down;
            mods.sides.alt = ModSide::Right;
        }
        K::MetaLeft => {
            mods.super_ = down;
            mods.sides.super_ = ModSide::Left;
        }
        K::MetaRight => {
            mods.super_ = down;
            mods.sides.super_ = ModSide::Right;
        }
        _ => {}
    }

    mods
}

/// Returns an input key from a keyval or None if we don't have a mapping.
pub fn key_from_keyval(keyval: u32) -> Option<K> {
    KEYMAP
        .iter()
        .find(|(g, _)| g.into_glib() == keyval)
        .map(|(_, k)| *k)
}

/// Returns a keyval from an input key or None if we don't have a mapping.
pub fn keyval_from_key(key: K) -> Option<u32> {
    KEYMAP
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(g, _)| g.into_glib())
}

/// Converts a trigger to a human-readable label for display in UI, using
/// GTK accelerator-style formatting (e.g. "Ctrl+Shift+A"). Returns None if
/// the trigger cannot be formatted (e.g. catch_all).
pub fn label_from_trigger(trigger: Trigger) -> Option<String> {
    let mut s = String::new();
    if trigger.mods.super_ {
        s.push_str("Super+");
    }
    if trigger.mods.ctrl {
        s.push_str("Ctrl+");
    }
    if trigger.mods.alt {
        s.push_str("Alt+");
    }
    if trigger.mods.shift {
        s.push_str("Shift+");
    }
    write_trigger_key_label(&mut s, trigger).then_some(s)
}

fn push_capitalized(w: &mut String, name: &str) {
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_lowercase() {
            w.push(first.to_ascii_uppercase());
            w.push_str(chars.as_str());
        } else {
            w.push_str(name);
        }
    }
}

/// Writes the key portion of a trigger in human-readable format.
fn write_trigger_key_label(w: &mut String, trigger: Trigger) -> bool {
    match trigger.key {
        TriggerKey::Physical(k) => {
            let Some(keyval) = keyval_from_key(k) else {
                return false;
            };
            let Some(name) = keyval_name(keyval) else {
                return false;
            };
            push_capitalized(w, &name);
        }
        TriggerKey::Unicode(cp) => {
            if let Some(name) = keyval_name(cp) {
                push_capitalized(w, &name);
            } else if let Some(c) = char::from_u32(cp) {
                let _ = write!(w, "{c}");
            }
        }
        TriggerKey::CatchAll => return false,
    }
    true
}

/// The keymap: mappings between GDK keyvals and our own key enum.
// TODO: media keys
const KEYMAP: &[(gdk::Key, K)] = &[
    (gdk::Key::a, K::KeyA),
    (gdk::Key::b, K::KeyB),
    (gdk::Key::c, K::KeyC),
    (gdk::Key::d, K::KeyD),
    (gdk::Key::e, K::KeyE),
    (gdk::Key::f, K::KeyF),
    (gdk::Key::g, K::KeyG),
    (gdk::Key::h, K::KeyH),
    (gdk::Key::i, K::KeyI),
    (gdk::Key::j, K::KeyJ),
    (gdk::Key::k, K::KeyK),
    (gdk::Key::l, K::KeyL),
    (gdk::Key::m, K::KeyM),
    (gdk::Key::n, K::KeyN),
    (gdk::Key::o, K::KeyO),
    (gdk::Key::p, K::KeyP),
    (gdk::Key::q, K::KeyQ),
    (gdk::Key::r, K::KeyR),
    (gdk::Key::s, K::KeyS),
    (gdk::Key::t, K::KeyT),
    (gdk::Key::u, K::KeyU),
    (gdk::Key::v, K::KeyV),
    (gdk::Key::w, K::KeyW),
    (gdk::Key::x, K::KeyX),
    (gdk::Key::y, K::KeyY),
    (gdk::Key::z, K::KeyZ),
    (gdk::Key::_0, K::Digit0),
    (gdk::Key::_1, K::Digit1),
    (gdk::Key::_2, K::Digit2),
    (gdk::Key::_3, K::Digit3),
    (gdk::Key::_4, K::Digit4),
    (gdk::Key::_5, K::Digit5),
    (gdk::Key::_6, K::Digit6),
    (gdk::Key::_7, K::Digit7),
    (gdk::Key::_8, K::Digit8),
    (gdk::Key::_9, K::Digit9),
    (gdk::Key::semicolon, K::Semicolon),
    (gdk::Key::space, K::Space),
    (gdk::Key::apostrophe, K::Quote),
    (gdk::Key::comma, K::Comma),
    (gdk::Key::grave, K::Backquote),
    (gdk::Key::period, K::Period),
    (gdk::Key::slash, K::Slash),
    (gdk::Key::minus, K::Minus),
    (gdk::Key::equal, K::Equal),
    (gdk::Key::bracketleft, K::BracketLeft),
    (gdk::Key::bracketright, K::BracketRight),
    (gdk::Key::backslash, K::Backslash),
    (gdk::Key::Up, K::ArrowUp),
    (gdk::Key::Down, K::ArrowDown),
    (gdk::Key::Right, K::ArrowRight),
    (gdk::Key::Left, K::ArrowLeft),
    (gdk::Key::Home, K::Home),
    (gdk::Key::End, K::End),
    (gdk::Key::Insert, K::Insert),
    (gdk::Key::Delete, K::Delete),
    (gdk::Key::Caps_Lock, K::CapsLock),
    (gdk::Key::Scroll_Lock, K::ScrollLock),
    (gdk::Key::Num_Lock, K::NumLock),
    (gdk::Key::Page_Up, K::PageUp),
    (gdk::Key::Page_Down, K::PageDown),
    (gdk::Key::Escape, K::Escape),
    (gdk::Key::Return, K::Enter),
    (gdk::Key::Tab, K::Tab),
    (gdk::Key::BackSpace, K::Backspace),
    (gdk::Key::Print, K::PrintScreen),
    (gdk::Key::Pause, K::Pause),
    (gdk::Key::F1, K::F1),
    (gdk::Key::F2, K::F2),
    (gdk::Key::F3, K::F3),
    (gdk::Key::F4, K::F4),
    (gdk::Key::F5, K::F5),
    (gdk::Key::F6, K::F6),
    (gdk::Key::F7, K::F7),
    (gdk::Key::F8, K::F8),
    (gdk::Key::F9, K::F9),
    (gdk::Key::F10, K::F10),
    (gdk::Key::F11, K::F11),
    (gdk::Key::F12, K::F12),
    (gdk::Key::F13, K::F13),
    (gdk::Key::F14, K::F14),
    (gdk::Key::F15, K::F15),
    (gdk::Key::F16, K::F16),
    (gdk::Key::F17, K::F17),
    (gdk::Key::F18, K::F18),
    (gdk::Key::F19, K::F19),
    (gdk::Key::F20, K::F20),
    (gdk::Key::F21, K::F21),
    (gdk::Key::F22, K::F22),
    (gdk::Key::F23, K::F23),
    (gdk::Key::F24, K::F24),
    (gdk::Key::F25, K::F25),
    (gdk::Key::KP_0, K::Numpad0),
    (gdk::Key::KP_1, K::Numpad1),
    (gdk::Key::KP_2, K::Numpad2),
    (gdk::Key::KP_3, K::Numpad3),
    (gdk::Key::KP_4, K::Numpad4),
    (gdk::Key::KP_5, K::Numpad5),
    (gdk::Key::KP_6, K::Numpad6),
    (gdk::Key::KP_7, K::Numpad7),
    (gdk::Key::KP_8, K::Numpad8),
    (gdk::Key::KP_9, K::Numpad9),
    (gdk::Key::KP_Decimal, K::NumpadDecimal),
    (gdk::Key::KP_Divide, K::NumpadDivide),
    (gdk::Key::KP_Multiply, K::NumpadMultiply),
    (gdk::Key::KP_Subtract, K::NumpadSubtract),
    (gdk::Key::KP_Add, K::NumpadAdd),
    (gdk::Key::KP_Enter, K::NumpadEnter),
    (gdk::Key::KP_Equal, K::NumpadEqual),
    (gdk::Key::KP_Separator, K::NumpadSeparator),
    (gdk::Key::KP_Left, K::NumpadLeft),
    (gdk::Key::KP_Right, K::NumpadRight),
    (gdk::Key::KP_Up, K::NumpadUp),
    (gdk::Key::KP_Down, K::NumpadDown),
    (gdk::Key::KP_Page_Up, K::NumpadPageUp),
    (gdk::Key::KP_Page_Down, K::NumpadPageDown),
    (gdk::Key::KP_Home, K::NumpadHome),
    (gdk::Key::KP_End, K::NumpadEnd),
    (gdk::Key::KP_Insert, K::NumpadInsert),
    (gdk::Key::KP_Delete, K::NumpadDelete),
    (gdk::Key::KP_Begin, K::NumpadBegin),
    (gdk::Key::Copy, K::Copy),
    (gdk::Key::Cut, K::Cut),
    (gdk::Key::Paste, K::Paste),
    (gdk::Key::Shift_L, K::ShiftLeft),
    (gdk::Key::Control_L, K::ControlLeft),
    (gdk::Key::Alt_L, K::AltLeft),
    (gdk::Key::Super_L, K::MetaLeft),
    (gdk::Key::Shift_R, K::ShiftRight),
    (gdk::Key::Control_R, K::ControlRight),
    (gdk::Key::Alt_R, K::AltRight),
    (gdk::Key::Super_R, K::MetaRight),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn uni(mods: Mods, cp: u32) -> Trigger {
        Trigger {
            mods,
            key: TriggerKey::Unicode(cp),
        }
    }

    fn all_mods() -> Mods {
        Mods {
            ctrl: true,
            alt: true,
            super_: true,
            shift: true,
            ..Mods::default()
        }
    }

    #[test]
    fn accel_from_trigger_test() {
        let m = Mods {
            super_: true,
            ..Mods::default()
        };
        assert_eq!(
            accel_from_trigger(uni(m, 'q' as u32)).as_deref(),
            Some("<Super>q")
        );
        assert_eq!(
            accel_from_trigger(uni(all_mods(), 92)).as_deref(),
            Some("<Shift><Ctrl><Alt><Super>backslash")
        );
    }

    #[test]
    fn xdg_shortcut_from_trigger_test() {
        let m = Mods {
            super_: true,
            ..Mods::default()
        };
        assert_eq!(
            xdg_shortcut_from_trigger(uni(m, 'q' as u32)).as_deref(),
            Some("LOGO+q")
        );
        assert_eq!(
            xdg_shortcut_from_trigger(uni(all_mods(), 92)).as_deref(),
            Some("SHIFT+CTRL+ALT+LOGO+backslash")
        );
    }

    #[test]
    fn label_from_trigger_test() {
        let m = Mods {
            super_: true,
            ..Mods::default()
        };
        assert_eq!(
            label_from_trigger(uni(m, 'q' as u32)).as_deref(),
            Some("Super+Q")
        );
        assert_eq!(
            label_from_trigger(uni(all_mods(), 92)).as_deref(),
            Some("Super+Ctrl+Alt+Shift+Backslash")
        );
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            label_from_trigger(Trigger {
                mods: ctrl,
                key: TriggerKey::Physical(K::KeyA)
            })
            .as_deref(),
            Some("Ctrl+A")
        );
        assert_eq!(
            label_from_trigger(Trigger {
                mods: Mods::default(),
                key: TriggerKey::Physical(K::Escape)
            })
            .as_deref(),
            Some("Escape")
        );
        assert_eq!(
            label_from_trigger(Trigger {
                mods: Mods::default(),
                key: TriggerKey::CatchAll
            }),
            None
        );
    }

    #[test]
    fn keymap_roundtrip() {
        for (g, k) in KEYMAP {
            assert_eq!(key_from_keyval(g.into_glib()), Some(*k));
        }
        assert_eq!(keyval_from_key(K::KeyA), Some(gdk::Key::a.into_glib()));
        assert_eq!(keyval_from_key(K::Unidentified), None);
    }

    #[test]
    fn translate_mods_test() {
        let m = translate_mods(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::LOCK_MASK);
        assert!(m.shift && m.caps_lock && !m.ctrl && !m.alt && !m.super_);
    }
}
