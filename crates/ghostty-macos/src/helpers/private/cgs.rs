//! `Helpers/Private/CGS.swift`: private CoreGraphics Spaces API.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use objc2_core_foundation::{CFArray, CFNumber, CFRetained};

pub type CGSConnectionID = i32;
pub type CGSSpaceID = usize;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSMainConnectionID() -> CGSConnectionID;
    fn CGSGetActiveSpace(cid: CGSConnectionID) -> CGSSpaceID;
    fn CGSSpaceGetType(cid: CGSConnectionID, space_id: CGSSpaceID) -> u32;
    /// Returns a +1 retained CFArray (CFNumbers) or null.
    fn CGSCopySpacesForWindows(
        cid: CGSConnectionID,
        mask: u32,
        window_ids: &CFArray<CFNumber>,
    ) -> *const CFArray<CFNumber>;
}

/// <https://github.com/NUIKit/CGSInternal/blob/c4f6f559d624dc1cfc2bf24c8c19dbf653317fcf/CGSSpace.h#L40>
/// converted to Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CGSSpaceMask(pub u32);

impl CGSSpaceMask {
    pub const INCLUDES_CURRENT: Self = Self(1 << 0);
    pub const INCLUDES_OTHERS: Self = Self(1 << 1);
    pub const INCLUDES_USER: Self = Self(1 << 2);
    pub const INCLUDES_VISIBLE: Self = Self(1 << 16);

    pub const CURRENT_SPACE: Self = Self(Self::INCLUDES_USER.0 | Self::INCLUDES_CURRENT.0);
    pub const OTHER_SPACES: Self = Self(Self::INCLUDES_OTHERS.0 | Self::INCLUDES_CURRENT.0);
    pub const ALL_SPACES: Self =
        Self(Self::INCLUDES_USER.0 | Self::INCLUDES_OTHERS.0 | Self::INCLUDES_CURRENT.0);
    pub const ALL_VISIBLE_SPACES: Self = Self(Self::INCLUDES_VISIBLE.0 | Self::ALL_SPACES.0);
}

/// `CGSSpaceType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CGSSpaceType {
    User = 0,
    System = 2,
    Fullscreen = 4,
}

impl CGSSpaceType {
    #[must_use]
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::User),
            2 => Some(Self::System),
            4 => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

/// Represents a unique identifier for a macOS Space (Desktop, Fullscreen, etc).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CGSSpace {
    pub raw_value: CGSSpaceID,
}

impl std::fmt::Display for CGSSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpaceID({})", self.raw_value)
    }
}

impl CGSSpace {
    /// Returns the currently active space.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn active() -> Self {
        // SAFETY: private but stable CoreGraphics calls taking/returning integers.
        let space = unsafe { CGSGetActiveSpace(CGSMainConnectionID()) };
        Self { raw_value: space }
    }

    /// List the spaces for the given window.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn list(window_id: u32, mask: CGSSpaceMask) -> Vec<Self> {
        let ids = CFArray::from_retained_objects(&[CFNumber::new_i64(i64::from(window_id))]);
        // SAFETY: `ids` is a valid CFArray of CFNumbers for the call; the
        // returned array (if non-null) is +1 retained and adopted below.
        let spaces = unsafe { CGSCopySpacesForWindows(CGSMainConnectionID(), mask.0, &ids) };
        let Some(spaces) = std::ptr::NonNull::new(spaces.cast_mut()) else {
            return Vec::new();
        };
        // SAFETY: ownership of the +1 reference is transferred.
        let spaces: CFRetained<CFArray<CFNumber>> = unsafe { CFRetained::from_raw(spaces) };
        spaces
            .iter()
            .filter_map(|n| n.as_i64())
            .map(|v| Self {
                raw_value: v as CGSSpaceID,
            })
            .collect()
    }

    /// `type`.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn space_type(&self) -> Option<CGSSpaceType> {
        // SAFETY: integer arguments only.
        let raw = unsafe { CGSSpaceGetType(CGSMainConnectionID(), self.raw_value) };
        CGSSpaceType::from_raw(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        assert_eq!(CGSSpaceMask::ALL_SPACES.0, 0b111);
        assert_eq!(CGSSpaceMask::ALL_VISIBLE_SPACES.0, (1 << 16) | 0b111);
        assert_eq!(CGSSpaceMask::CURRENT_SPACE.0, 0b101);
        assert_eq!(CGSSpaceType::from_raw(4), Some(CGSSpaceType::Fullscreen));
    }
}
