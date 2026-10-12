//! Key event C API (ZIG/src/terminal/c/key_event.zig, `include/ghostty/vt/key/event.h`).
#![allow(unsafe_code)]

use std::{ffi::c_int, ptr::NonNull};

use ghostty_input::{Action, Key, KeyEvent, Mods};

use crate::{
    alloc::{Alloc, Allocator},
    result::GhosttyResult,
};

/// C: `GhosttyKeyEvent` (opaque). Mirrors Zig's `KeyEventWrapper`.
///
/// The UTF-8 text is not owned by the wrapper: the caller is responsible for
/// keeping the bytes passed to `ghostty_key_event_set_utf8` alive.
pub struct KeyEventWrapper {
    alloc: Alloc,
    action: Action,
    key: Key,
    mods: Mods,
    consumed_mods: Mods,
    composing: bool,
    utf8: *const u8,
    utf8_len: usize,
    unshifted_codepoint: u32,
}

impl KeyEventWrapper {
    /// Borrow as the Rust [`KeyEvent`]. `utf8` is a byte slice (as in Zig) and is
    /// passed through unchanged, even if it is not valid UTF-8.
    ///
    /// # Safety
    /// The caller-provided UTF-8 bytes must still be alive.
    pub(crate) unsafe fn as_event(&self) -> KeyEvent<'_> {
        let bytes: &[u8] = if self.utf8.is_null() {
            &[]
        } else {
            // SAFETY: per the contract of `ghostty_key_event_set_utf8`.
            unsafe { std::slice::from_raw_parts(self.utf8, self.utf8_len) }
        };
        KeyEvent {
            action: self.action,
            key: self.key,
            mods: self.mods,
            consumed_mods: self.consumed_mods,
            composing: self.composing,
            utf8: bytes,
            unshifted_codepoint: self.unshifted_codepoint,
        }
    }
}

/// C: `GhosttyKeyAction`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum KeyAction {
    Release = 0,
    Press = 1,
    Repeat = 2,
}

fn action_from_c(v: c_int) -> Option<Action> {
    match v {
        0 => Some(Action::Release),
        1 => Some(Action::Press),
        2 => Some(Action::Repeat),
        _ => None,
    }
}

fn key_from_c(v: c_int) -> Option<Key> {
    usize::try_from(v)
        .ok()
        .and_then(|i| Key::ALL.get(i).copied())
}

/// # Safety
/// `allocator` must be null or valid; `event` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_new(
    allocator: *const Allocator,
    event: *mut *mut KeyEventWrapper,
) -> GhosttyResult {
    // SAFETY: forwarded caller contract.
    let alloc = unsafe { Alloc::from_c(allocator) };
    let d = KeyEvent::default();
    let Some(p) = alloc.create(KeyEventWrapper {
        alloc,
        action: d.action,
        key: d.key,
        mods: d.mods,
        consumed_mods: d.consumed_mods,
        composing: d.composing,
        utf8: std::ptr::null(),
        utf8_len: 0,
        unshifted_codepoint: d.unshifted_codepoint,
    }) else {
        return GhosttyResult::OutOfMemory;
    };
    // SAFETY: caller guarantees `event` is valid for writes.
    unsafe { *event = p.as_ptr() };
    GhosttyResult::Success
}

/// # Safety
/// `event` must be null or a live event from `ghostty_key_event_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_free(event: *mut KeyEventWrapper) {
    let Some(p) = NonNull::new(event) else {
        return;
    };
    // SAFETY: `p` came from `Alloc::create` in `ghostty_key_event_new`.
    unsafe {
        let alloc = p.as_ref().alloc;
        alloc.destroy(p);
    }
}

macro_rules! with_event {
    ($event:expr, $e:ident => $body:expr, $default:expr) => {
        // SAFETY: caller guarantees a live event (or null).
        match unsafe { $event.as_mut() } {
            Some($e) => $body,
            None => $default,
        }
    };
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_action(event: *mut KeyEventWrapper, action: c_int) {
    let Some(action) = action_from_c(action) else {
        log::warn!("set_action invalid action value={action}");
        return;
    };
    with_event!(event, e => e.action = action, ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_action(event: *mut KeyEventWrapper) -> c_int {
    with_event!(event, e => e.action as c_int, KeyAction::Press as c_int)
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_key(event: *mut KeyEventWrapper, key: c_int) {
    let Some(key) = key_from_c(key) else {
        log::warn!("set_key invalid key value={key}");
        return;
    };
    with_event!(event, e => e.key = key, ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_key(event: *mut KeyEventWrapper) -> c_int {
    with_event!(event, e => e.key as c_int, 0)
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_mods(event: *mut KeyEventWrapper, mods: u16) {
    with_event!(event, e => e.mods = Mods::from_int(mods), ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_mods(event: *mut KeyEventWrapper) -> u16 {
    with_event!(event, e => e.mods.int(), 0)
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_consumed_mods(
    event: *mut KeyEventWrapper,
    consumed_mods: u16,
) {
    with_event!(event, e => e.consumed_mods = Mods::from_int(consumed_mods), ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_consumed_mods(event: *mut KeyEventWrapper) -> u16 {
    with_event!(event, e => e.consumed_mods.int(), 0)
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_composing(
    event: *mut KeyEventWrapper,
    composing: bool,
) {
    with_event!(event, e => e.composing = composing, ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_composing(event: *mut KeyEventWrapper) -> bool {
    with_event!(event, e => e.composing, false)
}

/// # Safety
/// `event` must be null or live; `utf8` must be null or point to `len`
/// bytes that stay valid while the event is used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_utf8(
    event: *mut KeyEventWrapper,
    utf8: *const u8,
    len: usize,
) {
    with_event!(event, e => {
        if utf8.is_null() {
            e.utf8 = std::ptr::null();
            e.utf8_len = 0;
        } else {
            e.utf8 = utf8;
            e.utf8_len = len;
        }
    }, ());
}

/// # Safety
/// `event` must be null or live; `len`, if non-null, must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_utf8(
    event: *mut KeyEventWrapper,
    len: *mut usize,
) -> *const u8 {
    with_event!(event, e => {
        if !len.is_null() {
            // SAFETY: non-null and valid per the contract.
            unsafe { *len = e.utf8_len };
        }
        if e.utf8_len == 0 { std::ptr::null() } else { e.utf8 }
    }, std::ptr::null())
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_set_unshifted_codepoint(
    event: *mut KeyEventWrapper,
    codepoint: u32,
) {
    // Zig: `@truncate` to u21.
    with_event!(event, e => e.unshifted_codepoint = codepoint & 0x1f_ffff, ());
}

/// # Safety
/// `event` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_event_get_unshifted_codepoint(
    event: *mut KeyEventWrapper,
) -> u32 {
    with_event!(event, e => e.unshifted_codepoint, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new() -> *mut KeyEventWrapper {
        let mut e = std::ptr::null_mut();
        // SAFETY: valid out pointer, default allocator.
        assert_eq!(
            unsafe { ghostty_key_event_new(std::ptr::null(), &mut e) },
            GhosttyResult::Success
        );
        e
    }

    #[test]
    fn alloc() {
        let e = new();
        // SAFETY: live event.
        unsafe { ghostty_key_event_free(e) };
    }

    #[test]
    fn set_and_get_round_trip() {
        let e = new();
        let text = b"test";
        // SAFETY: live event; `text` outlives its use.
        unsafe {
            ghostty_key_event_set_action(e, KeyAction::Repeat as c_int);
            ghostty_key_event_set_key(e, Key::KeyZ as c_int);
            ghostty_key_event_set_mods(e, 0b1100); // alt | super
            ghostty_key_event_set_consumed_mods(e, 0b0100);
            ghostty_key_event_set_composing(e, true);
            ghostty_key_event_set_utf8(e, text.as_ptr(), text.len());
            ghostty_key_event_set_unshifted_codepoint(e, 'z' as u32);

            assert_eq!(ghostty_key_event_get_action(e), KeyAction::Repeat as c_int);
            assert_eq!(ghostty_key_event_get_key(e), Key::KeyZ as c_int);
            assert_eq!(ghostty_key_event_get_mods(e), 0b1100);
            assert_eq!(ghostty_key_event_get_consumed_mods(e), 0b0100);
            assert!(ghostty_key_event_get_composing(e));
            let mut len = 0usize;
            let p = ghostty_key_event_get_utf8(e, &mut len);
            assert_eq!(std::slice::from_raw_parts(p, len), b"test");
            assert_eq!(ghostty_key_event_get_unshifted_codepoint(e), 'z' as u32);

            ghostty_key_event_set_utf8(e, std::ptr::null(), 0);
            assert!(ghostty_key_event_get_utf8(e, std::ptr::null_mut()).is_null());
            ghostty_key_event_free(e);
        }
    }

    #[test]
    fn invalid_enum_values_are_ignored() {
        let e = new();
        // SAFETY: live event.
        unsafe {
            ghostty_key_event_set_action(e, 99);
            assert_eq!(ghostty_key_event_get_action(e), KeyAction::Press as c_int);
            ghostty_key_event_set_key(e, -1);
            ghostty_key_event_set_key(e, 100_000);
            assert_eq!(ghostty_key_event_get_key(e), 0);
            ghostty_key_event_free(e);
        }
    }

    #[test]
    fn key_numbering_matches_header() {
        assert_eq!(Key::ALL.len(), 176);
        assert_eq!(key_from_c(1), Some(Key::Backquote));
        assert_eq!(key_from_c(175), Some(Key::Paste));
        assert_eq!(key_from_c(176), None);
    }
}
