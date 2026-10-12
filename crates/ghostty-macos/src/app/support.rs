//! Small AppKit/Foundation operations `AppDelegate` needs that Swift gets from
//! framework conveniences (typed `register(defaults:)`, `Bundle.infoDictionary`,
//! `NSWorkspace.setDefaultApplication(at:toOpen:)` with a `UTType`).

#![allow(unsafe_code)]

use block2::RcBlock;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSNumber, NSString, NSURL};

use crate::helpers::extensions::user_defaults;

/// `UserDefaults.ghostty.register(defaults: [key: Bool])`.
pub fn register_defaults(defaults: &[(&str, bool)]) {
    let keys: Vec<Retained<NSString>> = defaults
        .iter()
        .map(|(k, _)| NSString::from_str(k))
        .collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<Retained<AnyObject>> = defaults
        .iter()
        .map(|(_, v)| {
            let n = NSNumber::new_bool(*v);
            // SAFETY: an NSNumber is an Objective-C object.
            unsafe { Retained::cast_unchecked::<AnyObject>(n) }
        })
        .collect();
    let dict: Retained<NSDictionary<NSString, AnyObject>> =
        NSDictionary::from_retained_objects(&key_refs, &values);
    // SAFETY: `registerDefaults:` takes an NSDictionary of property-list values.
    unsafe { user_defaults::ghostty().registerDefaults(&dict) };
}

/// `UserDefaults.ghostty.removePersistentDomain(forName: suite)` (debug builds).
pub fn remove_persistent_domain_for_ghostty_suite() {
    if let Some(suite) = user_defaults::ghostty_suite() {
        user_defaults::ghostty().removePersistentDomainForName(&NSString::from_str(&suite));
    }
}

/// `Bundle.main.infoDictionary?[key] as? Bool`.
#[must_use]
pub fn bundle_info_bool(key: &str) -> Option<bool> {
    let info = NSBundle::mainBundle().infoDictionary()?;
    let value = info.objectForKey(&NSString::from_str(key))?;
    let number = value.downcast_ref::<NSNumber>()?;
    Some(number.boolValue())
}

/// `NSWorkspace.shared.setDefaultApplication(at: Bundle.main.bundleURL,
/// toOpen: .unixExecutable) { error in ... }`.
pub fn set_default_terminal(handler: &RcBlock<dyn Fn(*mut NSError)>) {
    let Some(ut_type) = AnyClass::get(c"UTType") else {
        log::warn!("UTType is unavailable");
        return;
    };
    let identifier = NSString::from_str("public.unix-executable");
    // SAFETY: `+[UTType typeWithIdentifier:]` takes an NSString and returns UTType?.
    let content_type: Option<Retained<AnyObject>> =
        unsafe { msg_send![ut_type, typeWithIdentifier: &*identifier] };
    let Some(content_type) = content_type else {
        return;
    };
    let url: Retained<NSURL> = NSBundle::mainBundle().bundleURL();
    let workspace = NSWorkspace::sharedWorkspace();
    // SAFETY: selector and argument types are those of
    // `-setDefaultApplicationAtURL:toOpenContentType:completionHandler:`.
    let _: () = unsafe {
        msg_send![
            &*workspace,
            setDefaultApplicationAtURL: &*url,
            toOpenContentType: &*content_type,
            completionHandler: &**handler
        ]
    };
}
