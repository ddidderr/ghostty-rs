//! `Sources/Features/<Area>`: one submodule per Swift feature folder.
//!
//! Each area is owned by one agent (see `docs/rewrite/notes/macos-app-design.md`);
//! until it is ported the module holds only the surface that the foundation
//! (`app`, `ghostty`) calls into, marked `TODO(<owner>)`.
pub mod about;
pub mod app_intents;
pub mod applescript;
pub mod clipboard_confirmation;
pub mod command_palette;
pub mod custom_app_icon;
pub mod global_keybinds;
pub mod quick_terminal;
pub mod secure_input;
#[cfg(target_vendor = "apple")]
pub mod services;
pub mod settings;
pub mod splits;
pub mod terminal;
pub mod update;
