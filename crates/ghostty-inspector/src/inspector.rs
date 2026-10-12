//! The Inspector (ZIG/src/inspector/Inspector.zig).

use std::sync::{Mutex, MutexGuard, PoisonError};

use dear_imgui_rs::{ConfigFlags, Context, FontSource, StbTrueTypeFontData, Ui};
use ghostty_renderer::{overlay::Feature, state::InspectorHook};
use ghostty_terminal::terminal::Terminal;

use crate::widgets;

/// Setup the ImGui state. This requires an ImGui context.
///
/// `scale_factor` is the content scale of the display; Zig hardcodes a 2x
/// scale here (with a TODO) and sizes the font as `16 * 2`, this uses
/// `16 * scale_factor` instead.
pub fn setup(ctx: &mut Context, scale_factor: f64) {
    let io = ctx.io_mut();

    // Enable docking, which we use heavily for the UI. Our colorspace is
    // sRGB.
    io.set_config_flags(io.config_flags() | ConfigFlags::DOCKING_ENABLE | ConfigFlags::IS_SRGB);

    // Disable the ini file to save layout
    if let Err(err) = ctx.set_ini_filename::<std::path::PathBuf>(None) {
        log::warn!("failed to disable the imgui ini file err={err}");
    }
    if let Err(err) = ctx.set_log_filename::<std::path::PathBuf>(None) {
        log::warn!("failed to disable the imgui log file err={err}");
    }

    // Use our own embedded font
    let font_size = (16.0 * scale_factor.max(1.0)) as f32;
    match StbTrueTypeFontData::from_slice(ghostty_font::embedded::REGULAR) {
        Ok(data) => {
            ctx.font_atlas()
                .add_font(&[FontSource::stb_truetype_with_size(data, font_size)]);
        }
        Err(err) => log::warn!("failed to load the embedded inspector font err={err}"),
    }
}

/// The Inspector is a development tool to debug the terminal. This is
/// useful for terminal application developers as well as people potentially
/// debugging issues in Ghostty itself.
#[derive(Debug, Default)]
pub struct Inspector {
    /// Mouse state that we track in addition to normal mouse states that
    /// Ghostty always knows about.
    pub mouse: widgets::surface::Mouse,

    // ImGui state
    pub gui: widgets::surface::Inspector,
}

impl Inspector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the renderer info panel. This is a convenience function
    /// to access and find this state to read and modify.
    pub fn renderer_info(&mut self) -> &mut widgets::renderer::Info {
        &mut self.gui.renderer_info
    }

    /// Record a keyboard event.
    pub fn record_key_event(&mut self, ev: widgets::key::Event) {
        self.gui.key_stream.record(ev);
    }

    /// Record data read from the pty.
    pub fn record_pty_read(&mut self, t: &Terminal, data: &[u8]) {
        self.gui.vt_stream.record_pty_read(t, data);
    }

    /// Render the frame. The caller must hold the terminal lock and provide
    /// the surface information.
    pub fn render(&mut self, ui: &Ui, surface: &widgets::surface::SurfaceInfo, t: &mut Terminal) {
        // Draw the UI
        self.gui.draw(ui, surface, &self.mouse, t);
    }
}

/// The inspector as shared between the surface, the IO thread (VT events)
/// and the renderer (overlay features). Zig shares a pointer guarded by the
/// renderer mutex; the lock order is: renderer state, then this.
#[derive(Debug, Default)]
pub struct Shared(Mutex<Inspector>);

impl Shared {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock the inspector (poisoning is ignored: the data is display-only).
    pub fn lock(&self) -> MutexGuard<'_, Inspector> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl InspectorHook for Shared {
    fn overlay_features(&self) -> Vec<Feature> {
        self.lock().gui.renderer_info.overlay_features()
    }

    fn record_pty_read(&self, t: &Terminal, data: &[u8]) {
        self.lock().record_pty_read(t, data);
    }
}

#[cfg(test)]
mod tests {
    use ghostty_input::{Action, Key, KeyEvent};
    use ghostty_renderer::size::{CellSize, Padding, ScreenSize, Size};
    use ghostty_terminal::terminal::Options;
    use ghostty_vt::stream::Stream;

    use super::*;
    use crate::widgets::surface::SurfaceInfo;

    fn surface_info() -> SurfaceInfo {
        SurfaceInfo {
            size: Size {
                screen: ScreenSize {
                    width: 800,
                    height: 600,
                },
                cell: CellSize {
                    width: 10,
                    height: 20,
                },
                padding: Padding::default(),
            },
            font_size_points: 12.0,
            font_size_pixels: 16.0,
            click_pressed: [false; 12],
            left_click_pin: None,
            left_click_xpos: 0.0,
            left_click_ypos: 0.0,
        }
    }

    /// Draw a few full frames headlessly (no renderer) with every window
    /// open and the detached/pop-out state enabled, to catch Dear ImGui
    /// misuse (the safe wrapper panics on invalid API use).
    #[test]
    fn draws_frames_headless() {
        let mut ctx = Context::create();
        setup(&mut ctx, 1.0);
        ctx.io_mut().set_display_size([1000.0, 700.0]);
        ctx.io_mut().set_delta_time(1.0 / 60.0);
        ctx.font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy font atlas")
            .build();

        let mut t = Terminal::new(Options::new(80, 24)).expect("terminal");
        let mut insp = Inspector::new();

        // Feed some pty data through both the inspector and the terminal.
        let data =
            b"hello\x07\x1b[31mred\x1b[0m\r\n\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\\x1b]0;t\x07";
        insp.record_pty_read(&t, data);
        {
            let mut term_stream =
                Stream::new(ghostty_terminal::stream_readonly::Handler::new(&mut t));
            term_stream.next_slice(data).expect("process");
        }

        insp.record_key_event(crate::KeyEvent::new(&KeyEvent {
            action: Action::Press,
            key: Key::KeyA,
            utf8: "a".as_bytes(),
            ..KeyEvent::default()
        }));

        for frame in 0..8 {
            // Open the pop-out state after the layout has settled, like a
            // user clicking the buttons.
            if frame == 3 {
                insp.gui.terminal_info.misc_header.detached = true;
                insp.gui.terminal_info.layout_header.detached = true;
            }
            if frame == 2 {
                insp.gui.vt_stream.selected_event_seq = Some(1);
            }
            if frame == 4 {
                insp.gui.terminal_info.screens[0] = Some(crate::widgets::screen::Info::default());
                insp.gui.terminal_info.show_palette = true;
            }
            let ui = ctx.frame();
            insp.render(ui, &surface_info(), &mut t);
            let _ = ctx.render_legacy();
        }
        assert!(!insp.gui.vt_stream.events().is_empty());
    }
}
