//! The platform independent parts of `Helpers/Fullscreen.swift`: the
//! [`FullscreenMode`] enum (which `Ghostty.Config` also returns) and the
//! frame / menu decisions of non-native fullscreen.

use super::geometry::Rect;

/// The fullscreen modes we support define how the fullscreen behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FullscreenMode {
    Native,
    NonNative,
    NonNativeVisibleMenu,
    NonNativePaddedNotch,
}

impl FullscreenMode {
    /// The Swift `rawValue` (also the `Codable` encoding).
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::NonNative => "nonNative",
            Self::NonNativeVisibleMenu => "nonNativeVisibleMenu",
            Self::NonNativePaddedNotch => "nonNativePaddedNotch",
        }
    }

    /// `FullscreenMode(rawValue:)`.
    #[must_use]
    pub fn from_raw_value(raw: &str) -> Option<Self> {
        match raw {
            "native" => Some(Self::Native),
            "nonNative" => Some(Self::NonNative),
            "nonNativeVisibleMenu" => Some(Self::NonNativeVisibleMenu),
            "nonNativePaddedNotch" => Some(Self::NonNativePaddedNotch),
            _ => None,
        }
    }
}

/// `NonNativeFullscreen.Properties`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Properties {
    pub hide_menu: bool,
    pub padded_notch: bool,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            hide_menu: true,
            padded_notch: false,
        }
    }
}

impl Properties {
    /// The properties of the non-native style for `mode` (native has none).
    #[must_use]
    pub fn for_mode(mode: FullscreenMode) -> Self {
        match mode {
            FullscreenMode::NonNativeVisibleMenu => Self {
                hide_menu: false,
                ..Self::default()
            },
            FullscreenMode::NonNativePaddedNotch => Self {
                padded_notch: true,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

/// `NonNativeFullscreen.fullscreenFrame`: it would make more sense to use
/// `visibleFrame` but that omits space by the dock and isn't updated until an
/// event loop tick, so the frame is computed from `screen_frame`.
#[must_use]
pub fn fullscreen_frame(
    screen_frame: Rect,
    menu_bar_hidden: bool,
    menu_bar_height: f64,
    padded_notch: bool,
    safe_area_top: f64,
) -> Rect {
    let mut frame = screen_frame;
    if !menu_bar_hidden {
        // We need to subtract the menu height since we're still showing it.
        frame.size.height -= menu_bar_height;
    } else if padded_notch {
        // We are hiding the menu, we may need to avoid the notch.
        frame.size.height -= safe_area_top;
    }
    frame
}

/// `SavedState.menu`: hide the menu only if this window is not on any
/// fullscreen space (fullscreen spaces already hide the menu; inserting and
/// removing the presentation option causes issues, see #7075).
///
/// `spaces` is `(is_active_space, is_fullscreen_space)` per space of the window.
#[must_use]
pub fn can_hide_menu(spaces: &[(bool, bool)], active_is_fullscreen: bool) -> bool {
    if spaces.iter().any(|&(is_active, _)| is_active) {
        !active_is_fullscreen
    } else {
        spaces.iter().all(|&(_, fullscreen)| !fullscreen)
    }
}

/// Where a non-native-fullscreen window re-enters its tab group on exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabReinsert {
    /// Before ("below") the first window of the group.
    BelowFirst,
    /// After ("above") the window at this index.
    AboveIndex(usize),
    /// Below the last window.
    BelowLast,
}

/// `exit()`'s placement: `tab_index` was the saved index, `count` the current
/// number of windows in the (non-empty) group.
#[must_use]
pub fn tab_reinsert(tab_index: usize, count: usize) -> TabReinsert {
    if tab_index == 0 {
        TabReinsert::BelowFirst
    } else if tab_index <= count {
        TabReinsert::AboveIndex(tab_index - 1)
    } else {
        TabReinsert::BelowLast
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_values_roundtrip() {
        for m in [
            FullscreenMode::Native,
            FullscreenMode::NonNative,
            FullscreenMode::NonNativeVisibleMenu,
            FullscreenMode::NonNativePaddedNotch,
        ] {
            assert_eq!(FullscreenMode::from_raw_value(m.raw_value()), Some(m));
        }
        assert_eq!(FullscreenMode::from_raw_value("x"), None);
    }

    #[test]
    fn properties_per_mode() {
        assert!(Properties::for_mode(FullscreenMode::NonNative).hide_menu);
        assert!(!Properties::for_mode(FullscreenMode::NonNativeVisibleMenu).hide_menu);
        assert!(Properties::for_mode(FullscreenMode::NonNativePaddedNotch).padded_notch);
    }

    #[test]
    fn frame_subtracts_menu_or_notch() {
        let f = Rect::new(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(fullscreen_frame(f, false, 24.0, true, 32.0).height(), 776.0);
        assert_eq!(fullscreen_frame(f, true, 24.0, true, 32.0).height(), 768.0);
        assert_eq!(fullscreen_frame(f, true, 24.0, false, 32.0).height(), 800.0);
    }

    #[test]
    fn menu_hiding_decision() {
        assert!(can_hide_menu(&[], false));
        assert!(!can_hide_menu(&[(true, true)], true));
        assert!(can_hide_menu(&[(true, false), (false, true)], false));
        assert!(!can_hide_menu(&[(false, true)], false));
        assert!(can_hide_menu(&[(false, false)], false));
    }

    #[test]
    fn tab_positions() {
        assert_eq!(tab_reinsert(0, 3), TabReinsert::BelowFirst);
        assert_eq!(tab_reinsert(2, 3), TabReinsert::AboveIndex(1));
        assert_eq!(tab_reinsert(4, 3), TabReinsert::BelowLast);
    }
}
