//! Mouse input types (ZIG/src/input/mouse.zig).

/// The state of a mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ButtonState {
    Release,
    Press,
}

/// Possible mouse buttons. We only track up to 11 because that's the maximum
/// button input that terminal mouse tracking handles without becoming
/// ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Button {
    Unknown = 0,
    Left = 1,
    Right = 2,
    Middle = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Eleven = 11,
}

impl Button {
    /// The maximum value in this enum. This can be used to create a densely
    /// packed array, for example.
    pub const MAX: i32 = 11;
}

/// The "momentum" of a mouse scroll event. This matches the macOS events
/// because it is the only reliable source right now of momentum events.
/// This is used to handle "inertial scrolling" (i.e. flicking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Momentum {
    #[default]
    None = 0,
    Began = 1,
    Stationary = 2,
    Changed = 3,
    Ended = 4,
    Cancelled = 5,
    MayBegin = 6,
}

impl Momentum {
    pub fn from_int(v: u8) -> Option<Momentum> {
        Some(match v {
            0 => Momentum::None,
            1 => Momentum::Began,
            2 => Momentum::Stationary,
            3 => Momentum::Changed,
            4 => Momentum::Ended,
            5 => Momentum::Cancelled,
            6 => Momentum::MayBegin,
            _ => return None,
        })
    }
}

/// The pressure stage of a pressure-sensitive input device.
///
/// This currently only supports the stages that macOS supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum PressureStage {
    /// The input device is unpressed.
    #[default]
    None = 0,

    /// The input device is pressed a normal amount. On macOS trackpads,
    /// this is after a "click".
    Normal = 1,

    /// The input device is pressed a deep amount. On macOS trackpads,
    /// this is after a "force click".
    Deep = 2,
}

/// The bitmask for mods for scroll events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScrollMods {
    /// True if this is a high-precision scroll event. For example, Apple
    /// devices such as Magic Mouse, trackpads, etc. are high-precision
    /// and send very detailed scroll events.
    pub precision: bool,

    /// The momentum phase (if available, supported) of the scroll event.
    /// This is used to handle "inertial scrolling" (i.e. flicking).
    pub momentum: Momentum,
}

impl ScrollMods {
    /// Packed `u8` representation (bit 0 precision, bits 1..4 momentum).
    pub fn int(self) -> u8 {
        u8::from(self.precision) | ((self.momentum as u8) << 1)
    }

    pub fn from_int(v: u8) -> ScrollMods {
        ScrollMods {
            precision: v & 1 != 0,
            momentum: Momentum::from_int((v >> 1) & 0b111).unwrap_or(Momentum::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // For our own understanding
    #[test]
    fn scroll_mods_layout() {
        assert_eq!(ScrollMods::default().int(), 0b0);
        assert_eq!(
            ScrollMods {
                precision: true,
                ..Default::default()
            }
            .int(),
            0b0000_0001
        );
        assert_eq!(ScrollMods::from_int(0b0000_0011).momentum, Momentum::Began);
    }
}
