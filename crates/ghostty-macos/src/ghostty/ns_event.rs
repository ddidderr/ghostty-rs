//! `NSEvent+Extension.swift`: `NSEvent` -> Ghostty key event conversions.
#![allow(unsafe_code)]

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};

use super::{
    capi::{ghostty_input_action_e, ghostty_input_key_s},
    input::ghostty_mods,
};

/// Create a Ghostty key event for a given keyboard action.
///
/// This will not set the "text" or "composing" fields since these can't safely be set
/// with the information or lifetimes given.
///
/// The translationMods should be set to the modifiers used for actual character
/// translation if available.
#[must_use]
pub fn ghostty_key_event(
    event: &NSEvent,
    action: ghostty_input_action_e,
    translation_mods: Option<NSEventModifierFlags>,
) -> ghostty_input_key_s {
    let modifier_flags = event.modifierFlags();
    let mut key_ev = ghostty_input_key_s {
        action,
        mods: 0,
        consumed_mods: 0,
        keycode: u32::from(event.keyCode()),
        // We can't infer or set these safely from this method. Since text is
        // a cString, we can't use self.characters because of garbage collection.
        // We have to let the caller handle this.
        text: std::ptr::null(),
        unshifted_codepoint: 0,
        composing: false,
    };

    // macOS provides no easy way to determine the consumed modifiers for
    // producing text. We apply a simple heuristic here that has worked for years
    // so far: control and command never contribute to the translation of text,
    // assume everything else did.
    key_ev.mods = ghostty_mods(modifier_flags.0 as u64);
    let consumed = translation_mods.unwrap_or(modifier_flags)
        & !(NSEventModifierFlags::Control | NSEventModifierFlags::Command);
    key_ev.consumed_mods = ghostty_mods(consumed.0 as u64);

    // Our unshifted codepoint is the codepoint with no modifiers. We
    // ignore multi-codepoint values. We have to use `byApplyingModifiers`
    // instead of `charactersIgnoringModifiers` because the latter changes
    // behavior with ctrl pressed and we don't want any of that.
    key_ev.unshifted_codepoint = 0;
    let ty = event.r#type();
    if ty == NSEventType::KeyDown || ty == NSEventType::KeyUp {
        let chars = event.charactersByApplyingModifiers(NSEventModifierFlags::empty());
        if let Some(c) = chars.and_then(|chars| chars.to_string().chars().next()) {
            key_ev.unshifted_codepoint = c as u32;
        }
    }

    key_ev
}

/// Returns the text to set for a key event for Ghostty.
///
/// This namely contains logic to avoid control characters, since we handle control character
/// mapping manually within Ghostty.
#[must_use]
pub fn ghostty_characters(event: &NSEvent) -> Option<String> {
    // If we have no characters associated with this event we do nothing.
    let characters = event.characters()?.to_string();
    ghostty_characters_for(&characters, || {
        event
            .charactersByApplyingModifiers(event.modifierFlags() & !NSEventModifierFlags::Control)
            .map(|s| s.to_string())
    })
}

/// The pure part of [`ghostty_characters`]; `without_control` computes
/// `characters(byApplyingModifiers: modifierFlags.subtracting(.control))`.
fn ghostty_characters_for(
    characters: &str,
    without_control: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let mut it = characters.chars();
    if let (Some(scalar), None) = (it.next(), it.next()) {
        // If we have a single control character, then we return the characters
        // without control pressed. We do this because we handle control character
        // encoding directly within Ghostty's KeyEncoder.
        if (scalar as u32) < 0x20 {
            return without_control();
        }

        // If we have a single value in the PUA, then it's a function key and
        // we don't want to send PUA ranges down to Ghostty.
        if (0xF700..=0xF8FF).contains(&(scalar as u32)) {
            return None;
        }
    }

    Some(characters.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Not run on Linux (apple-only module); kept as documentation of intent.
    #[test]
    fn characters_rules() {
        assert_eq!(ghostty_characters_for("a", || None).as_deref(), Some("a"));
        assert_eq!(
            ghostty_characters_for("\u{3}", || Some("c".into())).as_deref(),
            Some("c")
        );
        assert_eq!(ghostty_characters_for("\u{F700}", || None), None);
        assert_eq!(ghostty_characters_for("ab", || None).as_deref(), Some("ab"));
    }
}
