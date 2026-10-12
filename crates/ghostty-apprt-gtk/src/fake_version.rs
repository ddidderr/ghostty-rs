//! Debug-build-only override of the runtime library versions, used to
//! exercise the version-gated compatibility fallbacks on a machine that only
//! has new GTK/libadwaita installed (there is no Zig counterpart).
//!
//! `GHOSTTY_DEBUG_FAKE_GTK_VERSION=4.14.0` / `GHOSTTY_DEBUG_FAKE_ADW_VERSION=1.4.0`
//! make `gtk_version::get_runtime_version` / `adw_version::get_runtime_version`
//! report that version. It only changes what the `at_least` family answers;
//! the real libraries are still the ones that are linked. Compiled out of
//! release builds.

use crate::gtk_version::Version;

/// Parses `major.minor[.micro]` from the environment variable `var`.
pub(crate) fn get(var: &str) -> Option<Version> {
    parse(&std::env::var(var).ok()?)
}

fn parse(s: &str) -> Option<Version> {
    let mut it = s.trim().split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let micro = it.next().map_or(Some(0), |v| v.parse().ok())?;
    Some((major, minor, micro))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_versions() {
        assert_eq!(parse("1.4"), Some((1, 4, 0)));
        assert_eq!(parse("4.14.2"), Some((4, 14, 2)));
        assert_eq!(parse("x"), None);
        assert_eq!(parse("1"), None);
    }
}
