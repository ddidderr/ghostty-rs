//! The frame math of `NSWindow.constrainToScreen()`
//! (`Helpers/Extensions/NSWindow+Extension.swift`).

use crate::helpers::geometry::Rect;

/// The frame `window_frame` constrained to `visible_frame` in size and
/// origin, or `None` if it already is (no `setFrame` needed).
#[must_use]
pub fn constrained_frame(window_frame: Rect, visible_frame: Rect) -> Option<Rect> {
    let mut f = window_frame;

    f.size.width = f.size.width.min(visible_frame.size.width);
    f.size.height = f.size.height.min(visible_frame.size.height);

    f.origin.x = visible_frame
        .min_x()
        .max(f.origin.x.min(visible_frame.max_x() - f.width()));
    f.origin.y = visible_frame
        .min_y()
        .max(f.origin.y.min(visible_frame.max_y() - f.height()));

    (f != window_frame).then_some(f)
}

/// `cgWindowId`: the window number if it has a window device (> 0).
#[must_use]
pub fn cg_window_id(window_number: isize) -> Option<u32> {
    if window_number > 0 {
        u32::try_from(window_number).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn already_inside() {
        assert!(constrained_frame(Rect::new(10.0, 10.0, 200.0, 200.0), V).is_none());
    }

    #[test]
    fn shrinks_and_moves() {
        let f = constrained_frame(Rect::new(-50.0, 700.0, 2000.0, 100.0), V).unwrap();
        assert_eq!(f, Rect::new(0.0, 700.0, 1000.0, 100.0));
        let f = constrained_frame(Rect::new(900.0, 750.0, 200.0, 200.0), V).unwrap();
        assert_eq!(f, Rect::new(800.0, 600.0, 200.0, 200.0));
    }

    #[test]
    fn window_ids() {
        assert_eq!(cg_window_id(0), None);
        assert_eq!(cg_window_id(-1), None);
        assert_eq!(cg_window_id(42), Some(42));
    }
}
