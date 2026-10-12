//! `Sources/App/macOS`: `main.swift`, `AppDelegate.swift`,
//! `AppDelegate+Ghostty.swift` and `MainMenu.xib`.
#[cfg(target_vendor = "apple")]
pub mod app_delegate;
#[cfg(target_vendor = "apple")]
pub mod app_delegate_ghostty;
#[cfg(target_vendor = "apple")]
pub mod entry;
#[cfg(target_vendor = "apple")]
pub mod main_menu;
pub mod menu_spec;
#[cfg(target_vendor = "apple")]
pub mod support;
