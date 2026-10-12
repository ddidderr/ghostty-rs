//! Locale setup (`os/locale.zig`).
//!
//! On every Unix target the full Zig behaviour is implemented in [`posix`]
//! (it needs `unsafe` for `setlocale`/`setenv`, allowed there with `SAFETY`
//! comments): `setlocale(LC_ALL, "")`, then with `LANG` unset, then
//! `en_US.UTF-8` (exported as `LANG`). On Darwin (`target_vendor = "apple"`)
//! `LANG`/`LANGUAGE` are additionally pre-populated from Cocoa when unset.
//! Windows runs the same chain against the C runtime: `setlocale` (with
//! Zig's literal `LC_ALL = 6`, which the UCRT rejects, so the chain proceeds
//! exactly as it does in Zig) and `_putenv_s` for the `LANG` updates.
//! wasm has no `setlocale` handling.

/// The locale Ghostty falls back to (and exports as `LANG`) when the
/// environment's locale can't be set.
pub const FALLBACK_LOCALE: &str = "en_US.UTF-8";

/// Ensure that the locale is set (`ensureLocale`).
/// Returns `Some("en_US.UTF-8")` if we had to fall back to it (and exported
/// it as `LANG`), otherwise `None`.
#[cfg(any(unix, windows))]
pub fn ensure_locale() -> Option<String> {
    posix::ensure_locale()
}

/// Ensure that the locale is set: nothing to do on targets without a C
/// runtime locale (wasm).
#[cfg(not(any(unix, windows)))]
pub fn ensure_locale() -> Option<String> {
    None
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

#[cfg(any(unix, windows))]
#[allow(unsafe_code)]
mod posix {
    #[cfg(windows)]
    use std::ffi::c_char;
    use std::ffi::{CStr, CString};

    use super::FALLBACK_LOCALE;
    #[cfg(target_vendor = "apple")]
    use super::{format_lang, preferred_language_value};

    /// Zig's `LC_ALL` constant (`os/locale.zig`, "from locale.h"). It is
    /// the value the Zig code passes on every platform, Windows included.
    #[cfg(windows)]
    const LC_ALL: i32 = 6;

    #[cfg(windows)]
    unsafe extern "C" {
        fn setlocale(category: i32, locale: *const c_char) -> *mut c_char;
        fn _putenv_s(name: *const c_char, value: *const c_char) -> i32;
    }

    #[cfg(windows)]
    fn setlocale_all(locale: &str) -> Option<String> {
        let c = CString::new(locale).ok()?;
        // SAFETY: valid NUL-terminated string; the C runtime returns a
        // pointer to a static string (or null) which we copy immediately.
        unsafe {
            let r = setlocale(LC_ALL, c.as_ptr());
            (!r.is_null()).then(|| CStr::from_ptr(r).to_string_lossy().into_owned())
        }
    }

    /// Zig's Windows `setenv`/`unsetenv`: `_putenv_s` (an empty value
    /// removes the variable).
    #[cfg(windows)]
    fn putenv_s(key: &str, value: &str) {
        let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) else {
            return;
        };
        // SAFETY: both pointers are valid NUL-terminated strings that the
        // C runtime copies; called during early startup like the Zig code.
        unsafe { _putenv_s(k.as_ptr(), v.as_ptr()) };
    }

    #[cfg(unix)]
    fn setlocale_all(locale: &str) -> Option<String> {
        let c = CString::new(locale).ok()?;
        // SAFETY: valid NUL-terminated string; setlocale returns a pointer to
        // a static string (or null) which we copy immediately.
        unsafe {
            let r = libc::setlocale(libc::LC_ALL, c.as_ptr());
            (!r.is_null()).then(|| CStr::from_ptr(r).to_string_lossy().into_owned())
        }
    }

    #[cfg(windows)]
    fn set_env(key: &str, value: &str) {
        putenv_s(key, value);
    }

    #[cfg(windows)]
    fn unset_env(key: &str) {
        putenv_s(key, "");
    }

    #[cfg(unix)]
    fn set_env(key: &str, value: &str) {
        // SAFETY: called during early process startup (locale setup) before
        // other threads read the environment, mirroring the Zig code.
        unsafe { std::env::set_var(key, value) };
    }

    #[cfg(unix)]
    fn unset_env(key: &str) {
        // SAFETY: see `set_env`.
        unsafe { std::env::remove_var(key) };
    }

    /// Sets LANG (and LANGUAGE) from the system preferences.
    #[cfg(target_vendor = "apple")]
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

    /// The process state `ensureLocale` reads and updates (a trait so the
    /// fallback chain is testable without touching the real process).
    pub(super) trait Sys {
        fn lang(&self) -> Option<std::ffi::OsString>;
        fn set_lang(&mut self, value: &str);
        fn unset_lang(&mut self);
        fn setlocale(&mut self, locale: &str) -> Option<String>;
    }

    struct Process;

    impl Sys for Process {
        fn lang(&self) -> Option<std::ffi::OsString> {
            std::env::var_os("LANG")
        }
        fn set_lang(&mut self, value: &str) {
            set_env("LANG", value);
        }
        fn unset_lang(&mut self) {
            unset_env("LANG");
        }
        fn setlocale(&mut self, locale: &str) -> Option<String> {
            setlocale_all(locale)
        }
    }

    pub(super) fn ensure_locale() -> Option<String> {
        // On macOS, pre-populate the LANG env var with system preferences.
        // When launching the .app, LANG is not set so we must query it from
        // the OS. When launching from the CLI, LANG is usually set by the
        // parent process.
        #[cfg(target_vendor = "apple")]
        if std::env::var_os("LANG").is_none_or(|v| v.is_empty()) {
            set_lang_from_cocoa();
        }

        ensure_locale_with(&mut Process)
    }

    pub(super) fn ensure_locale_with(sys: &mut impl Sys) -> Option<String> {
        // Set the locale to whatever is set in env vars.
        if let Some(v) = sys.setlocale("") {
            log::info!("setlocale from env result={v}");
            return None;
        }

        // setlocale failed. This is probably because the LANG env var is
        // invalid. Try to set it without the LANG var set to use the system
        // default.
        if sys.lang().is_some_and(|v| !v.is_empty()) {
            // We don't need to do both of these things but we do them both
            // to be sure that lang is either empty or unset completely.
            sys.set_lang("");
            sys.unset_lang();

            if let Some(v) = sys.setlocale("") {
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
        if let Some(v) = sys.setlocale(FALLBACK_LOCALE) {
            sys.set_lang(FALLBACK_LOCALE);
            log::info!("setlocale default result={v}");
            return Some(FALLBACK_LOCALE.to_owned());
        }
        log::warn!("setlocale failed even with the fallback, uncertain results");
        None
    }

    #[cfg(test)]
    mod tests {
        use std::ffi::OsString;

        use super::*;

        struct Mock {
            lang: Option<OsString>,
            /// Locales `setlocale` accepts, with the "current LANG" rule:
            /// `""` resolves through `lang`.
            supported: Vec<&'static str>,
            c_fallback_when_no_lang: bool,
            calls: Vec<String>,
        }

        impl Sys for Mock {
            fn lang(&self) -> Option<OsString> {
                self.lang.clone()
            }
            fn set_lang(&mut self, value: &str) {
                self.calls.push(format!("set_lang({value})"));
                self.lang = Some(value.into());
            }
            fn unset_lang(&mut self) {
                self.calls.push("unset_lang".into());
                self.lang = None;
            }
            fn setlocale(&mut self, locale: &str) -> Option<String> {
                self.calls.push(format!("setlocale({locale})"));
                if locale.is_empty() {
                    return match &self.lang {
                        Some(l) if !l.is_empty() => {
                            let l = l.to_string_lossy().into_owned();
                            self.supported.contains(&l.as_str()).then_some(l)
                        }
                        _ => self.c_fallback_when_no_lang.then(|| "C".to_owned()),
                    };
                }
                self.supported.contains(&locale).then(|| locale.to_owned())
            }
        }

        fn mock(lang: Option<&str>, supported: &[&'static str], c: bool) -> Mock {
            Mock {
                lang: lang.map(OsString::from),
                supported: supported.to_vec(),
                c_fallback_when_no_lang: c,
                calls: Vec::new(),
            }
        }

        #[test]
        fn valid_env_locale_is_used() {
            let mut m = mock(Some("de_DE.UTF-8"), &["de_DE.UTF-8"], false);
            assert_eq!(ensure_locale_with(&mut m), None);
            assert_eq!(m.calls, ["setlocale()"]);
        }

        #[test]
        fn invalid_lang_retries_without_lang() {
            // Invalid LANG, system default works (not "C").
            let mut m = mock(Some("bogus"), &[], false);
            m.supported = vec![];
            // After unsetting, "" resolves to a real default.
            struct Sys2(Mock);
            impl Sys for Sys2 {
                fn lang(&self) -> Option<OsString> {
                    self.0.lang()
                }
                fn set_lang(&mut self, v: &str) {
                    self.0.set_lang(v);
                }
                fn unset_lang(&mut self) {
                    self.0.unset_lang();
                }
                fn setlocale(&mut self, l: &str) -> Option<String> {
                    self.0.calls.push(format!("setlocale({l})"));
                    (l.is_empty() && self.0.lang.is_none()).then(|| "fr_FR.UTF-8".to_owned())
                }
            }
            let mut s = Sys2(m);
            assert_eq!(ensure_locale_with(&mut s), None);
            assert_eq!(
                s.0.calls,
                ["setlocale()", "set_lang()", "unset_lang", "setlocale()"]
            );
        }

        #[test]
        fn c_fallback_moves_on_to_en_us_and_exports_lang() {
            let mut m = mock(Some("bogus"), &["en_US.UTF-8"], true);
            assert_eq!(ensure_locale_with(&mut m).as_deref(), Some("en_US.UTF-8"));
            assert_eq!(m.lang.as_deref(), Some(std::ffi::OsStr::new("en_US.UTF-8")));
            assert_eq!(m.calls.last().unwrap(), "set_lang(en_US.UTF-8)");
        }

        #[test]
        fn nothing_works_leaves_lang_unset() {
            let mut m = mock(Some("bogus"), &[], false);
            assert_eq!(ensure_locale_with(&mut m), None);
            assert_eq!(m.lang, None);
        }

        #[test]
        fn unset_lang_goes_straight_to_fallback() {
            let mut m = mock(None, &["en_US.UTF-8"], false);
            assert_eq!(ensure_locale_with(&mut m).as_deref(), Some("en_US.UTF-8"));
            // No LANG to unset: setlocale("") then the fallback.
            assert_eq!(m.calls[0], "setlocale()");
            assert_eq!(m.calls[1], "setlocale(en_US.UTF-8)");
        }
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

    #[test]
    fn fallback_locale_value() {
        assert_eq!(FALLBACK_LOCALE, "en_US.UTF-8");
    }
}
