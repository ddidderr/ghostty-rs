//! Demo for the GTK app runtime with a dummy core.
//!
//! The dummy `CoreApp`/`CoreSurface` clear the `GLArea` with the configured
//! background color and implement just enough key handling (keybindings from
//! the user's config) to exercise windows, tabs, splits, dialogs, the command
//! palette, fullscreen, etc.
//!
//! Environment:
//! * `DEMO_QUIT_AFTER_MS=<n>`: quit automatically after n milliseconds.
//! * `DEMO_CONFIRM=1`: make every surface report "needs confirm quit".
//!
//! Run: `cargo run -p ghostty-apprt-gtk --example demo`

#![allow(unsafe_code)] // GL calls through glow

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use ghostty_apprt::{
    action::{self, Action, Target},
    runtime_api::{
        AppMessage,
        CellSize,
        ClipboardError,
        CoreApp,
        CoreError,
        CoreSurface,
        DesiredFontSize,
        InputEffect,
        KeyEvent,
        RtApp,
        RtSurface,
        SurfaceId,
    },
    structs::{ClipboardRequest, ColorScheme, ContentScale, CursorPos, ImePos, SurfaceSize},
};
use ghostty_apprt_gtk::app::App;
use ghostty_config::config::Config;
use ghostty_input::{
    Mods,
    MouseButton,
    MouseButtonState,
    ScrollMods,
    binding::{self, Action as Binding, set::Value as BindingValue},
};
use glow::HasContext;

fn confirm() -> bool {
    std::env::var("DEMO_CONFIRM").is_ok_and(|v| v == "1")
}

/// Map the subset of keybinding actions the demo understands to apprt actions.
fn binding_to_action(b: &Binding) -> Option<Action> {
    use binding::action::{CloseTabMode, SplitDirection, SplitFocusDirection};
    Some(match b {
        Binding::NewWindow => Action::NewWindow,
        Binding::NewTab => Action::NewTab,
        Binding::PreviousTab => Action::GotoTab(action::GotoTab::Previous),
        Binding::NextTab => Action::GotoTab(action::GotoTab::Next),
        Binding::LastTab => Action::GotoTab(action::GotoTab::Last),
        Binding::GotoTab(n) => Action::GotoTab(action::GotoTab::Index(*n as i32)),
        Binding::MoveTab(n) => Action::MoveTab(action::MoveTab { amount: *n }),
        Binding::ToggleTabOverview => Action::ToggleTabOverview,
        Binding::PromptSurfaceTitle => Action::PromptTitle(action::PromptTitle::Surface),
        Binding::PromptTabTitle => Action::PromptTitle(action::PromptTitle::Tab),
        Binding::NewSplit(d) => Action::NewSplit(match d {
            SplitDirection::Right | SplitDirection::Auto => action::SplitDirection::Right,
            SplitDirection::Down => action::SplitDirection::Down,
            SplitDirection::Left => action::SplitDirection::Left,
            SplitDirection::Up => action::SplitDirection::Up,
        }),
        Binding::GotoSplit(d) => Action::GotoSplit(match d {
            SplitFocusDirection::Previous => action::GotoSplit::Previous,
            SplitFocusDirection::Next => action::GotoSplit::Next,
            SplitFocusDirection::Up => action::GotoSplit::Up,
            SplitFocusDirection::Left => action::GotoSplit::Left,
            SplitFocusDirection::Down => action::GotoSplit::Down,
            SplitFocusDirection::Right => action::GotoSplit::Right,
        }),
        Binding::ToggleSplitZoom => Action::ToggleSplitZoom,
        Binding::EqualizeSplits => Action::EqualizeSplits,
        Binding::OpenConfig => Action::OpenConfig,
        Binding::ReloadConfig => Action::ReloadConfig(action::ReloadConfig::default()),
        Binding::CloseTab(m) => Action::CloseTab(match m {
            CloseTabMode::This => action::CloseTabMode::This,
            CloseTabMode::Other => action::CloseTabMode::Other,
            CloseTabMode::Right => action::CloseTabMode::Right,
        }),
        Binding::CloseWindow | Binding::CloseSurface => Action::CloseWindow,
        Binding::ToggleMaximize => Action::ToggleMaximize,
        Binding::ToggleFullscreen => Action::ToggleFullscreen(action::Fullscreen::Native),
        Binding::ToggleWindowDecorations => Action::ToggleWindowDecorations,
        Binding::ToggleCommandPalette => Action::ToggleCommandPalette,
        Binding::ToggleQuickTerminal => Action::ToggleQuickTerminal,
        Binding::ShowGtkInspector => Action::ShowGtkInspector,
        Binding::Quit => Action::Quit,
        _ => return None,
    })
}

// ---------------------------------------------------------------------

struct DemoSurface {
    id: SurfaceId,
    config: RefCell<Arc<Config>>,
    rt_app: Rc<dyn RtApp>,
    _rt_surface: Rc<dyn RtSurface>,
    gl: RefCell<Option<glow::Context>>,
    focused: Cell<bool>,
}

impl DemoSurface {
    fn perform(&self, a: Action) -> bool {
        self.rt_app.perform_action(Target::Surface(self.id), a)
    }
}

impl CoreSurface for DemoSurface {
    fn config(&self) -> Arc<Config> {
        self.config.borrow().clone()
    }

    fn key_callback(&self, event: KeyEvent<'_>) -> Result<InputEffect, CoreError> {
        if event.action == ghostty_input::Action::Release {
            return Ok(InputEffect::Ignored);
        }
        let config = self.config();
        let Some((_, value)) = config.keybind.set.get_event(&event) else {
            return Ok(InputEffect::Ignored);
        };
        let BindingValue::Leaf(leaf) = value else {
            return Ok(InputEffect::Ignored);
        };
        match binding_to_action(&leaf.action) {
            Some(a) => {
                let closes = matches!(a, Action::CloseWindow | Action::CloseTab(_));
                self.perform(a);
                Ok(if closes {
                    InputEffect::Closed
                } else {
                    InputEffect::Consumed
                })
            }
            None => Ok(InputEffect::Ignored),
        }
    }

    fn preedit_callback(&self, _text: Option<&[u8]>) -> Result<(), CoreError> {
        Ok(())
    }
    fn focus_callback(&self, focused: bool) -> Result<(), CoreError> {
        self.focused.set(focused);
        Ok(())
    }
    fn mouse_button_callback(
        &self,
        _state: MouseButtonState,
        _button: MouseButton,
        _mods: Mods,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }
    fn cursor_pos_callback(&self, _pos: CursorPos, _mods: Option<Mods>) -> Result<(), CoreError> {
        Ok(())
    }
    fn scroll_callback(&self, _x: f64, _y: f64, _mods: ScrollMods) -> Result<(), CoreError> {
        Ok(())
    }
    fn size_callback(&self, _size: SurfaceSize) -> Result<(), CoreError> {
        Ok(())
    }
    fn content_scale_callback(&self, _scale: ContentScale) -> Result<(), CoreError> {
        Ok(())
    }
    fn ime_point(&self) -> ImePos {
        ImePos {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 20.0,
        }
    }
    fn has_selection(&self) -> bool {
        false
    }
    fn color_scheme_callback(&self, _scheme: ColorScheme) -> Result<(), CoreError> {
        Ok(())
    }
    fn update_config(&self, config: &Arc<Config>) -> Result<(), CoreError> {
        *self.config.borrow_mut() = config.clone();
        self.perform(Action::ConfigChange(action::ConfigChange {
            config: config.clone(),
        }));
        Ok(())
    }
    fn perform_binding_action(&self, a: &Binding) -> Result<bool, CoreError> {
        Ok(binding_to_action(a).is_some_and(|a| self.perform(a)))
    }
    fn complete_clipboard_request(
        &self,
        _req: ClipboardRequest,
        data: &[u8],
        _confirmed: bool,
    ) -> Result<(), ClipboardError> {
        log::info!("demo: paste of {} bytes", data.len());
        Ok(())
    }
    fn set_clipboard_read_allowed(&self, _allow: bool) {}
    fn set_clipboard_write_allowed(&self, _allow: bool) {}
    fn needs_confirm_quit(&self) -> bool {
        confirm()
    }
    fn child_exited(&self) -> bool {
        false
    }
    fn renderer_realized(&self) -> Result<(), CoreError> {
        // SAFETY: the GLArea context is current while realizing; the loader
        // resolves symbols through libepoxy which dispatches to it.
        let gl =
            unsafe { glow::Context::from_loader_function(ghostty_apprt_gtk::gl::proc_address) };
        *self.gl.borrow_mut() = Some(gl);
        Ok(())
    }
    fn renderer_unrealized(&self) {
        *self.gl.borrow_mut() = None;
    }
    fn draw_frame(&self, default_framebuffer: u32) -> Result<(), CoreError> {
        let gl = self.gl.borrow();
        let Some(gl) = gl.as_ref() else {
            return Ok(());
        };
        let config = self.config();
        let bg = config.background;
        let a = config.background_opacity as f32;
        let (r, g, b) = (
            f32::from(bg.r) / 255.0,
            f32::from(bg.g) / 255.0,
            f32::from(bg.b) / 255.0,
        );
        // SAFETY: GL context is current (GLArea render signal).
        unsafe {
            gl.bind_framebuffer(
                glow::FRAMEBUFFER,
                std::num::NonZeroU32::new(default_framebuffer).map(glow::NativeFramebuffer),
            );
            // Premultiplied alpha.
            gl.clear_color(r * a, g * a, b * a, a);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------

#[derive(Default)]
struct DemoApp {
    surfaces: RefCell<Vec<SurfaceId>>,
    mailbox: RefCell<Vec<AppMessage>>,
}

impl CoreApp for DemoApp {
    fn tick(&self, rt: &dyn RtApp) -> Result<(), CoreError> {
        let msgs: Vec<AppMessage> = self.mailbox.borrow_mut().drain(..).collect();
        for m in msgs {
            match m {
                AppMessage::NewWindow => {
                    rt.perform_action(Target::App, Action::NewWindow);
                }
                AppMessage::OpenConfig => {
                    rt.perform_action(Target::App, Action::OpenConfig);
                }
                AppMessage::PresentSurface(id) => {
                    if self.surfaces.borrow().contains(&id) {
                        rt.perform_action(Target::Surface(id), Action::PresentTerminal);
                    }
                }
            }
        }
        Ok(())
    }

    fn update_config(&self, rt: &dyn RtApp, config: &Arc<Config>) -> Result<(), CoreError> {
        rt.perform_action(
            Target::App,
            Action::ConfigChange(action::ConfigChange {
                config: config.clone(),
            }),
        );
        Ok(())
    }
    fn needs_confirm_quit(&self) -> bool {
        confirm() && !self.surfaces.borrow().is_empty()
    }
    fn color_scheme_event(&self, _rt: &dyn RtApp, scheme: ColorScheme) {
        log::info!("demo: color scheme {scheme:?}");
    }
    fn perform_all_action(&self, rt: &dyn RtApp, a: &Binding) -> Result<(), CoreError> {
        if let Some(a) = binding_to_action(a) {
            rt.perform_action(Target::App, a);
        }
        Ok(())
    }
    fn push_message(&self, msg: AppMessage) {
        self.mailbox.borrow_mut().push(msg);
    }
    fn new_surface(
        &self,
        id: SurfaceId,
        config: Config,
        rt_app: Rc<dyn RtApp>,
        rt_surface: Rc<dyn RtSurface>,
    ) -> Result<Rc<dyn CoreSurface>, CoreError> {
        self.surfaces.borrow_mut().push(id);
        Ok(Rc::new(DemoSurface {
            id,
            config: RefCell::new(Arc::new(config)),
            rt_app,
            _rt_surface: rt_surface,
            gl: RefCell::new(None),
            focused: Cell::new(false),
        }))
    }
    fn delete_surface(&self, id: SurfaceId) {
        self.surfaces.borrow_mut().retain(|s| *s != id);
    }
    fn surface_ids(&self) -> Vec<SurfaceId> {
        self.surfaces.borrow().clone()
    }
    fn cell_size(&self, _config: &Config, _font_size: DesiredFontSize) -> Option<CellSize> {
        Some(CellSize {
            width: 9,
            height: 18,
        })
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let core = Rc::new(DemoApp::default());
    let app = App::new(core);

    if let Some(ms) = std::env::var("DEMO_QUIT_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        gtk::glib::timeout_add_local_once(Duration::from_millis(ms), || {
            ghostty_apprt_gtk::class::application::Application::default().quit();
        });
    }

    if let Err(err) = app.run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
    app.terminate();
}
