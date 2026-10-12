//! `Helpers/Extensions/Duration+Extension.swift`.

use std::time::Duration;

pub trait DurationExt {
    /// `Duration.timeInterval` (seconds as a double).
    fn time_interval(&self) -> f64;
}

impl DurationExt for Duration {
    fn time_interval(&self) -> f64 {
        self.as_secs() as f64 + f64::from(self.subsec_nanos()) / 1_000_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds() {
        assert_eq!(Duration::from_millis(1500).time_interval(), 1.5);
        assert_eq!(Duration::ZERO.time_interval(), 0.0);
    }
}
