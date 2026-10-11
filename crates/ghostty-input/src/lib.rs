//! ghostty-input: keyboard/mouse input handling (ZIG/src/input/*).

pub mod binding;
pub mod command;
pub mod config;
pub mod function_keys;
pub mod helpgen_actions;
pub mod key;
pub mod key_encode;
pub mod key_mods;
pub mod keyboard;
pub mod keycodes;
pub mod kitty;
pub mod link;
pub mod mouse;
pub mod paste;

pub use config::OptionAsAlt;
pub use key::{Action, Key, KeyEvent, ctrl_or_super};
pub use key_mods::{Mods, RemapSet as KeyRemapSet};
pub use keyboard::Layout as KeyboardLayout;
pub use link::Link;
pub use mouse::{
    Button as MouseButton, ButtonState as MouseButtonState, PressureStage as MousePressureStage,
    ScrollMods,
};
