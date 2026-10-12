//! `Helpers/Extensions/*`: one module per Swift file. A Swift `extension X`
//! becomes an extension trait `XExt` implemented for the objc2 type (or free
//! functions for value types). Pure logic is in the platform independent
//! modules; the AppKit traits are `cfg(target_vendor = "apple")`.
//!
//! Unportable (SwiftUI only): `View+Extension.swift`, `Transferable+Extension.swift`.
//! `UUID+Extension` (CFUUID) lives in [`uuid`]; `KeyboardShortcut+Extension`
//! is `helpers::keyboard_shortcut`.

// Platform independent.
pub mod array;
pub mod double;
pub mod duration;
pub mod event_modifiers;
pub mod file_handle;
pub mod ns_screen_logic;
pub mod ns_window_logic;
pub mod object_identifier;
pub mod optional;
pub mod os_color;
pub mod string;

// AppKit.
#[cfg(target_vendor = "apple")]
pub mod ns_appearance;
#[cfg(target_vendor = "apple")]
pub mod ns_application;
#[cfg(target_vendor = "apple")]
pub mod ns_color;
#[cfg(target_vendor = "apple")]
pub mod ns_image;
#[cfg(target_vendor = "apple")]
pub mod ns_menu;
#[cfg(target_vendor = "apple")]
pub mod ns_menu_item;
#[cfg(target_vendor = "apple")]
pub mod ns_notification;
#[cfg(target_vendor = "apple")]
pub mod ns_pasteboard;
#[cfg(target_vendor = "apple")]
pub mod ns_screen;
#[cfg(target_vendor = "apple")]
pub mod ns_view;
#[cfg(target_vendor = "apple")]
pub mod ns_window;
#[cfg(target_vendor = "apple")]
pub mod ns_workspace;
#[cfg(target_vendor = "apple")]
pub mod undo_manager;
#[cfg(target_vendor = "apple")]
pub mod user_defaults;
#[cfg(target_vendor = "apple")]
pub mod uuid;

/// `#available(macOS N, *)`: true if the running OS is at least `major`.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn macos_at_least(major: isize) -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: major,
        minorVersion: 0,
        patchVersion: 0,
    })
}

#[cfg(target_vendor = "apple")]
pub mod uti;
