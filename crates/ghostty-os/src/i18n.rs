use std::path::Path;

use crate::i18n_locales::LOCALES;

/// Application bundle ID, used as the gettext domain
/// (`build_config.bundle_id`).
// TODO(integration): take from a shared build-config crate.
pub const BUNDLE_ID: &str = "com.mitchellh.ghostty";

/// Membership test for supported locales (Zig: `locales_map`).
pub fn is_supported_locale(locale: &str) -> bool {
    LOCALES.contains(&locale)
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("invalid resources dir")]
    InvalidResourcesDir,
    #[error("out of memory")]
    OutOfMemory,
}

/// Initialize i18n support for the application. This should be
/// called automatically by the global state initialization.
///
/// This calls `bindtextdomain` for gettext with the proper directory
/// of translations. This does NOT call `textdomain` as we don't
/// want to set the domain for the entire application since this is also
/// used by libghostty.
pub fn init(resources_dir: &str) -> Result<(), InitError> {
    // Our resources dir is always nested below the share dir that
    // is standard for translations.
    // (`std.fs.path.dirname` returns null for a bare relative name, where
    // `Path::parent` returns an empty path.)
    let share_dir = Path::new(resources_dir)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(InitError::InvalidResourcesDir)?;

    // Build our locale path
    let path = format!("{}/locale", share_dir.display());

    // Bind our bundle ID to the given locale path
    log::debug!("binding domain={BUNDLE_ID} path={path}");
    gettextrs::bindtextdomain(BUNDLE_ID, path).map_err(|_| InitError::OutOfMemory)?;
    Ok(())
}

/// Set the global gettext domain to our bundle ID, allowing unqualified
/// `gettext` calls to look up translations for our application.
///
/// This should only be called for apprts that are fully owning the
/// Ghostty application. This should not be called for libghostty users.
pub fn init_global_domain() -> Result<(), InitError> {
    gettextrs::textdomain(BUNDLE_ID).map_err(|_| InitError::OutOfMemory)?;
    Ok(())
}

/// Translate a message for the Ghostty domain (Zig: `_`).
pub fn tr(msgid: &str) -> String {
    gettextrs::dgettext(BUNDLE_ID, msgid)
}

/// Canonicalize a locale name from a platform-specific value to
/// a POSIX-compliant value.
///
/// The Zig version is a thin layer over gnulib's
/// `_libintl_locale_name_canonicalize` (only linked when building libintl
/// from source, i.e. macOS). That function isn't available here, so this is
/// a pure-Rust approximation of the cases that matter: BCP-47 style tags
/// have `-` replaced with `_` and the Chinese script tags are mapped to
/// their POSIX region equivalents. Encodings are NOT parsed out (like the
/// original, `en_US.UTF-8` becomes `en_US.UTF_8`).
pub fn canonicalize_locale(locale: &str) -> String {
    // Fix zh locales
    if let Some(fixed) = fix_zh_locale(locale) {
        return fixed.to_owned();
    }

    // Emulate gnulib's mapping of bare zh script tags.
    match locale {
        "zh-Hans" => return "zh_CN".to_owned(),
        "zh-Hant" => return "zh_TW".to_owned(),
        _ => {}
    }

    locale.replace('-', "_")
}

/// Handles some zh locales canonicalization because internal libintl
/// canonicalization function doesn't handle correctly in these cases.
fn fix_zh_locale(locale: &str) -> Option<&'static str> {
    let mut it = locale.split('-');
    let name = it.next()?;
    if name != "zh" {
        return None;
    }

    let script = it.next()?;
    let region = it.next()?;

    if script == "Hans" {
        if region == "SG" {
            return Some("zh_SG");
        }
        return Some("zh_CN");
    }

    if script == "Hant" {
        if region == "MO" {
            return Some("zh_MO");
        }
        if region == "HK" {
            return Some("zh_HK");
        }
        return Some("zh_TW");
    }

    None
}

/// This can be called at any point a compile-time-known locale is
/// available. In a `const` context this verifies the locale is supported
/// (panics at compile time otherwise).
pub const fn static_locale(v: &'static str) -> &'static str {
    let vb = v.as_bytes();
    let mut i = 0;
    while i < LOCALES.len() {
        let lb = LOCALES[i].as_bytes();
        if lb.len() == vb.len() {
            let mut j = 0;
            let mut eq = true;
            while j < lb.len() {
                if lb[j] != vb[j] {
                    eq = false;
                    break;
                }
                j += 1;
            }
            if eq {
                return LOCALES[i];
            }
        }
        i += 1;
    }
    panic!("unsupported locale");
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Zig test is darwin-only; the canonicalization here is portable.
    #[test]
    fn canonicalize_locale_darwin() {
        assert_eq!("en_US", canonicalize_locale("en_US"));
        assert_eq!("zh_CN", canonicalize_locale("zh-Hans"));
        assert_eq!("zh_TW", canonicalize_locale("zh-Hant"));

        assert_eq!("zh_CN", canonicalize_locale("zh-Hans-CN"));
        assert_eq!("zh_SG", canonicalize_locale("zh-Hans-SG"));
        assert_eq!("zh_TW", canonicalize_locale("zh-Hant-TW"));
        assert_eq!("zh_HK", canonicalize_locale("zh-Hant-HK"));
        assert_eq!("zh_MO", canonicalize_locale("zh-Hant-MO"));

        // This is just an edge case I want to make sure we're aware of:
        // canonicalizeLocale does not handle encodings and will turn them into
        // underscores. We should parse them out before calling this function.
        assert_eq!("en_US.UTF_8", canonicalize_locale("en_US.UTF-8"));
    }

    #[test]
    fn locales_map_and_static_locale() {
        assert!(is_supported_locale("zh_CN"));
        assert!(is_supported_locale("kk"));
        assert!(!is_supported_locale("xx"));
        const L: &str = static_locale("de");
        assert_eq!(L, "de");
    }

    #[test]
    fn init_rejects_dirless_resources_dir() {
        assert!(matches!(
            init("ghostty"),
            Err(InitError::InvalidResourcesDir)
        ));
        assert!(matches!(init("/"), Err(InitError::InvalidResourcesDir)));
    }

    #[test]
    fn tr_falls_back_to_msgid() {
        assert_eq!(tr("some untranslated message"), "some untranslated message");
    }
}
