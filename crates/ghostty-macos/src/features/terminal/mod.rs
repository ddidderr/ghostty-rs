//! `Sources/Features/Terminal`: terminal windows and their controllers.
//!
//! TODO(MA3): this file is the *foundation stub* written by MA1 so that
//! `Ghostty.App`'s action glue and the app delegate compile. It declares the
//! classes with the API surface they use, logs `TODO(MA3)` and returns
//! conservative values. MA3 replaces every body with the real port of
//! `BaseTerminalController.swift`, `TerminalController.swift`,
//! `Window Styles/TerminalWindow.swift` etc., keeping these signatures.
#![cfg(target_vendor = "apple")]
// Stubs: bodies do not use `self` yet.
#![allow(clippy::unused_self)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use log::warn;
use objc2::{DefinedClass, MainThreadOnly, define_class, rc::Retained};
use objc2_app_kit::{NSResponder, NSWindow, NSWindowController};
use objc2_foundation::NSObject;

use crate::ghostty::{
    app::App,
    package::SplitFocusDirection,
    surface_configuration::SurfaceConfiguration,
    surface_view::SurfaceView,
};

/// Instance variables of [`BaseTerminalController`].
#[derive(Default)]
pub struct BaseTerminalControllerIvars {
    /// `bell` (TODO(MA3): published, drives the title bell indicator).
    pub bell: Cell<bool>,
    /// `titleOverride`.
    pub title_override: RefCell<Option<String>>,
}

define_class!(
    /// `BaseTerminalController`: the base class of all terminal window controllers.
    // SAFETY: NSWindowController has no subclassing requirements we violate;
    // no Drop impl.
    #[unsafe(super(NSWindowController, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GhosttyBaseTerminalController"]
    #[ivars = BaseTerminalControllerIvars]
    pub struct BaseTerminalController;
);

impl BaseTerminalController {
    /// `surfaceTree.isSplit`.
    #[must_use]
    pub fn surface_tree_is_split(&self) -> bool {
        warn!("TODO(MA3): BaseTerminalController::surface_tree_is_split");
        false
    }

    /// `surfaceTree.focusTarget(for:from:) != nil` for the node holding `view`
    /// (App.gotoSplit's performability check).
    #[must_use]
    pub fn focus_target_exists(
        &self,
        _direction: SplitFocusDirection,
        _view: &SurfaceView,
    ) -> bool {
        warn!("TODO(MA3): BaseTerminalController::focus_target_exists");
        false
    }

    /// `surfaceTree` as a flat list of its surface views (iterating the tree).
    #[must_use]
    pub fn surface_tree(&self) -> Vec<Retained<SurfaceView>> {
        warn!("TODO(MA3): BaseTerminalController::surface_tree");
        Vec::new()
    }

    /// `focusedSurface`.
    #[must_use]
    pub fn focused_surface(&self) -> Option<Retained<SurfaceView>> {
        warn!("TODO(MA3): BaseTerminalController::focused_surface");
        None
    }

    /// `toggleBackgroundOpacity()`.
    pub fn toggle_background_opacity(&self) {
        warn!("TODO(MA3): BaseTerminalController::toggle_background_opacity");
    }

    /// `promptTabTitle()`.
    pub fn prompt_tab_title(&self) {
        warn!("TODO(MA3): BaseTerminalController::prompt_tab_title");
    }

    /// `titleOverride = ...`.
    pub fn set_title_override(&self, title: Option<String>) {
        *self.ivars().title_override.borrow_mut() = title;
        warn!("TODO(MA3): BaseTerminalController::set_title_override (title not applied)");
    }

    /// `bell`.
    #[must_use]
    pub fn bell(&self) -> bool {
        self.ivars().bell.get()
    }

    /// `relabelTabs()`.
    pub fn relabel_tabs(&self) {
        warn!("TODO(MA3): BaseTerminalController::relabel_tabs");
    }
}

define_class!(
    /// `TerminalController`: the controller of a regular terminal window.
    // SAFETY: as above.
    #[unsafe(super(BaseTerminalController, NSWindowController, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GhosttyTerminalController"]
    #[ivars = ()]
    pub struct TerminalController;
);

impl TerminalController {
    /// `TerminalController.all`: every terminal controller, in creation order.
    #[must_use]
    pub fn all() -> Vec<Retained<TerminalController>> {
        Vec::new()
    }

    /// `TerminalController.newWindow(_:withBaseConfig:)`.
    #[must_use]
    pub fn new_window(
        _ghostty: &Rc<App>,
        _base_config: Option<&SurfaceConfiguration>,
    ) -> Option<Retained<TerminalController>> {
        warn!("TODO(MA3): TerminalController::new_window");
        None
    }

    /// `TerminalController.newTab(_:from:withBaseConfig:)`.
    #[must_use]
    pub fn new_tab(
        _ghostty: &Rc<App>,
        _from: Option<&NSWindow>,
        _base_config: Option<&SurfaceConfiguration>,
    ) -> Option<Retained<TerminalController>> {
        warn!("TODO(MA3): TerminalController::new_tab");
        None
    }

    /// `TerminalController.preferredParent`.
    #[must_use]
    pub fn preferred_parent() -> Option<Retained<TerminalController>> {
        None
    }

    /// `TerminalController.closeAllWindows()`.
    pub fn close_all_windows() {
        warn!("TODO(MA3): TerminalController::close_all_windows");
    }
}

/// `TerminalWindow.defaultLevelKey`.
pub const DEFAULT_LEVEL_KEY: &str = "TerminalDefaultLevel";

define_class!(
    /// `TerminalWindow`: the NSWindow subclass of terminal windows.
    // SAFETY: as above.
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GhosttyTerminalWindow"]
    #[ivars = ()]
    pub struct TerminalWindow;
);

impl TerminalWindow {
    /// `TerminalWindow.defaultLevelKey`.
    pub const DEFAULT_LEVEL_KEY: &'static str = DEFAULT_LEVEL_KEY;
}
