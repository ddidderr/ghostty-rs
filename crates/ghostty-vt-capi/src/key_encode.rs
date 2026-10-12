//! Key encoder C API (ZIG/src/terminal/c/key_encode.zig, `include/ghostty/vt/key/encoder.h`).
#![allow(unsafe_code)]

use std::{
    ffi::{c_int, c_void},
    io::{self, Write},
    ptr::NonNull,
};

use ghostty_input::{
    OptionAsAlt,
    key_encode::{Options, encode},
};
use ghostty_vt::kitty::key::Flags as KittyFlags;

use crate::{
    alloc::{Alloc, Allocator},
    key_event::KeyEventWrapper,
    result::GhosttyResult,
};

/// C: `GhosttyKeyEncoder` (opaque). Mirrors Zig's `KeyEncoderWrapper`.
pub struct KeyEncoderWrapper {
    alloc: Alloc,
    opts: Options,
}

/// C: `GhosttyKeyEncoderOption`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum KeyEncoderOption {
    CursorKeyApplication = 0,
    KeypadKeyApplication = 1,
    IgnoreKeypadWithNumlock = 2,
    AltEscPrefix = 3,
    ModifyOtherKeysState2 = 4,
    KittyFlags = 5,
    MacosOptionAsAlt = 6,
}

impl KeyEncoderOption {
    fn from_c(v: c_int) -> Option<Self> {
        Some(match v {
            0 => Self::CursorKeyApplication,
            1 => Self::KeypadKeyApplication,
            2 => Self::IgnoreKeypadWithNumlock,
            3 => Self::AltEscPrefix,
            4 => Self::ModifyOtherKeysState2,
            5 => Self::KittyFlags,
            6 => Self::MacosOptionAsAlt,
            _ => return None,
        })
    }
}

/// C: `GhosttyOptionAsAlt` (values of [`OptionAsAlt`]).
fn option_as_alt_from_c(v: c_int) -> Option<OptionAsAlt> {
    Some(match v {
        0 => OptionAsAlt::False,
        1 => OptionAsAlt::True,
        2 => OptionAsAlt::Left,
        3 => OptionAsAlt::Right,
        _ => return None,
    })
}

/// # Safety
/// `allocator` must be null or valid; `encoder` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_encoder_new(
    allocator: *const Allocator,
    encoder: *mut *mut KeyEncoderWrapper,
) -> GhosttyResult {
    // SAFETY: forwarded caller contract.
    let alloc = unsafe { Alloc::from_c(allocator) };
    let Some(p) = alloc.create(KeyEncoderWrapper {
        alloc,
        opts: Options::default(),
    }) else {
        return GhosttyResult::OutOfMemory;
    };
    // SAFETY: caller guarantees `encoder` is valid for writes.
    unsafe { *encoder = p.as_ptr() };
    GhosttyResult::Success
}

/// # Safety
/// `encoder` must be null or a live encoder from `ghostty_key_encoder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_encoder_free(encoder: *mut KeyEncoderWrapper) {
    let Some(p) = NonNull::new(encoder) else {
        return;
    };
    // SAFETY: `p` came from `Alloc::create` in `ghostty_key_encoder_new`.
    unsafe {
        let alloc = p.as_ref().alloc;
        alloc.destroy(p);
    }
}

/// # Safety
/// `encoder` must be null or live; `value` must be null or point to the type
/// documented for `option` (`bool`, `uint8_t` or `GhosttyOptionAsAlt`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_encoder_setopt(
    encoder: *mut KeyEncoderWrapper,
    option: c_int,
    value: *const c_void,
) {
    let Some(option) = KeyEncoderOption::from_c(option) else {
        log::warn!("setopt invalid option value={option}");
        return;
    };
    if value.is_null() {
        return;
    }
    // SAFETY: caller guarantees a live encoder (or null).
    let Some(enc) = (unsafe { encoder.as_mut() }) else {
        return;
    };
    let opts = &mut enc.opts;
    // SAFETY (all reads): `value` is non-null and points to the option's type.
    unsafe {
        let flag = || value.cast::<u8>().read_unaligned() != 0;
        match option {
            KeyEncoderOption::CursorKeyApplication => opts.cursor_key_application = flag(),
            KeyEncoderOption::KeypadKeyApplication => opts.keypad_key_application = flag(),
            KeyEncoderOption::IgnoreKeypadWithNumlock => {
                opts.ignore_keypad_with_numlock = flag();
            }
            KeyEncoderOption::AltEscPrefix => opts.alt_esc_prefix = flag(),
            KeyEncoderOption::ModifyOtherKeysState2 => opts.modify_other_keys_state_2 = flag(),
            KeyEncoderOption::KittyFlags => {
                // Zig: truncate to the 5 flag bits.
                opts.kitty_flags = KittyFlags::from_int(value.cast::<u8>().read_unaligned() & 0x1f);
            }
            KeyEncoderOption::MacosOptionAsAlt => {
                let raw = value.cast::<c_int>().read_unaligned();
                match option_as_alt_from_c(raw) {
                    Some(v) => opts.macos_option_as_alt = v,
                    None => log::warn!("setopt invalid OptionAsAlt value={raw}"),
                }
            }
        }
    }
}

/// Counts bytes without storing them (Zig `Writer.Discarding`).
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// # Safety
/// `encoder`/`event` must be live; `out_buf` must be null or point to
/// `out_buf_size` writable bytes; `out_len` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_key_encoder_encode(
    encoder: *mut KeyEncoderWrapper,
    event: *mut KeyEventWrapper,
    out_buf: *mut u8,
    out_buf_size: usize,
    out_len: *mut usize,
) -> GhosttyResult {
    // SAFETY: caller guarantees live handles.
    let (Some(enc), Some(ev)) = (unsafe { encoder.as_ref() }, unsafe { event.as_ref() }) else {
        return GhosttyResult::InvalidValue;
    };
    // SAFETY: the event's UTF-8 bytes must be alive per `set_utf8`'s contract.
    let ev = unsafe { ev.as_event() };

    let buf: &mut [u8] = if out_buf.is_null() {
        &mut []
    } else {
        // SAFETY: caller guarantees `out_buf_size` writable bytes.
        unsafe { std::slice::from_raw_parts_mut(out_buf, out_buf_size) }
    };
    let capacity = buf.len();
    let mut cursor: &mut [u8] = buf;
    let result = encode(&mut cursor, &ev, &enc.opts);
    let written = capacity - cursor.len();

    let (code, n) = if result.is_ok() {
        (GhosttyResult::Success, written)
    } else {
        // Not enough space: count how much we would have needed.
        let mut counter = Counter(0);
        let _ = encode(&mut counter, &ev, &enc.opts);
        (GhosttyResult::OutOfMemory, counter.0)
    };
    if !out_len.is_null() {
        // SAFETY: non-null and valid per the contract.
        unsafe { *out_len = n };
    }
    code
}

#[cfg(test)]
mod tests {
    use ghostty_input::Key;

    use super::*;
    use crate::key_event::{
        ghostty_key_event_free,
        ghostty_key_event_new,
        ghostty_key_event_set_action,
        ghostty_key_event_set_key,
        ghostty_key_event_set_mods,
    };

    fn new() -> *mut KeyEncoderWrapper {
        let mut e = std::ptr::null_mut();
        // SAFETY: valid out pointer, default allocator.
        assert_eq!(
            unsafe { ghostty_key_encoder_new(std::ptr::null(), &mut e) },
            GhosttyResult::Success
        );
        e
    }

    #[test]
    fn setopt_bool_kitty_and_option_as_alt() {
        let e = new();
        // SAFETY: live encoder; values point to the right types.
        unsafe {
            let t = true;
            ghostty_key_encoder_setopt(e, 0, (&raw const t).cast());
            assert!((*e).opts.cursor_key_application);
            let f = false;
            ghostty_key_encoder_setopt(e, 0, (&raw const f).cast());
            assert!(!(*e).opts.cursor_key_application);
            ghostty_key_encoder_setopt(e, 1, (&raw const t).cast());
            assert!((*e).opts.keypad_key_application);

            let flags = 0b0000_0011u8;
            ghostty_key_encoder_setopt(e, 5, (&raw const flags).cast());
            assert!((*e).opts.kitty_flags.disambiguate && (*e).opts.kitty_flags.report_events);
            assert!(!(*e).opts.kitty_flags.report_alternates);

            let left: c_int = 2;
            ghostty_key_encoder_setopt(e, 6, (&raw const left).cast());
            assert_eq!((*e).opts.macos_option_as_alt, OptionAsAlt::Left);
            let bad: c_int = 77;
            ghostty_key_encoder_setopt(e, 6, (&raw const bad).cast());
            assert_eq!((*e).opts.macos_option_as_alt, OptionAsAlt::Left);
            // Invalid option / null value are ignored.
            ghostty_key_encoder_setopt(e, 99, (&raw const t).cast());
            ghostty_key_encoder_setopt(e, 0, std::ptr::null());
            ghostty_key_encoder_free(e);
        }
    }

    #[test]
    fn encode_kitty_ctrl_release_with_ctrl_mod_set() {
        let enc = new();
        let mut ev = std::ptr::null_mut();
        // SAFETY: live handles, valid buffers.
        unsafe {
            let all = 0x1fu8;
            ghostty_key_encoder_setopt(enc, 5, (&raw const all).cast());
            assert_eq!(
                ghostty_key_event_new(std::ptr::null(), &mut ev),
                GhosttyResult::Success
            );
            ghostty_key_event_set_action(ev, 0);
            ghostty_key_event_set_key(ev, Key::ControlLeft as c_int);
            ghostty_key_event_set_mods(ev, 0b10);

            let mut required = 0usize;
            assert_eq!(
                ghostty_key_encoder_encode(enc, ev, std::ptr::null_mut(), 0, &mut required),
                GhosttyResult::OutOfMemory
            );
            let mut buf = [0u8; 128];
            let mut written = 0usize;
            assert_eq!(
                ghostty_key_encoder_encode(enc, ev, buf.as_mut_ptr(), buf.len(), &mut written),
                GhosttyResult::Success
            );
            assert_eq!(required, written);
            assert_eq!(&buf[..written], b"\x1b[57442;5:3u");

            // Exactly-too-small buffer reports the needed size.
            let mut small = [0u8; 3];
            assert_eq!(
                ghostty_key_encoder_encode(enc, ev, small.as_mut_ptr(), small.len(), &mut written),
                GhosttyResult::OutOfMemory
            );
            assert_eq!(written, required);

            ghostty_key_event_free(ev);
            ghostty_key_encoder_free(enc);
        }
    }
}
