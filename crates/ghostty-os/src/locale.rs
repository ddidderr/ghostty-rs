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
//! The macOS-specific `LANG` population from Cocoa is not applicable.

/// The locale Ghostty falls back to (and exports as `LANG`) when the
/// environment's locale can't be set.
pub const FALLBACK_LOCALE: &str = "en_US.UTF-8";

/// Ensure that the locale is set.
///
/// DEFERRED (see module docs): this is currently a no-op that only logs.
/// Returns `Some(value)` when `LANG` should be set to `value` for child
/// processes; always `None` for now.
pub fn ensure_locale() -> Option<String> {
    log::debug!("ensure_locale: setlocale is not available from safe Rust; relying on the toolkit");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_locale_is_noop() {
        assert_eq!(ensure_locale(), None);
        assert_eq!(FALLBACK_LOCALE, "en_US.UTF-8");
    }
}
