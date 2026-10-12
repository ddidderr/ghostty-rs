//! Typed access to `GtkSettings` (port of `apprt/gtk/gsettings.zig`).

use gtk::{glib, prelude::*};

/// Settings keys we read (the GTK property names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    GtkEnablePrimaryPaste,
    GtkXftDpi,
    GtkFontName,
}

impl Key {
    /// The GTK property name.
    pub fn name(self) -> &'static str {
        match self {
            Key::GtkEnablePrimaryPaste => "gtk-enable-primary-paste",
            Key::GtkXftDpi => "gtk-xft-dpi",
            Key::GtkFontName => "gtk-font-name",
        }
    }

    /// Whether the value type needs heap allocation (strings).
    pub fn requires_allocation(self) -> bool {
        matches!(self, Key::GtkFontName)
    }
}

fn settings() -> Option<gtk::Settings> {
    gtk::Settings::default()
}

/// Get a boolean setting (`gtk-enable-primary-paste`).
pub fn get_bool(key: Key) -> Option<bool> {
    debug_assert_eq!(key, Key::GtkEnablePrimaryPaste);
    let s = settings()?;
    Some(s.property::<bool>(key.name()))
}

/// Get an integer setting (`gtk-xft-dpi`).
pub fn get_int(key: Key) -> Option<i32> {
    debug_assert_eq!(key, Key::GtkXftDpi);
    let s = settings()?;
    Some(s.property::<i32>(key.name()))
}

/// Get a string setting (`gtk-font-name`). None if unset.
pub fn get_string(key: Key) -> Option<String> {
    debug_assert_eq!(key, Key::GtkFontName);
    let s = settings()?;
    s.property::<Option<glib::GString>>(key.name())
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_requires_allocation() {
        assert!(!Key::GtkEnablePrimaryPaste.requires_allocation());
        assert!(!Key::GtkXftDpi.requires_allocation());
        assert!(Key::GtkFontName.requires_allocation());
    }

    #[test]
    fn key_names() {
        assert_eq!(
            Key::GtkEnablePrimaryPaste.name(),
            "gtk-enable-primary-paste"
        );
        assert_eq!(Key::GtkXftDpi.name(), "gtk-xft-dpi");
        assert_eq!(Key::GtkFontName.name(), "gtk-font-name");
    }
}
