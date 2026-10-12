//! Duration formatting used by `Ghostty.App.commandFinished` notifications.
//! Portable (and tested) so the formatting rules are checked on every target.

use std::time::Duration;

/// `Duration.formatted(.units(allowed: [.hours, .minutes, .seconds,
/// .milliseconds], width: .abbreviated, fractionalPart: .hide))` in the en_US
/// locale: zero-valued units are hidden, units are joined with ", " and use
/// the abbreviations `hr`, `min`, `sec`, `ms` (e.g. "1 hr, 2 min, 3 sec").
/// A zero duration formats as "0 ms" (the smallest allowed unit). Sub
/// millisecond remainders are hidden (truncated).
///
/// Note: the exact rounding of `fractionalPart: .hide` and non-English locales
/// are Foundation behaviour that cannot be verified off macOS.
#[must_use]
pub fn format_duration_abbreviated(duration: Duration) -> String {
    let total_ms = duration.as_millis();
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms / 60_000) % 60;
    let seconds = (total_ms / 1000) % 60;
    let millis = total_ms % 1000;
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours} hr"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    if seconds > 0 {
        parts.push(format!("{seconds} sec"));
    }
    if millis > 0 || parts.is_empty() {
        parts.push(format!("{millis} ms"));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration_abbreviated(Duration::from_secs(5)), "5 sec");
        assert_eq!(
            format_duration_abbreviated(Duration::from_secs(3723)),
            "1 hr, 2 min, 3 sec"
        );
        assert_eq!(
            format_duration_abbreviated(Duration::from_millis(1500)),
            "1 sec, 500 ms"
        );
        assert_eq!(
            format_duration_abbreviated(Duration::from_secs(60)),
            "1 min"
        );
        assert_eq!(format_duration_abbreviated(Duration::ZERO), "0 ms");
        // Sub-millisecond remainders are hidden.
        assert_eq!(
            format_duration_abbreviated(Duration::from_nanos(2_000_999_999)),
            "2 sec"
        );
    }
}
