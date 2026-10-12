//! The platform independent parts of `Helpers/Extensions/NSScreen+Extension.swift`
//! (and the Swift `NSScreenTests`).

use crate::helpers::geometry::{Point, Rect, Size};

/// `NSScreen.origin(fromTopLeftOffsetX:offsetY:windowSize:)`: converts
/// top-left offset coordinates to bottom-left origin coordinates for window
/// positioning, relative to the screen's `visibleFrame`.
#[must_use]
pub fn origin_from_top_left_offset(
    visible_frame: Rect,
    x: f64,
    y: f64,
    window_size: Size,
) -> Point {
    // Convert top-left coordinates to bottom-left origin
    Point {
        x: visible_frame.min_x() + x,
        y: visible_frame.max_y() - y - window_size.height,
    }
}

/// The geometry part of `NSScreen.hasDock` (after the autohide default check).
#[must_use]
pub fn has_visible_dock(
    visible_frame: Rect,
    frame: Rect,
    menu_height: f64,
    notch_inset: f64,
) -> bool {
    // If our visible width is less than the frame we assume its the dock.
    if visible_frame.width() < frame.width() {
        return true;
    }

    // We need to see if our visible frame height is less than the full
    // screen height minus the menu and notch and such.
    let boundary_area_padding = 5.0;
    visible_frame.height() < (frame.height() - menu_height.max(notch_inset) - boundary_area_padding)
}

/// `NSScreen.hasNotch`: a top safe area inset is assumed to mean a notch.
#[must_use]
pub fn has_notch(safe_area_top: f64) -> bool {
    safe_area_top > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Size = Size {
        width: 400.0,
        height: 300.0,
    };

    #[test]
    fn positive_coordinate_conversion() {
        let o = origin_from_top_left_offset(Rect::new(0.0, 100.0, 1000.0, 800.0), 15.0, 15.0, WIN);
        assert_eq!((o.x, o.y), (15.0, 585.0));
    }

    #[test]
    fn zero_coordinates() {
        let o = origin_from_top_left_offset(Rect::new(0.0, 100.0, 1000.0, 800.0), 0.0, 0.0, WIN);
        assert_eq!((o.x, o.y), (0.0, 600.0));
    }

    #[test]
    fn offset_screen() {
        let o = origin_from_top_left_offset(
            Rect::new(1440.0, 0.0, 1920.0, 1080.0),
            100.0,
            50.0,
            Size {
                width: 600.0,
                height: 400.0,
            },
        );
        assert_eq!((o.x, o.y), (1540.0, 630.0));
    }

    #[test]
    fn large_coordinates() {
        let o = origin_from_top_left_offset(Rect::new(0.0, 0.0, 1920.0, 1080.0), 500.0, 200.0, WIN);
        assert_eq!((o.x, o.y), (500.0, 580.0));
    }

    #[test]
    fn dock_detection() {
        let frame = Rect::new(0.0, 0.0, 1000.0, 800.0);
        // side dock
        assert!(has_visible_dock(
            Rect::new(70.0, 0.0, 930.0, 776.0),
            frame,
            24.0,
            0.0
        ));
        // bottom dock
        assert!(has_visible_dock(
            Rect::new(0.0, 70.0, 1000.0, 706.0),
            frame,
            24.0,
            0.0
        ));
        // no dock, only menu bar
        assert!(!has_visible_dock(
            Rect::new(0.0, 0.0, 1000.0, 776.0),
            frame,
            24.0,
            0.0
        ));
        // notch inset larger than the menu
        assert!(!has_visible_dock(
            Rect::new(0.0, 0.0, 1000.0, 760.0),
            frame,
            24.0,
            38.0
        ));
    }

    #[test]
    fn notch() {
        assert!(has_notch(32.0));
        assert!(!has_notch(0.0));
    }
}
