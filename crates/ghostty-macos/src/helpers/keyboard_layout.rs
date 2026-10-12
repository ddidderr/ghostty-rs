//! `Helpers/KeyboardLayout.swift`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::{ffi::c_void, ptr::NonNull};

use objc2_core_foundation::{CFRetained, CFString};

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: &CFString) -> *const c_void;
    static kTISPropertyInputSourceID: &'static CFString;
}

pub struct KeyboardLayout;

impl KeyboardLayout {
    /// Return a string ID of the current keyboard input source.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn id() -> Option<String> {
        // SAFETY: TISCopyCurrentKeyboardInputSource returns a +1 retained
        // CFTypeRef or null; `CFRetained` takes ownership (released on drop).
        // TISGetInputSourceProperty returns a borrowed CFString valid while
        // the source is alive, which we copy into a Rust String before then.
        unsafe {
            let raw = NonNull::new(TISCopyCurrentKeyboardInputSource())?;
            let source: CFRetained<CFString> = CFRetained::from_raw(raw.cast());
            let ptr = TISGetInputSourceProperty(raw.as_ptr(), kTISPropertyInputSourceID);
            let id = (!ptr.is_null()).then(|| (*ptr.cast::<CFString>()).to_string());
            drop(source);
            id
        }
    }
}
