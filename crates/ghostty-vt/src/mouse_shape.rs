//! Mouse cursor shapes (ZIG/src/terminal/mouse_shape.zig).

/// The possible cursor shapes. Not all app runtimes support these shapes.
/// The shapes are always based on the W3C supported cursor styles so we
/// can have a cross platform list.
// Must be kept in sync with ghostty_cursor_shape_e
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(i32)]
pub enum MouseShape {
    #[default]
    Default,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    AllScroll,
    ColResize,
    RowResize,
    NResize,
    EResize,
    SResize,
    WResize,
    NeResize,
    NwResize,
    SeResize,
    SwResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ZoomIn,
    ZoomOut,
}

impl MouseShape {
    /// Build cursor shape from string or None if its unknown.
    pub fn from_string(v: &str) -> Option<MouseShape> {
        Some(match v {
            "default" => MouseShape::Default,
            "context-menu" => MouseShape::ContextMenu,
            "help" => MouseShape::Help,
            "pointer" => MouseShape::Pointer,
            "progress" => MouseShape::Progress,
            "wait" => MouseShape::Wait,
            "cell" => MouseShape::Cell,
            "crosshair" => MouseShape::Crosshair,
            "text" => MouseShape::Text,
            "vertical-text" => MouseShape::VerticalText,
            "alias" => MouseShape::Alias,
            "copy" => MouseShape::Copy,
            "move" => MouseShape::Move,
            "no-drop" => MouseShape::NoDrop,
            "not-allowed" => MouseShape::NotAllowed,
            "grab" => MouseShape::Grab,
            "grabbing" => MouseShape::Grabbing,
            "all-scroll" => MouseShape::AllScroll,
            "col-resize" => MouseShape::ColResize,
            "row-resize" => MouseShape::RowResize,
            "n-resize" => MouseShape::NResize,
            "e-resize" => MouseShape::EResize,
            "s-resize" => MouseShape::SResize,
            "w-resize" => MouseShape::WResize,
            "ne-resize" => MouseShape::NeResize,
            "nw-resize" => MouseShape::NwResize,
            "se-resize" => MouseShape::SeResize,
            "sw-resize" => MouseShape::SwResize,
            "ew-resize" => MouseShape::EwResize,
            "ns-resize" => MouseShape::NsResize,
            "nesw-resize" => MouseShape::NeswResize,
            "nwse-resize" => MouseShape::NwseResize,
            "zoom-in" => MouseShape::ZoomIn,
            "zoom-out" => MouseShape::ZoomOut,
            "left_ptr" => MouseShape::Default,
            "question_arrow" => MouseShape::Help,
            "hand" => MouseShape::Pointer,
            "left_ptr_watch" => MouseShape::Progress,
            "watch" => MouseShape::Wait,
            "cross" => MouseShape::Crosshair,
            "xterm" => MouseShape::Text,
            "dnd-link" => MouseShape::Alias,
            "dnd-copy" => MouseShape::Copy,
            "dnd-move" => MouseShape::Move,
            "dnd-no-drop" => MouseShape::NoDrop,
            "crossed_circle" => MouseShape::NotAllowed,
            "hand1" => MouseShape::Grab,
            "right_side" => MouseShape::EResize,
            "top_side" => MouseShape::NResize,
            "top_right_corner" => MouseShape::NeResize,
            "top_left_corner" => MouseShape::NwResize,
            "bottom_side" => MouseShape::SResize,
            "bottom_right_corner" => MouseShape::SeResize,
            "bottom_left_corner" => MouseShape::SwResize,
            "left_side" => MouseShape::WResize,
            "fleur" => MouseShape::AllScroll,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_shape_from_string() {
        assert_eq!(
            MouseShape::from_string("default"),
            Some(MouseShape::Default)
        );
        assert_eq!(MouseShape::from_string("xterm"), Some(MouseShape::Text));
        assert_eq!(MouseShape::from_string("nope"), None);
    }
}
