//! `AppDelegate+Ghostty.swift`: the Ghostty app delegate protocol
//! (`Ghostty.Delegate`), used by the libghostty glue for app-global
//! information.

use objc2::{MainThreadOnly, rc::Retained};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSObjectProtocol, NSUUID};

use super::app_delegate::AppDelegate;
use crate::{features::terminal::BaseTerminalController, ghostty::surface_view::SurfaceView};

impl AppDelegate {
    /// `ghosttySurface(id:)`: find a surface view by its id in any terminal window.
    #[must_use]
    pub fn ghostty_surface(&self, id: &NSUUID) -> Option<Retained<SurfaceView>> {
        for window in NSApplication::sharedApplication(self.mtm()).windows() {
            let Some(controller) = window
                .windowController()
                .and_then(|c| c.downcast::<BaseTerminalController>().ok())
            else {
                continue;
            };

            for surface in controller.surface_tree() {
                if surface.id().isEqual(Some(id)) {
                    return Some(surface);
                }
            }
        }

        None
    }
}
