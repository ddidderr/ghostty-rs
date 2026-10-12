//! Locale setup (`os/locale.zig`).
//!
//! DEFERRED: the Zig `ensureLocale` is built from `setlocale(LC_ALL, "")`
//! plus `setenv`/`unsetenv` of `LANG`. All of these are `unsafe` in Rust
//! (`libc::setlocale`, `gettextrs::setlocale` and Rust 2024's
//! `std::env::set_var`) and this crate forbids `unsafe`. There is no safe
//! crate that wraps `setlocale`.
//!
//! Resolution options for integration:
//!   * GTK (`gtk_init`, called by `gtk4::init()`) already calls
//!     `setlocale(LC_ALL, "")` on Linux, so for the GTK apprt nothing else is
//!     needed for gettext/Pango to pick up the locale.
//!   * If an explicit call is needed (e.g. CLI actions without GTK), an
//!     `unsafe`-permitted crate (apprt-gtk / the root binary) must perform the
//!     libc call; [`FALLBACK_LOCALE`] documents the Zig fallback behaviour
//!     (try `setlocale(LC_ALL, "")`, then with `LANG` unset, then
//!     `en_US.UTF-8` and export `LANG=en_US.UTF-8`).
//!
//! On Darwin (`target_vendor = "apple"`) the full Zig behaviour is
//! implemented in [`darwin`] (it needs `unsafe` for `setlocale`/`setenv`):
//! `LANG`/`LANGUAGE` are pre-populated from Cocoa when unset and
//! `setlocale(LC_ALL, "")` is called with the Zig fallback chain.

/// The locale Ghostty falls back to (and exports as `LANG`) when the
/// environment's locale can't be set.
pub const FALLBACK_LOCALE: &str = "en_US.UTF-8";

/// Ensure that the locale is set.
///
/// DEFERRED (see module docs): this is currently a no-op that only logs.
/// Returns `Some(value)` when `LANG` should be set to `value` for child
/// processes; always `None` for now.
#[cfg(not(target_vendor = "apple"))]
pub fn ensure_locale() -> Option<String> {
    log::debug!("ensure_locale: setlocale is not available from safe Rust; relying on the toolkit");
    None
}

/// Ensure that the locale is set (Darwin: full port of `ensureLocale`).
/// Returns `Some("en_US.UTF-8")` if we had to fall back to it (and exported
/// it as `LANG`), otherwise `None`.
#[cfg(target_vendor = "apple")]
pub fn ensure_locale() -> Option<String> {
    darwin::ensure_locale()
}

/// Format the system locale as `<lang>_<country>.UTF-8` (the value Zig's
/// `setLangFromCocoa` exports as `LANG`).
pub fn format_lang(lang: &str, country: &str) -> String {
    format!("{lang}_{country}.UTF-8")
}

/// Build the `LANGUAGE` value from Apple's preferred languages
/// (`preferredLanguageFromCocoa`): each BCP-47 tag is canonicalized to the
/// POSIX format, gets `.UTF-8` appended and they are joined with `:`.
/// Returns `None` if there are no preferred languages.
///
/// gettext uses `LANGUAGE` to override only translations, with a priority
/// order given by colon-separated entries (e.g. `en:fr`).
pub fn preferred_language_value<S: AsRef<str>>(preferred: &[S]) -> Option<String> {
    let mut out = String::new();
    for lang in preferred {
        if !out.is_empty() {
            out.push(':');
        }
        out.push_str(&crate::i18n::canonicalize_locale(lang.as_ref()));
        // The canonicalized locale never contains the encoding and all of
        // our translations require UTF-8 so we add that.
        out.push_str(".UTF-8");
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
mod darwin {
    use std::ffi::{CStr, CString};

    use super::{FALLBACK_LOCALE, format_lang, preferred_language_value};

    fn setlocale_all(locale: &str) -> Option<String> {
        let c = CString::new(locale).ok()?;
        // SAFETY: valid NUL-terminated string; setlocale returns a pointer to
        // a static string (or null) which we copy immediately.
        unsafe {
            let r = libc::setlocale(libc::LC_ALL, c.as_ptr());
            (!r.is_null()).then(|| CStr::from_ptr(r).to_string_lossy().into_owned())
        }
    }

    fn set_env(key: &str, value: &str) {
        // SAFETY: called during early process startup (locale setup) before
        // other threads read the environment, mirroring the Zig code.
        unsafe { std::env::set_var(key, value) };
    }

    fn unset_env(key: &str) {
        // SAFETY: see `set_env`.
        unsafe { std::env::remove_var(key) };
    }

    /// Sets LANG (and LANGUAGE) from the system preferences.
    fn set_lang_from_cocoa() {
        use objc2_foundation::NSLocale;

        let locale = NSLocale::currentLocale();
        let lang = locale.languageCode().to_string();
        // `countryCode` is deprecated by Apple but is what the Zig code uses.
        #[allow(deprecated)]
        let country = locale.countryCode();
        let Some(country) = country.map(|c| c.to_string()) else {
            log::warn!("languageCode or countryCode not found. Locale may be incorrect.");
            return;
        };

        let env_value = format_lang(&lang, &country);
        log::info!("detected system locale={env_value}");
        set_env("LANG", &env_value);

        // Get our preferred languages and set that to the LANGUAGE env var
        // in case our language differs from our locale.
        let preferred: Vec<String> = NSLocale::preferredLanguages()
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Some(pref) = preferred_language_value(&preferred) {
            log::debug!("setting LANGUAGE from preferred languages value={pref}");
            set_env("LANGUAGE", &pref);
        }
    }

    pub(super) fn ensure_locale() -> Option<String> {
        // On macOS, pre-populate the LANG env var with system preferences.
        // When launching the .app, LANG is not set so we must query it from
        // the OS. When launching from the CLI, LANG is usually set by the
        // parent process.
        let lang = std::env::var_os("LANG");
        if lang.as_ref().is_none_or(|v| v.is_empty()) {
            set_lang_from_cocoa();
        }

        // Set the locale to whatever is set in env vars.
        if let Some(v) = setlocale_all("") {
            log::info!("setlocale from env result={v}");
            return None;
        }

        // setlocale failed. This is probably because the LANG env var is
        // invalid. Try to set it without the LANG var set to use the system
        // default.
        if std::env::var_os("LANG").is_some_and(|v| !v.is_empty()) {
            // We don't need to do both of these things but we do them both
            // to be sure that lang is either empty or unset completely.
            set_env("LANG", "");
            unset_env("LANG");

            if let Some(v) = setlocale_all("") {
                log::info!("setlocale after unset lang result={v}");

                // If we try to setlocale to an unsupported locale it'll
                // return "C" as the POSIX/C fallback, if that's the case we
                // want to not use it and move to our fallback.
                if v != "C" {
                    return None;
                }
            }
        }

        // Failure again... fallback to en_US.UTF-8
        log::warn!(
            "setlocale failed with LANG and system default. Falling back to {FALLBACK_LOCALE}"
        );
        if let Some(v) = setlocale_all(FALLBACK_LOCALE) {
            set_env("LANG", FALLBACK_LOCALE);
            log::info!("setlocale default result={v}");
            return Some(FALLBACK_LOCALE.to_owned());
        }
        log::warn!("setlocale failed even with the fallback, uncertain results");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_lang_value() {
        assert_eq!(format_lang("en", "US"), "en_US.UTF-8");
    }

    #[test]
    fn preferred_languages_to_language_var() {
        assert_eq!(preferred_language_value::<&str>(&[]), None);
        assert_eq!(
            preferred_language_value(&["en-US"]).as_deref(),
            Some("en_US.UTF-8")
        );
        assert_eq!(
            preferred_language_value(&["zh-Hans-CN", "en", "fr-CA"]).as_deref(),
            Some("zh_CN.UTF-8:en.UTF-8:fr_CA.UTF-8")
        );
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn ensure_locale_is_noop() {
        assert_eq!(ensure_locale(), None);
        assert_eq!(FALLBACK_LOCALE, "en_US.UTF-8");
    }
}
