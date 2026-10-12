//! Port of `Metrics.Modifier` from `font/Metrics.zig`.
//!
//! The font crate depends on this crate (not the other way around), so the
//! modifier type used by the `adjust-*` config options lives here.

use std::fmt;

use crate::{
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

/// A modifier to apply to a metrics value. The modifier value represents
/// a delta, so percent is a percentage to change, not a percentage of.
/// For example, "20%" is 20% larger, not 20% of the value. Likewise,
/// an absolute value of "20" is 20 larger, not literally 20.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricModifier {
    Percent(f64),
    Absolute(i32),
}

impl Default for MetricModifier {
    fn default() -> Self {
        MetricModifier::Absolute(0)
    }
}

impl MetricModifier {
    /// Parses the modifier value. If the value ends in "%" it is assumed
    /// to be a percent, otherwise the value is parsed as an integer.
    pub fn parse(input: &str) -> Result<MetricModifier, ArgError> {
        if input.is_empty() {
            return Err(ArgError::other("InvalidFormat"));
        }

        if let Some(num) = input.strip_suffix('%') {
            let mut percent = num
                .parse::<f64>()
                .map_err(|_| ArgError::other("InvalidFormat"))?;
            percent /= 100.0;

            if percent <= -1.0 {
                return Ok(MetricModifier::Percent(0.0));
            }
            return Ok(MetricModifier::Percent(1.0 + percent));
        }

        input
            .parse::<i32>()
            .map(MetricModifier::Absolute)
            .map_err(|_| ArgError::other("InvalidFormat"))
    }

    /// So it works with the config framework.
    pub fn parse_cli_value(input: Option<&str>) -> Result<MetricModifier, ArgError> {
        Self::parse(input.ok_or(ArgError::ValueRequired)?)
    }

    /// Apply a modifier to an integer value (saturating, clamped at zero for
    /// unsigned results, like the Zig implementation for unsigned types).
    #[must_use]
    pub fn apply_i64(&self, v: i64, unsigned: bool) -> i64 {
        match *self {
            MetricModifier::Percent(p) => {
                let p = p.max(0.0);
                (v as f64 * p).round() as i64
            }
            MetricModifier::Absolute(abs) => {
                let applied = v.saturating_add(i64::from(abs));
                if unsigned { applied.max(0) } else { applied }
            }
        }
    }

    /// Apply to a `u32`.
    #[must_use]
    pub fn apply_u32(&self, v: u32) -> u32 {
        let r = self.apply_i64(i64::from(v), true);
        u32::try_from(r).unwrap_or(u32::MAX)
    }

    /// Apply to an `i32`.
    #[must_use]
    pub fn apply_i32(&self, v: i32) -> i32 {
        let r = self.apply_i64(i64::from(v), false);
        // Mirrors Zig: out-of-range yields maxInt * sign.
        i32::try_from(r).unwrap_or(if r < 0 { -i32::MAX } else { i32::MAX })
    }

    /// Apply to a float value.
    #[must_use]
    pub fn apply_f64(&self, v: f64) -> f64 {
        match *self {
            MetricModifier::Percent(p) => v * p.max(0.0),
            MetricModifier::Absolute(abs) => v + f64::from(abs),
        }
    }
}

impl ConfigField for MetricModifier {
    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        let value = crate::cli_args::text_value(value)?;
        *self = Self::parse_cli_value(value)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let s = match self {
            MetricModifier::Percent(v) => format!("{}%", (v - 1.0) * 100.0),
            MetricModifier::Absolute(v) => format!("{v}"),
        };
        f.format_entry(&s)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fmt1(m: MetricModifier) -> String {
        let mut s = String::new();
        crate::formatter::format_entry("a", &m, &mut s).unwrap();
        s
    }

    #[test]
    fn modifier_parse_absolute() {
        assert_eq!(
            MetricModifier::parse("100").unwrap(),
            MetricModifier::Absolute(100)
        );
        assert_eq!(
            MetricModifier::parse("-100").unwrap(),
            MetricModifier::Absolute(-100)
        );
    }

    #[test]
    fn modifier_parse_percent() {
        assert_eq!(
            MetricModifier::parse("20%").unwrap(),
            MetricModifier::Percent(1.2)
        );
        assert_eq!(
            MetricModifier::parse("-20%").unwrap(),
            MetricModifier::Percent(0.8)
        );
        assert_eq!(
            MetricModifier::parse("0%").unwrap(),
            MetricModifier::Percent(1.0)
        );
    }

    #[test]
    fn modifier_percent() {
        assert_eq!(MetricModifier::Percent(0.8).apply_u32(100), 80);
        assert_eq!(MetricModifier::Percent(1.8).apply_u32(100), 180);
    }

    #[test]
    fn modifier_absolute() {
        assert_eq!(MetricModifier::Absolute(-100).apply_u32(100), 0);
        assert_eq!(MetricModifier::Absolute(-120).apply_u32(100), 0);
        assert_eq!(MetricModifier::Absolute(100).apply_u32(100), 200);
        assert_eq!(MetricModifier::Absolute(-100).apply_i32(100), 0);
        assert_eq!(MetricModifier::Absolute(-120).apply_i32(100), -20);
    }

    #[test]
    fn format_config_percent() {
        let mut m = MetricModifier::default();
        m.parse_cli(Some("24%")).unwrap();
        assert_eq!(fmt1(m), "a = 24%\n");
    }

    #[test]
    fn format_config_absolute() {
        let mut m = MetricModifier::default();
        m.parse_cli(Some("-30")).unwrap();
        assert_eq!(fmt1(m), "a = -30\n");
    }
}
