//! `Sources/Features/Global Keybinds` (owner: MA4).
//! MA1 provides the `GlobalEventTap.shared` surface `AppDelegate` uses.
//! TODO(MA4): CGEventTap implementation.

use std::rc::Rc;

/// `GlobalEventTap`.
#[derive(Default)]
pub struct GlobalEventTap;

thread_local! {
    static SHARED: Rc<GlobalEventTap> = Rc::default();
}

impl GlobalEventTap {
    #[must_use]
    pub fn shared() -> Rc<Self> {
        SHARED.with(Rc::clone)
    }

    /// `enable()`. TODO(MA4)
    pub fn enable(&self) {
        log::warn!("TODO(MA4): GlobalEventTap.enable");
    }

    /// `disable()`. TODO(MA4)
    pub fn disable(&self) {}
}
