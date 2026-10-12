//! `Helpers/Extensions/UserDefaults+Extension.swift`.

use objc2::{AnyThread, rc::Retained};
use objc2_foundation::{NSString, NSUserDefaults};

/// `UserDefaults.ghosttySuite`: only set in debug builds from
/// `GHOSTTY_USER_DEFAULTS_SUITE`.
#[must_use]
pub fn ghostty_suite() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("GHOSTTY_USER_DEFAULTS_SUITE").ok()
    } else {
        None
    }
}

/// `UserDefaults.ghostty`: the suite if configured, else the standard defaults.
#[must_use]
pub fn ghostty() -> Retained<NSUserDefaults> {
    ghostty_suite()
        .and_then(|suite| {
            NSUserDefaults::initWithSuiteName(
                NSUserDefaults::alloc(),
                Some(&NSString::from_str(&suite)),
            )
        })
        .unwrap_or_else(NSUserDefaults::standardUserDefaults)
}

/// `UserDefaults.ghostty.register(defaults: [...])` with boolean values.
pub fn register_defaults(defaults: &[(&str, bool)]) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber};
    let keys: Vec<Retained<NSString>> = defaults
        .iter()
        .map(|(k, _)| NSString::from_str(k))
        .collect();
    let values: Vec<Retained<NSNumber>> = defaults
        .iter()
        .map(|(_, v)| NSNumber::new_bool(*v))
        .collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let value_refs: Vec<&AnyObject> = values.iter().map(|v| &***v as &AnyObject).collect();
    let dict = NSDictionary::from_slices(&key_refs, &value_refs);
    // SAFETY: the dictionary holds NSNumbers keyed by NSStrings (a plist).
    #[allow(unsafe_code)]
    unsafe {
        ghostty().registerDefaults(&dict);
    }
}

/// `UserDefaults.ghostty.removePersistentDomain(forName: suite)` when a
/// suite is configured (debug builds; see [`ghostty_suite`]).
pub fn remove_persistent_domain_for_ghostty_suite() {
    if let Some(suite) = ghostty_suite() {
        ghostty().removePersistentDomainForName(&NSString::from_str(&suite));
    }
}
