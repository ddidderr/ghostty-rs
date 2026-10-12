//! Compile-time platform predicates used by the input code
//! (`builtin.target.os.tag.isDarwin()` / `builtin.os.tag == .macos` in Zig).
//!
//! In test builds the predicates can be overridden per thread with
//! [`with_darwin`] so the macOS-only branches (option-as-alt, super-instead-
//! of-ctrl, keycode table column) are exercised on every host.

/// `isDarwin()`: macOS and iOS.
const DARWIN: bool = cfg!(target_vendor = "apple");

/// `os.tag == .macos`.
const MACOS: bool = cfg!(target_os = "macos");

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<(bool, bool)>> =
        const { std::cell::Cell::new(None) };
}

/// True when targeting Darwin (macOS or iOS).
#[inline]
pub fn is_darwin() -> bool {
    #[cfg(test)]
    if let Some((darwin, _)) = OVERRIDE.with(std::cell::Cell::get) {
        return darwin;
    }
    DARWIN
}

/// True when targeting macOS proper.
#[inline]
pub fn is_macos() -> bool {
    #[cfg(test)]
    if let Some((_, macos)) = OVERRIDE.with(std::cell::Cell::get) {
        return macos;
    }
    MACOS
}

/// Run `f` pretending to be macOS (`darwin == macos == true`) or not.
#[cfg(test)]
pub fn with_darwin<R>(on: bool, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<(bool, bool)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _reset = Reset(OVERRIDE.with(|o| o.replace(Some((on, on)))));
    f()
}
