//! `Ghostty/FullscreenMode+Extension.swift`.

use super::capi::consts::{
    GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE,
    GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE_PADDED_NOTCH,
    GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE_VISIBLE_MENU,
    GHOSTTY_FULLSCREEN_NATIVE,
};
use crate::helpers::fullscreen_logic::FullscreenMode;

/// `FullscreenMode.from(ghostty:)`: initialize from a Ghostty fullscreen action.
#[must_use]
pub fn fullscreen_mode_from_ghostty(raw: std::ffi::c_int) -> Option<FullscreenMode> {
    match raw {
        GHOSTTY_FULLSCREEN_NATIVE => Some(FullscreenMode::Native),
        GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE => Some(FullscreenMode::NonNative),
        GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE_VISIBLE_MENU => {
            Some(FullscreenMode::NonNativeVisibleMenu)
        }
        GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE_PADDED_NOTCH => {
            Some(FullscreenMode::NonNativePaddedNotch)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_modes() {
        assert_eq!(
            fullscreen_mode_from_ghostty(GHOSTTY_FULLSCREEN_NATIVE),
            Some(FullscreenMode::Native)
        );
        assert_eq!(
            fullscreen_mode_from_ghostty(GHOSTTY_FULLSCREEN_MACOS_NON_NATIVE_PADDED_NOTCH),
            Some(FullscreenMode::NonNativePaddedNotch)
        );
        assert_eq!(fullscreen_mode_from_ghostty(-1), None);
    }
}
