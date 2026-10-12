//! `Sources/Features/Secure Input` (owner: MA4).
//! MA1 provides the `SecureInput.shared` surface `AppDelegate` uses
//! (`global`, `enabled`). TODO(MA4): Carbon `EnableSecureEventInput` handling,
//! scoped objects, activation observers and the overlay.

use std::{cell::Cell, rc::Rc};

/// `SecureInput`.
#[derive(Default)]
pub struct SecureInput {
    global: Cell<bool>,
    enabled: Cell<bool>,
}

thread_local! {
    static SHARED: Rc<SecureInput> = Rc::default();
}

impl SecureInput {
    /// `shared`.
    #[must_use]
    pub fn shared() -> Rc<Self> {
        SHARED.with(Rc::clone)
    }

    #[must_use]
    pub fn global(&self) -> bool {
        self.global.get()
    }

    /// `global` setter (`didSet { apply() }`). TODO(MA4): apply.
    pub fn set_global(&self, v: bool) {
        self.global.set(v);
    }

    /// `enabled`.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }
}
