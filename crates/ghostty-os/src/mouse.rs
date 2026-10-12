/// The system-configured double-click interval if its available.
///
/// Only macOS can report this; elsewhere this is always `None`.
#[cfg(not(target_os = "macos"))]
pub fn click_interval() -> Option<u32> {
    None
}

/// Convert an `NSEvent.doubleClickInterval` (seconds) to milliseconds,
/// rounding up like Zig's `@ceil(interval * 1000)`.
pub fn click_interval_ms(seconds: f64) -> u32 {
    (seconds * 1000.0).ceil() as u32
}

/// The system-configured double-click interval (`NSEvent.doubleClickInterval`).
#[cfg(target_os = "macos")]
pub fn click_interval() -> Option<u32> {
    Some(click_interval_ms(
        objc2_app_kit::NSEvent::doubleClickInterval(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_interval_rounds_up() {
        assert_eq!(click_interval_ms(0.5), 500);
        assert_eq!(click_interval_ms(0.2501), 251);
        assert_eq!(click_interval_ms(0.0), 0);
    }
}
