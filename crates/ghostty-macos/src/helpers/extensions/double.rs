//! `Helpers/Extensions/Double+Extension.swift`.

use std::ops::RangeInclusive;

pub trait DoubleExt {
    /// `clamped(to:)`.
    fn clamped_to(self, range: RangeInclusive<f64>) -> f64;
}

impl DoubleExt for f64 {
    fn clamped_to(self, range: RangeInclusive<f64>) -> f64 {
        self.max(*range.start()).min(*range.end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps() {
        assert_eq!(5.0.clamped_to(0.0..=1.0), 1.0);
        assert_eq!((-5.0).clamped_to(0.0..=1.0), 0.0);
        assert_eq!(0.5.clamped_to(0.0..=1.0), 0.5);
    }
}
