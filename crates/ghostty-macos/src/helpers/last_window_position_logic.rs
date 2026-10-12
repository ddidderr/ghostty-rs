//! The frame computation of `Helpers/LastWindowPosition.swift`'s `restore`,
//! independent of AppKit.

use super::geometry::Rect;

/// `UserDefaults` key the position is stored under.
pub const POSITION_KEY: &str = "NSWindowLastPosition";

/// `[x, y, width, height]` as stored in defaults.
#[must_use]
pub fn encode(frame: &Rect) -> [f64; 4] {
    [
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    ]
}

/// The new frame for `window_frame` given the saved `values`, or `None` if
/// there was nothing to restore.
#[must_use]
pub fn restored_frame(
    values: &[f64],
    window_frame: Rect,
    visible_frame: Rect,
    restore_origin: bool,
    restore_size: bool,
) -> Option<Rect> {
    if !(restore_origin || restore_size) || values.len() < 2 {
        return None;
    }

    let mut new_frame = window_frame;
    if restore_origin {
        new_frame.origin.x = values[0];
        new_frame.origin.y = values[1];
    }

    if restore_size && values.len() >= 4 {
        new_frame.size.width = values[2].min(visible_frame.width());
        new_frame.size.height = values[3].min(visible_frame.height());
    }

    // If the new frame is not constrained to the visible screen, shift it a
    // little before AppKit does so the correct size is saved beforehand.
    if restore_origin && !visible_frame.contains_rect(&new_frame) {
        new_frame.origin.x = visible_frame
            .min_x()
            .max((visible_frame.max_x() - new_frame.width()).min(new_frame.origin.x));
        new_frame.origin.y = visible_frame
            .min_y()
            .max((visible_frame.max_y() - new_frame.height()).min(new_frame.origin.y));
    }

    Some(new_frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VISIBLE: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn nothing_to_restore() {
        let w = Rect::new(5.0, 5.0, 100.0, 100.0);
        assert!(restored_frame(&[1.0, 2.0], w, VISIBLE, false, false).is_none());
        assert!(restored_frame(&[1.0], w, VISIBLE, true, true).is_none());
    }

    #[test]
    fn restores_origin_only() {
        let w = Rect::new(5.0, 5.0, 100.0, 100.0);
        let r = restored_frame(&[10.0, 20.0, 300.0, 300.0], w, VISIBLE, true, false).unwrap();
        assert_eq!(r, Rect::new(10.0, 20.0, 100.0, 100.0));
    }

    #[test]
    fn size_is_clamped_and_origin_shifted_onscreen() {
        let w = Rect::new(0.0, 0.0, 100.0, 100.0);
        let r = restored_frame(&[900.0, 700.0, 5000.0, 400.0], w, VISIBLE, true, true).unwrap();
        assert_eq!(r.size.width, 1000.0);
        assert_eq!(r.size.height, 400.0);
        assert_eq!(r.origin.x, 0.0);
        assert_eq!(r.origin.y, 400.0);
    }

    #[test]
    fn encode_roundtrip() {
        assert_eq!(encode(&Rect::new(1.0, 2.0, 3.0, 4.0)), [1.0, 2.0, 3.0, 4.0]);
    }
}
