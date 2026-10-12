//! `QuickTerminalSize.swift`: the `quick-terminal-size` value.

use libghostty::types::{
    ghostty_config_quick_terminal_size_s,
    ghostty_quick_terminal_size_s,
    quick_terminal_size_tag,
};

use super::QuickTerminalPosition;

/// A width/height in points (`CGSize`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size2 {
    pub width: f64,
    pub height: f64,
}

/// `QuickTerminalSize.Size`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Percentage(f32),
    Pixels(u32),
}

impl Size {
    /// `init?(from: ghostty_quick_terminal_size_s)`.
    #[must_use]
    pub fn from_c(c: &ghostty_quick_terminal_size_s) -> Option<Self> {
        match c.tag {
            quick_terminal_size_tag::PERCENTAGE => {
                // SAFETY: `tag` says the `percentage` member is the active one.
                #[allow(unsafe_code)]
                Some(Self::Percentage(unsafe { c.value.percentage }))
            }
            quick_terminal_size_tag::PIXELS => {
                // SAFETY: `tag` says the `pixels` member is the active one.
                #[allow(unsafe_code)]
                Some(Self::Pixels(unsafe { c.value.pixels }))
            }
            // NONE (and anything unknown: Swift asserts and returns nil).
            _ => None,
        }
    }

    #[must_use]
    pub fn to_pixels(self, parent_dimension: f64) -> f64 {
        match self {
            Self::Percentage(v) => parent_dimension * f64::from(v) / 100.0,
            Self::Pixels(v) => f64::from(v),
        }
    }
}

/// `struct QuickTerminalSize`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct QuickTerminalSize {
    pub primary: Option<Size>,
    pub secondary: Option<Size>,
}

impl QuickTerminalSize {
    #[must_use]
    pub fn from_c(c: &ghostty_config_quick_terminal_size_s) -> Self {
        Self {
            primary: Size::from_c(&c.primary),
            secondary: Size::from_c(&c.secondary),
        }
    }

    /// An almost direct port of the Zig function `QuickTerminalSize.calculate`.
    #[must_use]
    pub fn calculate(&self, position: QuickTerminalPosition, screen: Size2) -> Size2 {
        let dims = screen;
        match position {
            QuickTerminalPosition::Left | QuickTerminalPosition::Right => Size2 {
                width: self.primary.map_or(400.0, |s| s.to_pixels(dims.width)),
                height: self
                    .secondary
                    .map_or(dims.height, |s| s.to_pixels(dims.height)),
            },
            QuickTerminalPosition::Top | QuickTerminalPosition::Bottom => Size2 {
                width: self
                    .secondary
                    .map_or(dims.width, |s| s.to_pixels(dims.width)),
                height: self.primary.map_or(400.0, |s| s.to_pixels(dims.height)),
            },
            QuickTerminalPosition::Center => {
                if dims.width >= dims.height {
                    // Landscape
                    Size2 {
                        width: self.primary.map_or(800.0, |s| s.to_pixels(dims.width)),
                        height: self.secondary.map_or(400.0, |s| s.to_pixels(dims.height)),
                    }
                } else {
                    // Portrait
                    Size2 {
                        width: self.secondary.map_or(400.0, |s| s.to_pixels(dims.width)),
                        height: self.primary.map_or(800.0, |s| s.to_pixels(dims.height)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Size2 = Size2 {
        width: 1000.0,
        height: 600.0,
    };

    #[test]
    fn defaults_per_position() {
        let s = QuickTerminalSize::default();
        assert_eq!(
            s.calculate(QuickTerminalPosition::Top, SCREEN),
            Size2 {
                width: 1000.0,
                height: 400.0
            }
        );
        assert_eq!(
            s.calculate(QuickTerminalPosition::Left, SCREEN),
            Size2 {
                width: 400.0,
                height: 600.0
            }
        );
        assert_eq!(
            s.calculate(QuickTerminalPosition::Center, SCREEN),
            Size2 {
                width: 800.0,
                height: 400.0
            }
        );
        let portrait = Size2 {
            width: 600.0,
            height: 1000.0,
        };
        assert_eq!(
            s.calculate(QuickTerminalPosition::Center, portrait),
            Size2 {
                width: 400.0,
                height: 800.0
            }
        );
    }

    #[test]
    fn explicit_sizes() {
        let s = QuickTerminalSize {
            primary: Some(Size::Percentage(50.0)),
            secondary: Some(Size::Pixels(300)),
        };
        assert_eq!(
            s.calculate(QuickTerminalPosition::Bottom, SCREEN),
            Size2 {
                width: 300.0,
                height: 300.0
            }
        );
        assert_eq!(
            s.calculate(QuickTerminalPosition::Right, SCREEN),
            Size2 {
                width: 500.0,
                height: 300.0
            }
        );
    }
}
