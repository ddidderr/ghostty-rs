//! A single entry within our SSH entry cache (ZIG/src/cli/ssh-cache/Entry.zig).
//! Our SSH entry cache stores which hosts we've sent our terminfo to so that
//! we don't have to send it again. It doesn't store any sensitive information.

use std::{
    fmt::Write as _,
    time::{SystemTime, UNIX_EPOCH},
};

const S_PER_DAY: i64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub hostname: String,
    pub timestamp: i64,
    pub terminfo_version: String,
}

/// `std.time.timestamp()`: seconds since the unix epoch.
#[must_use]
pub fn now_timestamp() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

impl Entry {
    /// Parse a `hostname|timestamp|terminfo_version` line.
    #[must_use]
    pub fn parse(line: &str) -> Option<Entry> {
        let trimmed = line.trim_matches([' ', '\t', '\r', '\n']);
        if trimmed.is_empty() {
            return None;
        }

        // Parse format: hostname|timestamp|terminfo_version
        // (`tokenizeScalar` skips empty tokens.)
        let mut iter = trimmed.split('|').filter(|s| !s.is_empty());
        let hostname = iter.next()?;
        let timestamp_str = iter.next()?;
        let terminfo_version = iter.next().unwrap_or("xterm-ghostty");
        let timestamp = match ghostty_os::zig_fmt::parse_int::<i64>(timestamp_str.as_bytes(), 10) {
            Ok(t) => t,
            Err(err) => {
                log::warn!("Invalid timestamp in cache entry: {timestamp_str} err={err}");
                return None;
            }
        };

        Some(Entry {
            hostname: hostname.to_string(),
            timestamp,
            terminfo_version: terminfo_version.to_string(),
        })
    }

    /// Write the entry in the on-disk format (`hostname|timestamp|terminfo\n`).
    pub fn format(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "{}|{}|{}",
            self.hostname, self.timestamp, self.terminfo_version
        );
    }

    #[must_use]
    pub fn is_expired(&self, expire_days: Option<u32>) -> bool {
        self.is_expired_at(expire_days, now_timestamp())
    }

    fn is_expired_at(&self, expire_days: Option<u32>, now: i64) -> bool {
        let Some(expire_days) = expire_days else {
            return false;
        };
        let age_days = now.saturating_sub(self.timestamp) / S_PER_DAY;
        age_days > i64::from(expire_days)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn entry(host: &str, timestamp: i64) -> Entry {
        Entry {
            hostname: host.to_string(),
            timestamp,
            terminfo_version: "xterm-ghostty".to_string(),
        }
    }

    #[test]
    fn cache_entry_expiration() {
        let now = now_timestamp();
        let fresh_entry = entry("test.com", now - S_PER_DAY); // 1 day old
        assert!(!fresh_entry.is_expired(Some(90)));

        let old_entry = entry("old.com", now - S_PER_DAY * 100); // 100 days old
        assert!(old_entry.is_expired(Some(90)));

        // Test never-expire case
        assert!(!old_entry.is_expired(None));
    }

    #[test]
    fn cache_entry_expiration_exact_boundary() {
        let now = now_timestamp();
        // Exactly at expiration boundary
        let boundary_entry = entry("example.com", now - S_PER_DAY * 30);
        assert!(!boundary_entry.is_expired_at(Some(30), now));
        assert!(boundary_entry.is_expired_at(Some(29), now));
    }

    #[test]
    fn cache_entry_expiration_large_timestamp() {
        let now = now_timestamp();
        let boundary_entry = entry("example.com", now + S_PER_DAY * 30);
        assert!(!boundary_entry.is_expired(Some(30)));
    }

    #[test]
    fn cache_entry_parsing_valid_formats() {
        let e = Entry::parse("example.com|1640995200|xterm-ghostty").unwrap();
        assert_eq!("example.com", e.hostname);
        assert_eq!(1_640_995_200, e.timestamp);
        assert_eq!("xterm-ghostty", e.terminfo_version);

        // Test default terminfo version
        let e = Entry::parse("test.com|1640995200").unwrap();
        assert_eq!("xterm-ghostty", e.terminfo_version);

        // Test complex hostnames
        let e = Entry::parse("user@server.example.com|1640995200|xterm-ghostty").unwrap();
        assert_eq!("user@server.example.com", e.hostname);
    }

    #[test]
    fn cache_entry_parsing_invalid_formats() {
        assert!(Entry::parse("").is_none());

        // Invalid format (no pipe)
        assert!(Entry::parse("v1").is_none());

        // Missing timestamp
        assert!(Entry::parse("example.com").is_none());

        // Invalid timestamp
        assert!(Entry::parse("example.com|invalid").is_none());

        // Empty terminfo should default
        assert!(Entry::parse("example.com|1640995200|").is_some());
    }

    #[test]
    fn cache_entry_parsing_malformed_data_resilience() {
        // Extra pipes should not break parsing
        assert!(Entry::parse("host|123|term|extra").is_some());

        // Whitespace handling
        assert!(Entry::parse("  host|123|term  ").is_some());
        assert!(Entry::parse("\n").is_none());
        assert!(Entry::parse("   \t  \n").is_none());

        // Extremely large timestamp
        assert!(
            Entry::parse("host|999999999999999999999999999999999999999999999999|xterm-ghostty")
                .is_none()
        );
    }

    #[test]
    fn cache_entry_format_round_trip() {
        let e = entry("a.com", 5);
        let mut s = String::new();
        e.format(&mut s);
        assert_eq!("a.com|5|xterm-ghostty\n", s);
        assert_eq!(Some(e), Entry::parse(&s));
    }
}
