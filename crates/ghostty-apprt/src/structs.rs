//! Port of `structs.zig`.

/// ContentScale is the ratio between the current DPI and the platform's
/// default DPI. This is used to determine how much certain rendered elements
/// need to be scaled up or down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentScale {
    pub x: f32,
    pub y: f32,
}

/// The size of the surface in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

/// The position of the cursor in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorPos {
    pub x: f32,
    pub y: f32,
}

/// Input Method Editor (IME) position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImePos {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The clipboard type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Clipboard {
    /// ctrl+c/v
    Standard = 0,
    Selection = 1,
    Primary = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardContent {
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRequestType {
    Paste,
    Osc52Read,
    Osc52Write,
}

/// Clipboard request. This is used to request clipboard contents and must
/// be sent as a response to a ClipboardRequest event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRequest {
    /// A direct paste of clipboard contents.
    Paste,

    /// A request to read clipboard contents via OSC 52.
    Osc52Read(Clipboard),

    /// A request to write clipboard contents via OSC 52.
    Osc52Write(Clipboard),
}

impl ClipboardRequest {
    #[must_use]
    pub fn request_type(&self) -> ClipboardRequestType {
        match self {
            Self::Paste => ClipboardRequestType::Paste,
            Self::Osc52Read(_) => ClipboardRequestType::Osc52Read,
            Self::Osc52Write(_) => ClipboardRequestType::Osc52Write,
        }
    }
}

/// The color scheme in use (light vs dark).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ColorScheme {
    Light = 0,
    Dark = 1,
}

/// Selection information
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Top-left point of the selection in the viewport in scaled
    /// window pixels. (0,0) is the top-left of the window.
    pub tl_x_px: f64,
    pub tl_y_px: f64,

    /// The offset of the selection start in cells from the top-left
    /// of the viewport.
    ///
    /// This is a strange metric but its used by macOS.
    pub offset_start: u32,
    pub offset_len: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_values() {
        assert_eq!(Clipboard::Standard as u8, 0);
        assert_eq!(Clipboard::Selection as u8, 1);
        assert_eq!(Clipboard::Primary as u8, 2);
        assert_eq!(ColorScheme::Light as u8, 0);
        assert_eq!(ColorScheme::Dark as u8, 1);
    }

    #[test]
    fn clipboard_request_type() {
        assert_eq!(
            ClipboardRequest::Osc52Read(Clipboard::Primary).request_type(),
            ClipboardRequestType::Osc52Read
        );
        assert_eq!(
            ClipboardRequest::Paste.request_type(),
            ClipboardRequestType::Paste
        );
    }
}
