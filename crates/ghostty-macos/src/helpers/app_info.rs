//! `Helpers/AppInfo.swift`.

/// True if we appear to be running in Xcode.
#[must_use]
pub fn is_running_in_xcode() -> bool {
    is_running_in_xcode_with(|k| std::env::var_os(k).is_some())
}

fn is_running_in_xcode_with(has_var: impl Fn(&str) -> bool) -> bool {
    has_var("__XCODE_BUILT_PRODUCTS_DIR_PATHS")
}

/// `Bundle.main.infoDictionary?[key] as? Bool`.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn bundle_info_bool(key: &str) -> Option<bool> {
    use objc2_foundation::{NSBundle, NSNumber, NSString};
    let info = NSBundle::mainBundle().infoDictionary()?;
    let value = info.objectForKey(&NSString::from_str(key))?;
    value.downcast_ref::<NSNumber>().map(NSNumber::boolValue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_xcode_env() {
        assert!(is_running_in_xcode_with(
            |k| k == "__XCODE_BUILT_PRODUCTS_DIR_PATHS"
        ));
        assert!(!is_running_in_xcode_with(|_| false));
    }
}
