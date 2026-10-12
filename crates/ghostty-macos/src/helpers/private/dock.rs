//! `Helpers/Private/Dock.swift`: private APIs to get/set the Dock state.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    // Private API to get Dock location
    fn CoreDockGetOrientationAndPinning(out_orientation: *mut i32, out_pinning: *mut i32);
    // Private API to get the current Dock auto-hide state
    fn CoreDockGetAutoHideEnabled() -> bool;
    // Toggles the Dock's auto-hide state
    fn CoreDockSetAutoHideEnabled(flag: bool);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockOrientation {
    Top = 1,
    Bottom = 2,
    Left = 3,
    Right = 4,
}

impl DockOrientation {
    #[must_use]
    pub fn from_raw(raw: i32) -> Option<Self> {
        match raw {
            1 => Some(Self::Top),
            2 => Some(Self::Bottom),
            3 => Some(Self::Left),
            4 => Some(Self::Right),
            _ => None,
        }
    }
}

pub struct Dock;

impl Dock {
    /// Returns the orientation of the dock or None if it can't be determined.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn orientation() -> Option<DockOrientation> {
        let mut orientation = 0i32;
        let mut pinning = 0i32;
        // SAFETY: both out-pointers are valid for writes for the call.
        unsafe { CoreDockGetOrientationAndPinning(&raw mut orientation, &raw mut pinning) };
        DockOrientation::from_raw(orientation)
    }

    /// The dock autohide.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn auto_hide_enabled() -> bool {
        // SAFETY: no arguments; plain getter.
        unsafe { CoreDockGetAutoHideEnabled() }
    }

    #[allow(unsafe_code)]
    pub fn set_auto_hide_enabled(value: bool) {
        // SAFETY: plain setter taking a bool.
        unsafe { CoreDockSetAutoHideEnabled(value) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_raw() {
        assert_eq!(DockOrientation::from_raw(3), Some(DockOrientation::Left));
        assert_eq!(DockOrientation::from_raw(0), None);
    }
}
