//! Block based observation of AppKit/Foundation notifications
//! (`NotificationCenter.default.addObserver(forName:...)`), the AppKit
//! counterpart of `helpers::notification_center` (which carries Ghostty's own
//! notifications). Dropping the [`NsObserver`] removes it.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::{
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_foundation::{
    NSNotification,
    NSNotificationCenter,
    NSNotificationName,
    NSObjectProtocol,
};

#[must_use = "dropping the observer removes it"]
pub struct NsObserver {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl NsObserver {
    /// Keep observing for the life of the process.
    pub fn forget(self) {
        std::mem::forget(self);
    }
}

impl Drop for NsObserver {
    fn drop(&mut self) {
        // SAFETY: `token` was returned by addObserverForName on this center.
        #[allow(unsafe_code)]
        unsafe {
            self.center.removeObserver(self.token.as_ref());
        }
    }
}

/// `NotificationCenter.default.addObserver(forName:object:queue:nil)`:
/// `f` runs synchronously on the posting thread.
pub fn observe(
    name: &NSNotificationName,
    object: Option<&AnyObject>,
    f: impl Fn(&NSNotification) + 'static,
) -> NsObserver {
    let center = NSNotificationCenter::defaultCenter();
    let block = RcBlock::new(move |n: NonNull<NSNotification>| {
        // SAFETY: the notification is valid for the duration of the callback.
        #[allow(unsafe_code)]
        f(unsafe { n.as_ref() });
    });
    // SAFETY: the block is a valid `void (^)(NSNotification *)`.
    #[allow(unsafe_code)]
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), object, None, &block)
    };
    NsObserver { center, token }
}

/// `NSWindow.didBecomeKeyNotification`.
#[must_use]
pub fn name_window_did_become_key() -> &'static NSNotificationName {
    // SAFETY: reading AppKit's immutable notification name constant.
    unsafe { objc2_app_kit::NSWindowDidBecomeKeyNotification }
}
