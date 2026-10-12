//! Single source of truth for the build version metadata (Zig:
//! `build_config.zig` `version`, `version_string`, `release_channel`, plus
//! `builtin.mode`).
//!
//! Everything is derived at compile time by this crate's `build.rs` (git
//! describe / workspace `app-version`, overridable with the
//! `GHOSTTY_VERSION_STRING` environment variable). Consumers (XTVERSION,
//! `TERM_PROGRAM_VERSION`, the About dialog, crash metadata, `+version`) must
//! use these constants instead of hardcoding a version.

/// `build_config.version_string`, e.g. `1.3.1` for tagged releases or
/// `1.3.1-main+abc1234` for development builds.
pub const VERSION_STRING: &str = env!("GHOSTTY_VERSION_STRING");

/// The git commit hash (`build_config.version.build`), if the version has
/// one (tagged releases do not).
pub const COMMIT: Option<&str> = {
    let c = env!("GHOSTTY_COMMIT");
    if c.is_empty() { None } else { Some(c) }
};

/// `build_config.release_channel` tag name: `stable` or `tip`.
pub const RELEASE_CHANNEL: &str = env!("GHOSTTY_RELEASE_CHANNEL");

/// `{}` of `builtin.mode` (`.Debug`, `.ReleaseSafe`, `.ReleaseFast`).
pub const BUILD_MODE: &str = env!("GHOSTTY_BUILD_MODE");

/// A semantic version (Zig: `std.SemanticVersion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticVersion<'a> {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Pre-release part (after `-`), e.g. `main`.
    pub pre: Option<&'a str>,
    /// Build metadata (after `+`), e.g. the commit hash.
    pub build: Option<&'a str>,
}

/// Parse `X.Y.Z[-pre][+build]`.
pub fn parse(v: &str) -> Option<SemanticVersion<'_>> {
    let (rest, build) = match v.split_once('+') {
        Some((a, b)) => (a, Some(b)),
        None => (v, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(SemanticVersion {
        major,
        minor,
        patch,
        pre,
        build,
    })
}

/// `build_config.version` parsed from [`VERSION_STRING`].
pub fn version() -> Option<SemanticVersion<'static>> {
    parse(VERSION_STRING)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn version_string_is_semver() {
        let v = version().expect("derived version must be semantic");
        assert!(v.major >= 1);
        assert_eq!(v.build, COMMIT.filter(|_| v.build.is_some()));
    }

    #[test]
    fn channel_matches_prerelease() {
        let v = version().unwrap();
        assert_eq!(RELEASE_CHANNEL == "tip", v.pre.is_some());
    }

    #[test]
    fn parse_cases() {
        assert_eq!(
            parse("1.3.1"),
            Some(SemanticVersion {
                major: 1,
                minor: 3,
                patch: 1,
                pre: None,
                build: None
            })
        );
        let v = parse("1.4.0-main+abc1234").unwrap();
        assert_eq!((v.pre, v.build), (Some("main"), Some("abc1234")));
        assert_eq!(parse("1.3"), None);
        assert_eq!(parse("1.3.1.2"), None);
    }
}
