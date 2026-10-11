//! Input-related config enums (ZIG/src/input/config.zig).

/// Determines the macOS option key behavior. See the config
/// `macos-option-as-alt` for a lot more details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(i32)]
pub enum OptionAsAlt {
    #[default]
    False,
    True,
    Left,
    Right,
}
