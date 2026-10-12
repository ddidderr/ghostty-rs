//! `Sources/Features/Services` (owner: MA4): `ServiceProvider`
//! (`NSApp.servicesProvider`, the "New Ghostty Tab/Window Here" services).

#![allow(unsafe_code)]

use objc2::{MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_foundation::NSObject;

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ServiceProvider"]
    pub struct ServiceProvider;

    impl ServiceProvider {
        // TODO(MA4): openTab:userData:error: and openWindow:userData:error:
        // (ServiceProvider.swift) once the terminal controllers exist.
    }
);

impl ServiceProvider {
    #[must_use]
    pub fn new() -> Retained<Self> {
        let this = Self::alloc(crate::helpers::main_thread());
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![this, init] }
    }
}
