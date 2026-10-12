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
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
pub fn init(_resources_dir: &std::path::Path) -> Result<(), InitError> {
    // i18n is unsupported on Windows
    Ok(())
}

/// See the Windows variant above.
#[cfg(not(windows))]
pub fn init(resources_dir: &std::path::Path) -> Result<(), InitError> {
    // Our resources dir is always nested below the share dir that
    // is standard for translations.
    // (`std.fs.path.dirname` returns null for a bare relative name, where
    // `Path::parent` returns an empty path.)
    let share_dir = resources_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(InitError::InvalidResourcesDir)?;

    // Build our locale path
    let path = share_dir.join("locale");

    // Bind our bundle ID to the given locale path (gettextrs takes a
    // `Into<PathBuf>`, so non-UTF-8 bytes are preserved).
    log::debug!("binding domain={BUNDLE_ID} path={}", path.display());
    gettextrs::bindtextdomain(BUNDLE_ID, path).map_err(|_| InitError::OutOfMemory)?;
    Ok(())
}

/// Set the global gettext domain to our bundle ID, allowing unqualified
/// `gettext` calls to look up translations for our application.
///
/// This should only be called for apprts that are fully owning the
/// Ghostty application. This should not be called for libghostty users.
pub fn init_global_domain() -> Result<(), InitError> {
    #[cfg(not(windows))]
    gettextrs::textdomain(BUNDLE_ID).map_err(|_| InitError::OutOfMemory)?;
    Ok(())
}

/// Translate a message for the Ghostty domain (Zig: `_`).
pub fn tr(msgid: &str) -> String {
    #[cfg(windows)]
    return msgid.to_owned();
    #[cfg(not(windows))]
    gettextrs::dgettext(BUNDLE_ID, msgid)
}

/// Canonicalize a locale name from a platform-specific value to
/// a POSIX-compliant value.
///
/// The Zig version is a thin layer over gnulib's
/// `_libintl_locale_name_canonicalize` (the Mac OS X flavour of
/// `gl_locale_name_canonicalize`), after fixing up some `zh` locales. This
/// ports that function exactly, including its legacy-name, language-tag and
/// script tables ([`crate::i18n_canon_table`]). Encodings are NOT parsed out
/// (like the original, `en_US.UTF-8` becomes `en_US.UTF_8`).
pub fn canonicalize_locale(locale: &str) -> String {
    // Fix zh locales
    if let Some(fixed) = fix_zh_locale(locale) {
        return fixed.to_owned();
    }

    libintl_locale_name_canonicalize(locale)
}

/// `gl_locale_name_canonicalize` (gnulib `localename-unsafe.c`).
fn libintl_locale_name_canonicalize(name: &str) -> String {
    use crate::i18n_canon_table::{LANGTAG_TABLE, LEGACY_TABLE, SCRIPT_TABLE};

    // Step 1: Convert using legacy_table.
    if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && let Some((_, unixy)) = LEGACY_TABLE.iter().find(|(l, _)| *l == name)
    {
        return (*unixy).to_owned();
    }

    // Step 2: Convert using langtag_table and script_table.
    if name.len() == 7 && name.as_bytes()[2] == b'-' {
        if let Some((_, unixy)) = LANGTAG_TABLE.iter().find(|(l, _)| *l == name) {
            return (*unixy).to_owned();
        }

        // `name.get(3..)` is None only when byte 3 is not a char boundary,
        // in which case no script entry can match.
        if let Some(script) = name.get(3..)
            && let Some((_, unixy)) = SCRIPT_TABLE.iter().find(|(s, _)| *s == script)
        {
            // name[2] = '@'; strcpy (name + 3, unixy)
            return format!("{}@{unixy}", &name[..2]);
        }
    }

    // Step 3: Convert new-style dash to Unix underscore.
    name.replace('-', "_")
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
    fn canonicalize_locale_libintl_tables() {
        // langtag_table
        assert_eq!("pa_IN", canonicalize_locale("pa-Guru"));
        assert_eq!("pa_PK", canonicalize_locale("pa-Arab"));
        assert_eq!("az", canonicalize_locale("az-Latn"));
        assert_eq!("sr", canonicalize_locale("sr-Cyrl"));
        assert_eq!("kk", canonicalize_locale("kk-Cyrl"));
        assert_eq!("ga", canonicalize_locale("ga-dots"));
        // 8 bytes long: the `strlen == 7` guard skips the tables (upstream quirk).
        assert_eq!("yue_Hans", canonicalize_locale("yue-Hans"));
        // script_table (name[2] = '@')
        assert_eq!("sr@latin", canonicalize_locale("sr-Latn"));
        assert_eq!("uz@arabic", canonicalize_locale("uz-Arab"));
        assert_eq!("mn@mongolian", canonicalize_locale("mn-Mong"));
        // legacy_table (only for names starting uppercase)
        assert_eq!("pt_BR", canonicalize_locale("Brazilian Portuguese"));
        assert_eq!("zh_TW", canonicalize_locale("Traditional Chinese"));
        assert_eq!("de", canonicalize_locale("German"));
        assert_eq!("yi", canonicalize_locale("Yiddish"));
        assert_eq!("english", canonicalize_locale("english"));
        // dash fallback
        assert_eq!("pt_BR", canonicalize_locale("pt-BR"));
        assert_eq!("en_US", canonicalize_locale("en-US"));
        // non-ASCII input of length 7 must not panic
        assert_eq!(
            "\u{e9}\u{e9}\u{e9}x",
            canonicalize_locale("\u{e9}\u{e9}\u{e9}x")
        );
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
    #[cfg(windows)]
    fn init_is_a_noop_on_windows() {
        // i18n is unsupported on Windows (Zig returns before looking at it).
        assert!(init(std::path::Path::new("ghostty")).is_ok());
        assert!(init(std::path::Path::new("/")).is_ok());
    }

    #[test]
    #[cfg(not(windows))]
    fn init_rejects_dirless_resources_dir() {
        assert!(matches!(
            init(std::path::Path::new("ghostty")),
            Err(InitError::InvalidResourcesDir)
        ));
        assert!(matches!(
            init(std::path::Path::new("/")),
            Err(InitError::InvalidResourcesDir)
        ));
    }

    #[test]
    fn tr_falls_back_to_msgid() {
        assert_eq!(tr("some untranslated message"), "some untranslated message");
    }
}
