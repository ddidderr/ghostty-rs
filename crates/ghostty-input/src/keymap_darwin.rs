//! Keymap is responsible for translating keyboard inputs into localized
//! chars. Port of `input/KeymapDarwin.zig`.
//!
//! For example, the physical key "S" on a US-layout keyboard might mean "O"
//! in Dvorak. On international keyboard layouts, it may require multiple
//! keystrokes to produce a single character that is otherwise a single
//! keystroke on a US-layout keyboard.
//!
//! This information is critical to know for many reasons. For keybindings,
//! if a user configures "ctrl+o" to do something, it should work with the
//! physical "ctrl+S" key on a Dvorak keyboard and so on.
//!
//! The Carbon / HIToolbox FFI only exists on macOS (`target_os = "macos"`);
//! the platform-independent pieces (Carbon modifier mapping, UTF-16
//! decoding) compile everywhere so they are unit-testable.

use crate::Mods;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("get input source failed")]
    GetInputSourceFailed,
    #[error("translate failed")]
    TranslateFailed,
    #[error("invalid utf-16 in translation")]
    InvalidUtf16,
}

/// The state that has to be passed in with each call to translate.
/// The contents of this are meant to mostly be opaque and can change
/// for platform-specific reasons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    pub dead_key: u32,
}

/// The result of a translation. The result of a translation can be multiple
/// states. For example, if the user types a dead key, the result will be
/// "composing" since they're still in the process of composing a full
/// character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Translation {
    /// The translation result. If this is a dead key state, then this will
    /// be pre-edit text that can be displayed but will ultimately be
    /// replaced.
    pub text: String,

    /// Whether the text is still composing, i.e. this is a dead key state.
    pub composing: bool,

    /// The mods that were consumed to produce this translation
    pub mods: Mods,
}

/// Map to the modifiers format used by the UCKeyTranslate function.
/// We use a u32 here because our bit arithmetic is all u32 anyways.
///
/// Layout (`packed struct(u32)` upstream): 8 padding bits, then `meta`,
/// `shift`, `caps_lock`, `alt`, `ctrl`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CarbonMods {
    pub meta: bool,
    pub shift: bool,
    pub caps_lock: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl CarbonMods {
    /// The packed u32 representation (`@bitCast` upstream).
    pub fn to_bits(self) -> u32 {
        u32::from(self.meta) << 8
            | u32::from(self.shift) << 9
            | u32::from(self.caps_lock) << 10
            | u32::from(self.alt) << 11
            | u32::from(self.ctrl) << 12
    }

    /// Translate NSEventModifierFlags into the format used by
    /// UCKeyTranslate.
    pub fn uc_key_translate(self) -> u32 {
        (self.to_bits() >> 8) & 0xFF
    }
}

/// Convert our mods to the Carbon API format. On macOS we strip ctrl because
/// UCKeyTranslate converts to the masked values (i.e. ctrl+c becomes 3) and
/// we don't want that behavior in Ghostty ever. This makes this file not a
/// general-purpose keymap implementation.
pub fn carbon_modifier_state(input_mods: Mods) -> (Mods, u32) {
    let mut mods = input_mods;
    mods.ctrl = false;

    let state = CarbonMods {
        alt: mods.alt,
        ctrl: mods.ctrl,
        meta: mods.super_,
        shift: mods.shift,
        caps_lock: mods.caps_lock,
    }
    .uc_key_translate();
    (mods, state)
}

/// Convert the UTF-16 produced by UCKeyTranslate to UTF-8
/// (`std.unicode.utf16LeToUtf8`).
pub fn utf16_to_utf8(units: &[u16]) -> Result<String, Error> {
    String::from_utf16(units).map_err(|_| Error::InvalidUtf16)
}

#[cfg(target_os = "macos")]
pub use darwin::Keymap;

#[cfg(target_os = "macos")]
#[allow(unsafe_code, non_snake_case, non_upper_case_globals)]
mod darwin {
    use std::{ffi::c_void, ptr::NonNull};

    use objc2_core_foundation::{CFData, CFRetained, CFString, CFType};

    use super::{Error, State, Translation, carbon_modifier_state, utf16_to_utf8};
    use crate::{
        Mods,
        keycodes::{NativeColumn, native_for_code},
    };

    // The documentation for all of these types and functions is in the macOS
    // SDK: Carbon.framework/Versions/A/Frameworks/HIToolbox.framework/
    // Versions/A/Headers/TextInputSources.h
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: &CFString) -> *const c_void;
        fn LMGetKbdType() -> u8;
        fn UCKeyTranslate(
            key_layout: *const c_void,
            virtual_key_code: u16,
            key_action: u16,
            modifier_key_state: u32,
            keyboard_type: u32,
            key_translate_options: u32,
            dead_key_state: *mut u32,
            max_string_length: std::ffi::c_ulong,
            actual_string_length: *mut std::ffi::c_ulong,
            unicode_string: *mut u16,
        ) -> i32;

        static kTISPropertyUnicodeKeyLayoutData: &'static CFString;
        static kTISPropertyInputSourceID: &'static CFString;
    }

    const K_UC_KEY_ACTION_DOWN: u16 = 0;
    const K_UC_KEY_TRANSLATE_NO_DEAD_KEYS_BIT: u32 = 0;
    const K_UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1 << K_UC_KEY_TRANSLATE_NO_DEAD_KEYS_BIT;

    /// The keyboard layout translation state of the current input source.
    pub struct Keymap {
        /// The current input source that is selected for the keyboard. This
        /// can and does change whenever the user selects a new keyboard
        /// layout. This change doesn't happen automatically; the user of
        /// this struct has to detect it and then call `reload` to update
        /// the keymap.
        source: CFRetained<CFType>,

        /// The keyboard layout (`UCKeyboardLayout *`) for the current input
        /// source. This doesn't need to be freed because its owned by the
        /// input source.
        unicode_layout: *const c_void,
    }

    // SAFETY: the input source is a thread-safe CF object and the layout
    // data is immutable and owned by it.
    unsafe impl Send for Keymap {}

    impl Keymap {
        pub fn new() -> Result<Keymap, Error> {
            let (source, unicode_layout) = Self::load()?;
            let keymap = Keymap {
                source,
                unicode_layout,
            };
            if cfg!(debug_assertions)
                && let Some(id) = keymap.source_id()
            {
                log::debug!("keyboard layout={id}");
            }
            Ok(keymap)
        }

        /// Reload the keymap. This must be called if the user changes their
        /// keyboard layout.
        pub fn reload(&mut self) -> Result<(), Error> {
            let (source, unicode_layout) = Self::load()?;
            self.source = source;
            self.unicode_layout = unicode_layout;
            Ok(())
        }

        fn load() -> Result<(CFRetained<CFType>, *const c_void), Error> {
            // SAFETY: plain Carbon call; returns a +1 retained input source
            // or null.
            let raw = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
            let raw = NonNull::new(raw.cast::<CFType>()).ok_or(Error::GetInputSourceFailed)?;
            // SAFETY: we own the +1 retain count from the Copy function.
            let source = unsafe { CFRetained::from_raw(raw) };

            // This returns a CFDataRef.
            // SAFETY: `source` is a valid TISInputSource and the key is a
            // valid constant.
            let data_raw = unsafe {
                TISGetInputSourceProperty(raw.as_ptr().cast(), kTISPropertyUnicodeKeyLayoutData)
            };
            if data_raw.is_null() {
                return Err(Error::GetInputSourceFailed);
            }
            // SAFETY: the property is documented to be a CFDataRef owned by
            // the input source (alive as long as `source`).
            let data = unsafe { &*data_raw.cast::<CFData>() };

            // The CFDataRef contains a UCKeyboardLayout pointer.
            Ok((source, data.byte_ptr().cast()))
        }

        /// Get the input source ID for the current keyboard layout. The
        /// input source ID is a unique identifier for the keyboard layout
        /// which is uniquely defined by Apple.
        ///
        /// This is macOS-only. Other platforms don't have an equivalent of
        /// this so this isn't expected to be generally implemented.
        pub fn source_id(&self) -> Option<String> {
            // SAFETY: valid source and constant key; the result is a
            // CFStringRef owned by the source.
            let id_raw = unsafe {
                TISGetInputSourceProperty(
                    std::ptr::from_ref::<CFType>(&self.source).cast_mut().cast(),
                    kTISPropertyInputSourceID,
                )
            };
            if id_raw.is_null() {
                return None;
            }
            // SAFETY: documented to be a CFStringRef.
            let id = unsafe { &*id_raw.cast::<CFString>() };
            Some(id.to_string())
        }

        /// Translate a single key input into a utf8 sequence.
        pub fn translate(
            &self,
            state: &mut State,
            code: u16,
            input_mods: Mods,
        ) -> Result<Translation, Error> {
            // On macOS we strip ctrl because UCKeyTranslate converts to the
            // masked values (see `carbon_modifier_state`).
            let (mods, modifier_state) = carbon_modifier_state(input_mods);

            // Get the keycode for the space key.
            let code_space =
                native_for_code("Space", NativeColumn::Mac).expect("space code not found") as u16;

            // We use 4 here because the Chromium source code uses 4 and
            // Chrome works pretty well. They have a todo to look into
            // longer sequences but given how mature that software is I think
            // this is fine.
            //
            // From Chromium: Per Apple docs, the buffer length can be up to
            // 255 but is rarely more than 4.
            // https://developer.apple.com/documentation/coreservices/1390584-uckeytranslate
            let mut char_buf = [0u16; 4];
            let mut char_count: std::ffi::c_ulong = 0;
            // SAFETY: the layout pointer is valid while `self.source` lives;
            // all out pointers reference live locals with matching sizes.
            let rc = unsafe {
                UCKeyTranslate(
                    self.unicode_layout,
                    code,
                    K_UC_KEY_ACTION_DOWN,
                    modifier_state,
                    u32::from(LMGetKbdType()),
                    K_UC_KEY_TRANSLATE_NO_DEAD_KEYS_BIT,
                    &raw mut state.dead_key,
                    char_buf.len() as std::ffi::c_ulong,
                    &raw mut char_count,
                    char_buf.as_mut_ptr(),
                )
            };
            if rc != 0 {
                return Err(Error::TranslateFailed);
            }

            // If we got a dead key, then we translate again with "space" in
            // order to get the pre-edit text.
            let composing = if state.dead_key != 0 && char_count == 0 {
                // We need to copy our dead key state so that it isn't
                // modified.
                let mut dead_key_ignore: u32 = state.dead_key;
                // SAFETY: as above.
                let rc = unsafe {
                    UCKeyTranslate(
                        self.unicode_layout,
                        code_space,
                        K_UC_KEY_ACTION_DOWN,
                        modifier_state,
                        u32::from(LMGetKbdType()),
                        K_UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK,
                        &raw mut dead_key_ignore,
                        char_buf.len() as std::ffi::c_ulong,
                        &raw mut char_count,
                        char_buf.as_mut_ptr(),
                    )
                };
                if rc != 0 {
                    return Err(Error::TranslateFailed);
                }
                true
            } else {
                false
            };

            // Convert the utf16 to utf8
            let text = utf16_to_utf8(&char_buf[..(char_count as usize).min(char_buf.len())])?;
            Ok(Translation {
                text,
                composing,
                mods,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn carbon_mods_expected_values() {
        // We got this from dumping the values out of another program since
        // I can't find the header for this anywhere.
        assert_eq!(
            CarbonMods {
                meta: true,
                ..Default::default()
            }
            .to_bits(),
            0x100
        );
        assert_eq!(
            CarbonMods {
                shift: true,
                ..Default::default()
            }
            .to_bits(),
            0x200
        );
        assert_eq!(
            CarbonMods {
                caps_lock: true,
                ..Default::default()
            }
            .to_bits(),
            0x400
        );
        assert_eq!(
            CarbonMods {
                alt: true,
                ..Default::default()
            }
            .to_bits(),
            0x800
        );
        assert_eq!(
            CarbonMods {
                ctrl: true,
                ..Default::default()
            }
            .to_bits(),
            0x1000
        );
    }

    #[test]
    fn uc_key_translate_shifts_down_eight_bits() {
        let m = CarbonMods {
            meta: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(m.uc_key_translate(), 0x03);
        let m = CarbonMods {
            alt: true,
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(m.uc_key_translate(), 0x18);
    }

    #[test]
    fn modifier_state_strips_ctrl_and_maps_super_to_meta() {
        let (mods, state) = carbon_modifier_state(Mods {
            ctrl: true,
            super_: true,
            shift: true,
            caps_lock: true,
            alt: true,
            ..Mods::default()
        });
        assert!(!mods.ctrl);
        assert!(mods.super_ && mods.shift && mods.alt && mods.caps_lock);
        // meta(1) | shift(2) | caps(4) | alt(8); ctrl removed.
        assert_eq!(state, 0x0F);
    }

    #[test]
    fn utf16_decoding() {
        assert_eq!(utf16_to_utf8(&[0x00E1]).unwrap(), "á");
        assert_eq!(utf16_to_utf8(&[0xD83D, 0xDE00]).unwrap(), "😀");
        assert_eq!(utf16_to_utf8(&[]).unwrap(), "");
        assert_eq!(utf16_to_utf8(&[0xD83D]), Err(Error::InvalidUtf16));
    }
}
