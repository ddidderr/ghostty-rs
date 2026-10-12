//! `Sources/Features/Settings` (owner: MA4).
//!
//! MA1 provides the `ConfigurationErrorsController` surface that
//! `AppDelegate.ghosttyConfigDidChange` drives. TODO(MA4): the window
//! (ConfigurationErrors.xib + SwiftUI view), SettingsView.

use std::cell::RefCell;

/// `ConfigurationErrorsController.sharedInstance` (data model only).
#[derive(Default)]
pub struct ConfigurationErrorsController {
    errors: RefCell<Vec<String>>,
    window_visible: std::cell::Cell<bool>,
}

thread_local! {
    static SHARED: std::rc::Rc<ConfigurationErrorsController> = std::rc::Rc::default();
}

impl ConfigurationErrorsController {
    /// `sharedInstance`.
    #[must_use]
    pub fn shared_instance() -> std::rc::Rc<Self> {
        SHARED.with(std::rc::Rc::clone)
    }

    /// `errors` getter.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        self.errors.borrow().clone()
    }

    /// `errors` setter: with no errors the window closes (`didSet`).
    pub fn set_errors(&self, errors: Vec<String>) {
        let empty = errors.is_empty();
        *self.errors.borrow_mut() = errors;
        if empty {
            self.window_visible.set(false);
        }
    }

    /// `window == nil || !window!.isVisible`.
    #[must_use]
    pub fn is_window_visible(&self) -> bool {
        self.window_visible.get()
    }

    /// `showWindow(_:)`. TODO(MA4)
    pub fn show_window(&self) {
        log::warn!(
            "TODO(MA4): ConfigurationErrorsController.showWindow ({} errors)",
            self.errors.borrow().len()
        );
        self.window_visible.set(true);
    }
}
