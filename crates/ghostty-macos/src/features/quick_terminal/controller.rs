//! Minimal `QuickTerminalController` surface used by `AppDelegate`
//! (`quickController`, `toggleQuickTerminal`, state restoration).
//! TODO(MA4): port QuickTerminalController.swift, QuickTerminalWindow,
//! QuickTerminalScreenStateCache, the restorable state and the animations.

#![allow(unsafe_code)]

use std::{cell::Cell, rc::Rc};

use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::NSWindowController;

use super::QuickTerminalPosition;
use crate::ghostty::{app::App, surface_configuration::SurfaceConfiguration};

/// `QuickTerminalRestorableState` (TODO(MA4): surface tree, screen state).
#[derive(Clone, Debug, Default)]
pub struct QuickTerminalRestorableState;

impl QuickTerminalRestorableState {
    /// `baseConfig`.
    #[must_use]
    pub fn base_config(&self) -> Option<SurfaceConfiguration> {
        let mut config = SurfaceConfiguration::default();
        config
            .environment_variables
            .insert("GHOSTTY_QUICK_TERMINAL".to_string(), "1".to_string());
        Some(config)
    }
}

/// Instance variables of [`QuickTerminalController`].
#[derive(Default)]
pub struct Ivars {
    visible: Cell<bool>,
    position: Cell<QuickTerminalPosition>,
}

define_class!(
    // SAFETY: NSWindowController has no subclassing requirements we violate and
    // the class does not implement Drop.
    #[unsafe(super(NSWindowController))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickTerminalController"]
    #[ivars = Ivars]
    pub struct QuickTerminalController;

    impl QuickTerminalController {}
);

impl QuickTerminalController {
    /// `init(_ ghostty:position:baseConfig:restorationState:)`.
    pub fn new(
        _ghostty: &Rc<App>,
        position: QuickTerminalPosition,
        _base_config: Option<SurfaceConfiguration>,
        _restoration_state: Option<&QuickTerminalRestorableState>,
    ) -> Retained<Self> {
        let mtm = crate::helpers::main_thread();
        let ivars = Ivars::default();
        ivars.position.set(position);
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is NSWindowController's designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// `visible`.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.ivars().visible.get()
    }

    /// `restorable`.
    #[must_use]
    pub fn restorable(&self) -> bool {
        false
    }

    /// `toggle()`. TODO(MA4)
    pub fn toggle(&self) {
        log::warn!("TODO(MA4): QuickTerminalController.toggle");
    }
}
