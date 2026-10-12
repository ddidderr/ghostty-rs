//! Helpers to name things like Zig's `@tagName`.

use dear_imgui_rs::Key as ImKey;

/// The `snake_case` form of the `Debug` output of a field-less enum value,
/// which is what Zig's `@tagName` returns for the equivalent enum.
pub fn tag_name(v: &dyn std::fmt::Debug) -> String {
    let dbg = format!("{v:?}");
    // Only take the variant name (drop payloads for data-carrying enums).
    let name = dbg
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    snake_case(name)
}

/// Convert `CamelCase` to `snake_case` (`SgrPixels` -> `sgr_pixels`,
/// `X10` -> `x10`).
pub fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_lowercase();
        }
    }
    out
}

/// Returns the Dear ImGui key for a Ghostty key (Zig: `Key.imguiKey`), or
/// `None` for keys that have no ImGui equivalent.
pub fn imgui_key(key: ghostty_input::Key) -> Option<ImKey> {
    use dear_imgui_rs::Key as I;
    use ghostty_input::Key as K;
    Some(match key {
        K::KeyA => I::A,
        K::KeyB => I::B,
        K::KeyC => I::C,
        K::KeyD => I::D,
        K::KeyE => I::E,
        K::KeyF => I::F,
        K::KeyG => I::G,
        K::KeyH => I::H,
        K::KeyI => I::I,
        K::KeyJ => I::J,
        K::KeyK => I::K,
        K::KeyL => I::L,
        K::KeyM => I::M,
        K::KeyN => I::N,
        K::KeyO => I::O,
        K::KeyP => I::P,
        K::KeyQ => I::Q,
        K::KeyR => I::R,
        K::KeyS => I::S,
        K::KeyT => I::T,
        K::KeyU => I::U,
        K::KeyV => I::V,
        K::KeyW => I::W,
        K::KeyX => I::X,
        K::KeyY => I::Y,
        K::KeyZ => I::Z,
        K::Digit0 => I::Key0,
        K::Numpad0 => I::Keypad0,
        K::Digit1 => I::Key1,
        K::Numpad1 => I::Keypad1,
        K::Digit2 => I::Key2,
        K::Numpad2 => I::Keypad2,
        K::Digit3 => I::Key3,
        K::Numpad3 => I::Keypad3,
        K::Digit4 => I::Key4,
        K::Numpad4 => I::Keypad4,
        K::Digit5 => I::Key5,
        K::Numpad5 => I::Keypad5,
        K::Digit6 => I::Key6,
        K::Numpad6 => I::Keypad6,
        K::Digit7 => I::Key7,
        K::Numpad7 => I::Keypad7,
        K::Digit8 => I::Key8,
        K::Numpad8 => I::Keypad8,
        K::Digit9 => I::Key9,
        K::Numpad9 => I::Keypad9,
        K::F1 => I::F1,
        K::F2 => I::F2,
        K::F3 => I::F3,
        K::F4 => I::F4,
        K::F5 => I::F5,
        K::F6 => I::F6,
        K::F7 => I::F7,
        K::F8 => I::F8,
        K::F9 => I::F9,
        K::F10 => I::F10,
        K::F11 => I::F11,
        K::F12 => I::F12,
        K::Semicolon => I::Semicolon,
        K::Space => I::Space,
        K::Quote => I::Apostrophe,
        K::Comma => I::Comma,
        K::Backquote => I::GraveAccent,
        K::Period => I::Period,
        K::Slash => I::Slash,
        K::Minus => I::Minus,
        K::Equal => I::Equal,
        K::BracketLeft => I::LeftBracket,
        K::BracketRight => I::RightBracket,
        K::Backslash => I::Backslash,
        K::ArrowUp => I::UpArrow,
        K::ArrowDown => I::DownArrow,
        K::ArrowLeft => I::LeftArrow,
        K::ArrowRight => I::RightArrow,
        K::Home => I::Home,
        K::End => I::End,
        K::Insert => I::Insert,
        K::Delete => I::Delete,
        K::CapsLock => I::CapsLock,
        K::ScrollLock => I::ScrollLock,
        K::NumLock => I::NumLock,
        K::PageUp => I::PageUp,
        K::PageDown => I::PageDown,
        K::Escape => I::Escape,
        K::Enter => I::Enter,
        K::Tab => I::Tab,
        K::Backspace => I::Backspace,
        K::PrintScreen => I::PrintScreen,
        K::Pause => I::Pause,
        K::ContextMenu => I::Menu,
        K::NumpadDecimal => I::KeypadDecimal,
        K::NumpadDivide => I::KeypadDivide,
        K::NumpadMultiply => I::KeypadMultiply,
        K::NumpadSubtract => I::KeypadSubtract,
        K::NumpadAdd => I::KeypadAdd,
        K::NumpadEnter => I::KeypadEnter,
        K::NumpadEqual => I::KeypadEqual,
        K::NumpadLeft => I::LeftArrow,
        K::NumpadRight => I::RightArrow,
        K::NumpadUp => I::UpArrow,
        K::NumpadDown => I::DownArrow,
        K::NumpadPageUp => I::PageUp,
        K::NumpadPageDown => I::PageUp,
        K::NumpadHome => I::Home,
        K::NumpadEnd => I::End,
        K::NumpadInsert => I::Insert,
        K::NumpadDelete => I::Delete,
        K::NumpadBegin => I::Tab,
        K::ShiftLeft => I::LeftShift,
        K::ControlLeft => I::LeftCtrl,
        K::AltLeft => I::LeftAlt,
        K::MetaLeft => I::LeftSuper,
        K::ShiftRight => I::RightShift,
        K::ControlRight => I::RightCtrl,
        K::AltRight => I::RightAlt,
        K::MetaRight => I::RightSuper,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_names() {
        assert_eq!(snake_case("SgrPixels"), "sgr_pixels");
        assert_eq!(snake_case("X10"), "x10");
        assert_eq!(snake_case("Utf8"), "utf8");
        assert_eq!(snake_case("Active"), "active");
        assert_eq!(tag_name(&Some(3)), "some");
    }

    #[test]
    fn imgui_key_mapping() {
        use ghostty_input::Key;
        assert_eq!(imgui_key(Key::KeyA), Some(ImKey::A));
        assert_eq!(imgui_key(Key::Digit5), Some(ImKey::Key5));
        assert_eq!(imgui_key(Key::ArrowUp), Some(ImKey::UpArrow));
        assert_eq!(imgui_key(Key::F13), None);
        assert_eq!(imgui_key(Key::NumpadPageDown), Some(ImKey::PageUp));
    }
}
