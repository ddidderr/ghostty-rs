//! The C representation of config values (Zig: `config/c_get.zig` together
//! with the `cval` functions of the config types).
//!
//! `ghostty_config_get` writes a value of a key-dependent C type into a
//! caller supplied pointer. This crate cannot use `unsafe`, so the typed
//! value is described by [`CValue`] and the C API crate (`ghostty-lib`) does
//! the actual write.

use ghostty_input::command::CommandC;

/// A size of the quick terminal along one axis in C form
/// (`ghostty_quick_terminal_size_s`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuickTerminalSizeC {
    None,
    Percentage(f32),
    Pixels(u32),
}

/// The value of a config field in the form the C API exposes it.
#[derive(Debug, Clone, PartialEq)]
pub enum CValue {
    /// The type of this field is not supported by the C API (Zig `getValue`
    /// returns false).
    Unsupported,
    /// An optional field without a value (returns false).
    Null,
    /// An optional string without a value: `ghostty_config_get` returns true
    /// and stores a null pointer.
    NullString,
    /// A string (`?[:0]const u8`): stored as a NUL terminated `const char*`.
    String(String),
    /// A boolean (`bool`).
    Bool(bool),
    /// `u8` / `u32`, stored as `c_uint`.
    Uint(u32),
    /// `i16`, stored as `c_short`.
    Short(i16),
    /// `f32`.
    F32(f32),
    /// `f64`.
    F64(f64),
    /// `usize` (`Duration.cval`).
    Usize(usize),
    /// An enum: the tag name, stored as a NUL terminated `const char*`.
    EnumName(&'static str),
    /// A packed struct of at most 32 bits: its bits stored as `c_uint`.
    Packed(u32),
    /// `ghostty_config_color_s`.
    Color { r: u8, g: u8, b: u8 },
    /// `ghostty_config_color_list_s`.
    ColorList(Vec<[u8; 3]>),
    /// `ghostty_config_palette_s`.
    Palette(Box<[[u8; 3]; 256]>),
    /// `ghostty_config_command_list_s`.
    Commands(Vec<CommandC>),
    /// `ghostty_config_quick_terminal_size_s`.
    QuickTerminalSize {
        primary: QuickTerminalSizeC,
        secondary: QuickTerminalSizeC,
    },
    /// `ghostty_config_path_s`.
    Path { path: String, optional: bool },
}
