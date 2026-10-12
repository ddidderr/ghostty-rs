//! `Helpers/Extensions/NSAppearance+Extension.swift`.

use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};

use super::os_color;
use crate::ghostty::config::Config;

pub trait NSAppearanceExt {
    /// Returns true if the appearance is some kind of dark.
    fn is_dark(&self) -> bool;
}

impl NSAppearanceExt for NSAppearance {
    fn is_dark(&self) -> bool {
        self.name().to_string().to_lowercase().contains("dark")
    }
}

/// `NSAppearance(ghosttyConfig:)`: the desired NSAppearance for the Ghostty
/// configuration (`window-theme`), if any.
#[must_use]
#[allow(unsafe_code)]
pub fn appearance_from_ghostty_config(config: &Config) -> Option<Retained<NSAppearance>> {
    let theme = config.window_theme()?;
    // SAFETY: reading AppKit's immutable appearance name constants.
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    match theme.as_str() {
        "dark" => NSAppearance::appearanceNamed(dark),
        "light" => NSAppearance::appearanceNamed(aqua),
        "auto" => {
            let c = config.background_color();
            if os_color::is_light_color(c.red, c.green, c.blue) {
                NSAppearance::appearanceNamed(aqua)
            } else {
                NSAppearance::appearanceNamed(dark)
            }
        }
        _ => None,
    }
}
