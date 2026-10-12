//! The terminal inspector hooks of the surface (`Surface.zig`:
//! `activateInspector`, `deactivateInspector` and the inspector parts of the
//! key and mouse callbacks).

use std::sync::Arc;

use ghostty_apprt::runtime_api::CoreError;
use ghostty_inspector::Shared as Inspector;
use ghostty_inspector::imgui::Ui;
use ghostty_inspector::widgets::surface::SurfaceInfo;
use ghostty_renderer::message::Message as RendererMessage;
use ghostty_renderer::state::InspectorHook;
use ghostty_termio::message::Message as IoMessage;

use super::Surface;

impl Surface {
    /// The active inspector, if any.
    pub fn inspector(&self) -> Option<Arc<Inspector>> {
        self.inspector.borrow().clone()
    }

    /// Activate the inspector. This will begin collecting inspection data.
    /// This will not affect the GUI. The GUI must use `perform_action` to
    /// show/hide the inspector UI.
    pub fn activate_inspector(&self) -> Result<(), CoreError> {
        if self.inspector.borrow().is_some() {
            return Ok(());
        }

        // Setup the inspector
        let insp = Arc::new(Inspector::new());
        *self.inspector.borrow_mut() = Some(Arc::clone(&insp));

        // Put the inspector onto the render state
        {
            let mut st = self.lock_state();
            debug_assert!(st.inspector.is_none());
            st.inspector = Some(insp as Arc<dyn InspectorHook>);
        }

        // Notify our components we have an inspector active
        self.send_renderer(RendererMessage::Inspector(true));
        self.queue_io(IoMessage::Inspector(true));
        Ok(())
    }

    /// Deactivate the inspector and stop collecting any information.
    pub fn deactivate_inspector(&self) {
        if self.inspector.borrow().is_none() {
            return;
        }

        // Remove the inspector from the render state
        {
            let mut st = self.lock_state();
            debug_assert!(st.inspector.is_some());
            st.inspector = None;
        }

        // Notify our components we have deactivated inspector
        self.send_renderer(RendererMessage::Inspector(false));
        self.queue_io(IoMessage::Inspector(false));

        // Drop the inspector
        *self.inspector.borrow_mut() = None;
    }

    /// Draw the inspector UI into the current Dear ImGui frame
    /// (`Inspector.render`). Does nothing if the inspector isn't active.
    pub fn render_inspector(&self, ui: &Ui) {
        let Some(insp) = self.inspector() else {
            return;
        };

        // The information about the surface that the widgets show.
        let info = {
            let font_size = self.font.borrow().size;
            let m = self.mouse.borrow();
            let mut click_pressed = [false; 12];
            for (i, state) in m.click_state.iter().enumerate().take(click_pressed.len()) {
                click_pressed[i] = *state == ghostty_input::MouseButtonState::Press;
            }
            (
                self.size.get(),
                font_size,
                click_pressed,
                m.left_click_pin,
                m.left_click_screen,
                m.left_click_xpos,
                m.left_click_ypos,
            )
        };

        {
            // The renderer mutex protects the terminal; lock order is the
            // render state first, then the inspector.
            let mut guard = self.lock_terminal();
            let (size, font_size, click_pressed, pin, pin_screen, xpos, ypos) = info;
            let left_click_pin = if pin_screen == guard.screens.active_key {
                pin.and_then(|h| guard.screens.active().pages.tracked_pin(h))
            } else {
                None
            };
            let surface = SurfaceInfo {
                size,
                font_size_points: font_size.points,
                font_size_pixels: font_size.pixels(),
                click_pressed,
                left_click_pin,
                left_click_xpos: xpos,
                left_click_ypos: ypos,
            };

            insp.lock().render(ui, &surface, &mut guard);
        }

        // We always trigger a rebuild of the surface when the inspector
        // is focused because modifying the inspector can change the terminal
        // state. This is KIND OF expensive (wasted CPU if nothing was done)
        // but the inspector is a development tool and it expressly costs
        // more resources while open so its okay.
        self.queue_render();
    }

    /// Record the mouse position for the inspector (the inspector block of
    /// `cursorPosCallback`).
    pub(crate) fn inspector_record_cursor_pos(&self, xpos: f64, ypos: f64) {
        let Some(insp) = self.inspector() else {
            return;
        };

        let pos_vp = self.pos_to_viewport(xpos, ypos);
        let pin = self
            .lock_terminal()
            .screens
            .active()
            .pages
            .pin(ghostty_terminal::point::Point::Viewport(pos_vp));
        {
            let mut guard = insp.lock();
            guard.mouse.last_xpos = xpos;
            guard.mouse.last_ypos = ypos;
            guard.mouse.last_point = pin;
        }
        self.queue_render();
    }
}
