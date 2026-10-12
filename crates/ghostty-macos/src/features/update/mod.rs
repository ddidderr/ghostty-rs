//! `Sources/Features/Update` (owner: MA5; Sparkle via the ObjC runtime).
//!
//! MA1 only provides the `UpdateController` surface `AppDelegate` calls.
//! TODO(MA5): everything (UpdateController, driver, view model, pill/popover UI).

use std::cell::Cell;

/// `UpdateController` (Sparkle's `SPUUpdater` wrapper). `updater` settings are
/// stored so `ghosttyConfigDidChange` can apply them once MA5 wires Sparkle.
#[derive(Default)]
pub struct UpdateController {
    pub automatically_checks_for_updates: Cell<bool>,
    pub automatically_downloads_updates: Cell<bool>,
    installing: Cell<bool>,
}

impl UpdateController {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `startUpdater()`. TODO(MA5)
    pub fn start_updater(&self) {
        log::debug!("TODO(MA5): UpdateController.startUpdater");
    }

    /// `checkForUpdates()`. TODO(MA5)
    pub fn check_for_updates(&self) {
        log::warn!("TODO(MA5): UpdateController.checkForUpdates");
    }

    /// `isInstalling`.
    #[must_use]
    pub fn is_installing(&self) -> bool {
        self.installing.get()
    }
}
