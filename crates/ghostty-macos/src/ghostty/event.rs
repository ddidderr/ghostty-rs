//! `Ghostty.Event.swift`.

use objc2_app_kit::{NSEvent, NSEventModifierFlags};

/// A comparable event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComparableKeyEvent {
    pub key_code: u16,
    pub flags: NSEventModifierFlags,
}

impl ComparableKeyEvent {
    #[must_use]
    pub fn new(event: &NSEvent) -> Self {
        Self {
            key_code: event.keyCode(),
            flags: event.modifierFlags(),
        }
    }
}
