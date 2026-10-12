//! Surface represents a single terminal "surface" (port of `Surface.zig`).
//!
//! A terminal surface is a minimal "widget" where the terminal is drawn and
//! responds to events such as keyboard and mouse. Each surface also creates
//! and owns its pty session.
//!
//! The word "surface" is used because it is left to the higher level
//! application runtime to determine if the surface is a window, a tab, a
//! split, a preview pane in a larger window, etc. This struct doesn't care:
//! it just draws and responds to events. The events come from the
//! application runtime so the runtime can determine when and how those are
//! delivered (i.e. with focus, without focus, and so on).
//!
//! # Structure
//!
//! The Zig file is split into several Rust modules that all add `impl
//! Surface` blocks:
//!
//! * this file: the struct, creation/teardown (threads), the message
//!   handler, config updates, sizing, fonts, focus, preedit, selection and
//!   the [`CoreSurface`] implementation;
//! * [`clipboard`]: OSC 52, paste (safety, bracketed), desktop notifications;
//! * [`keyboard`]: key events, bindings, sequences, key tables, key encoding;
//! * [`mouse`]: mouse buttons/motion/scroll, mouse reporting, selection by
//!   mouse, links;
//! * [`binding`]: `performBindingAction`, search, screen file dumps;
//! * [`config`]: the derived configuration.
//!
//! # Threading and interior mutability
//!
//! Every method runs on the runtime's main thread. Other threads (renderer,
//! IO, search) talk to the surface through the app [`Mailbox`]. Methods take
//! `&self`; state lives in `Cell`/`RefCell`. **Never hold a `RefCell` borrow
//! or the terminal lock across a call into the runtime** (`rt_app`,
//! `rt_surface`): the runtime may call straight back into this surface.

pub mod binding;
pub mod clipboard;
pub mod config;
pub mod inspector;
pub mod keyboard;
pub mod mouse;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    thread::JoinHandle,
    time::{Duration as StdDuration, Instant},
};

use config::DerivedConfig;
use ghostty_apprt::{
    action::{
        Action as RtAction,
        CellSize as ActionCellSize,
        ColorChange,
        ColorKind,
        CommandFinished,
        ConfigChange,
        InitialSize,
        MouseVisibility,
        Pwd,
        ReloadConfig,
        SecureInput,
        SetTitle,
        SizeLimit,
        Target,
    },
    runtime_api::{
        ClipboardError,
        CoreError,
        CoreSurface,
        DesiredFontSize,
        InputEffect,
        RtApp,
        RtSurface,
        SurfaceId,
    },
    structs::{ClipboardRequest, ColorScheme, ContentScale, CursorPos, ImePos, SurfaceSize},
    surface::{Mailbox as SurfaceMailboxImpl, Message},
};
use ghostty_config::{conditional, config::Config, types_a::ConfirmCloseSurface};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_font::{
    face::DesiredSize,
    metrics::Metrics,
    shared_grid_set::{DerivedConfig as FontDerivedConfig, Key as FontGridKey},
};
use ghostty_input::{Mods, MouseButton, MouseButtonState, ScrollMods};
use ghostty_renderer::{
    Api as GraphicsApi,
    Health,
    // The renderer over the graphics API selected for the target (Zig:
    // `renderer.zig`): Metal on Apple targets, OpenGL elsewhere.
    Renderer,
    generic::Host as RendererHost,
    message::{FontGridChange, Message as RendererMessage},
    options::Options as RendererOptions,
    size::{CellSize, Coordinate, Padding, ScreenSize, Size},
    state::{Preedit, PreeditCodepoint, State as RenderStateData},
    thread::{Handle as RendererHandle, Running as RendererRunning},
};
use ghostty_terminal::{
    page_list::Scrollbar,
    point::Coordinate as PointCoordinate,
    selection::Selection,
};
use ghostty_termio::{
    Backend,
    DerivedConfig as IoDerivedConfig,
    Exec,
    Mailbox as IoMailbox,
    Options as IoOptions,
    RendererMailbox,
    SharedState,
    SharedStateExt,
    TerminalGuard,
    Termio,
    Thread as IoThread,
    Wakeup,
    exec,
    message::Message as IoMessage,
};

use crate::{
    app::{AppState, Mailbox, Message as AppMessage},
    global,
};

/// Minimum window size in cells (Zig: `min_window_width_cells`).
pub const MIN_WINDOW_WIDTH_CELLS: u32 = ghostty_apprt::runtime_api::MIN_WINDOW_WIDTH_CELLS;
/// See [`MIN_WINDOW_WIDTH_CELLS`].
pub const MIN_WINDOW_HEIGHT_CELLS: u32 = ghostty_apprt::runtime_api::MIN_WINDOW_HEIGHT_CELLS;

/// The default DPI (Zig: `font.face.default_dpi`).
pub const DEFAULT_DPI: f32 = 96.0;

/// The effect of an input event (Zig: `Surface.InputEffect`).
pub type InputEffectT = InputEffect;

/// Mouse button count (Zig: `input.MouseButton.max`).
pub const MOUSE_BUTTON_COUNT: usize = 12;

/// The surface mailbox type termio and the renderer use to message us.
type SurfaceMailboxT = SurfaceMailboxImpl<SurfaceId, Mailbox>;

/// Bridge from termio to the renderer thread's mailbox. The IO side is
/// created before the renderer thread exists (the renderer needs the shared
/// terminal state which termio creates), so the handle is filled in later.
#[derive(Default)]
struct RendererLink {
    handle: std::sync::OnceLock<RendererHandle>,
}

impl RendererMailbox for RendererLink {
    fn push(&self, msg: RendererMessage, timeout: Timeout) -> usize {
        match self.handle.get() {
            Some(h) => usize::from(h.push(msg, timeout)),
            None => 0,
        }
    }
}

/// What the renderer needs from us (thread-safe, non-blocking).
struct HostImpl {
    id: SurfaceId,
    mailbox: Mailbox,
}

impl RendererHost for HostImpl {
    fn scrollbar(&self, scrollbar: Scrollbar) -> bool {
        self.mailbox.push(
            AppMessage::SurfaceMessage {
                surface: self.id,
                message: Message::Scrollbar(scrollbar),
            },
            Timeout::Instant,
        ) > 0
    }

    fn renderer_health(&self, health: Health) {
        self.mailbox.push(
            AppMessage::SurfaceMessage {
                surface: self.id,
                message: Message::RendererHealth(health),
            },
            Timeout::Forever,
        );
    }

    fn redraw_surface(&self) -> bool {
        self.mailbox
            .push(AppMessage::RedrawSurface(self.id), Timeout::Instant)
            > 0
    }
}

/// Create the graphics API for a new surface (Zig: `Renderer.surfaceInit`).
#[cfg(not(target_vendor = "apple"))]
fn init_graphics_api(
    config: &Config,
    rt_surface: &dyn RtSurface,
) -> Result<GraphicsApi, CoreError> {
    let mut api = GraphicsApi::init(config.alpha_blending);
    api.prepare_context(|name| rt_surface.gl_proc_address(name))
        .map_err(|e| format!("failed to prepare the OpenGL context: {e}"))?;
    Ok(api)
}

/// Create the graphics API for a new surface (Zig: `Renderer.surfaceInit`):
/// the Metal layer is attached to the NSView/UIView the host provided.
#[cfg(target_vendor = "apple")]
fn init_graphics_api(
    config: &Config,
    rt_surface: &dyn RtSurface,
) -> Result<GraphicsApi, CoreError> {
    let view = rt_surface.platform_view();
    let scale = f64::from(rt_surface.get_content_scale().x);
    GraphicsApi::init_with_view(config.alpha_blending, view, scale)
        .map_err(|e| format!("failed to initialize the Metal renderer: {e}").into())
}

/// Font state of the surface.
pub struct FontState {
    /// The key of the font grid we hold a reference on.
    pub grid_key: FontGridKey,
    /// The desired size (points + DPI).
    pub size: DesiredSize,
    /// The metrics of the current grid.
    pub metrics: Metrics,
    /// This keeps track of if the font size was ever modified. If it wasn't,
    /// then config reloading will change the font. If it was manually
    /// adjusted, we don't change it on config reload since we assume the
    /// user wants a specific size.
    pub adjusted: bool,
}

/// A single terminal surface. See the module docs.
pub struct Surface {
    /// The identifier the runtime gave us (Zig: the `*Surface` pointer).
    pub(crate) id: SurfaceId,

    /// The app that this surface is attached to.
    pub(crate) app: Rc<AppState>,

    /// The windowing system surface and app.
    pub(crate) rt_app: Rc<dyn RtApp>,
    pub(crate) rt_surface: Rc<dyn RtSurface>,

    /// The font structures.
    pub(crate) font: RefCell<FontState>,

    /// The renderer for this surface. It is created and destroyed on the
    /// main thread (GL context current) and drawn from the main thread.
    pub(crate) renderer: Arc<Renderer>,

    /// The render state (terminal + preedit + mouse) behind the renderer
    /// mutex (`renderer_state.mutex` in Zig).
    pub(crate) renderer_state: SharedState,

    /// The renderer thread.
    pub(crate) renderer_handle: RendererHandle,
    renderer_thread: RefCell<Option<RendererRunning>>,

    /// The IO thread and the mailbox used to send it messages.
    pub(crate) io_mailbox: IoMailbox,
    io_stop: Wakeup,
    io_thread: RefCell<Option<JoinHandle<()>>>,

    /// The command arguments the IO backend was started with (for the
    /// abnormal exit message).
    command_args: Vec<String>,

    /// Mouse state.
    pub(crate) mouse: RefCell<mouse::Mouse>,

    /// Keyboard input state.
    pub(crate) keyboard: RefCell<keyboard::Keyboard>,

    /// A currently pressed key. This is used so that we can send a keyboard
    /// release event when the surface is unfocused. See Zig docs.
    pub(crate) pressed_key: RefCell<Option<keyboard::OwnedKeyEvent>>,

    /// All our sizing information.
    pub(crate) size: Cell<Size>,

    /// The dimensions recorded in crash reports (see `ghostty_crash`).
    crash_dims: Arc<ghostty_crash::SurfaceDims>,

    /// The configuration derived from the main config.
    config: RefCell<Rc<DerivedConfig>>,

    /// The conditional state of the configuration.
    pub(crate) config_conditional_state: Cell<conditional::State>,

    /// This is set to true if our IO thread notifies us our child exited.
    pub(crate) child_exited: Cell<bool>,

    /// We maintain our focus state and assume we're focused by default.
    pub(crate) focused: Cell<bool>,

    /// Used to determine whether to continuously scroll.
    pub(crate) selection_scroll_active: Cell<bool>,

    /// True if the surface is in read-only mode. When read-only, no input is
    /// sent to the PTY but terminal-level operations like selections,
    /// (native) scrolling, and copy keybinds still work.
    pub(crate) readonly: Cell<bool>,

    /// Used to send notifications that long running commands have finished.
    command_timer: Cell<Option<Instant>>,

    /// Search state.
    pub(crate) search: RefCell<Option<binding::Search>>,

    /// Used to rate limit BEL handling.
    last_bell_time: Cell<Option<Instant>>,

    /// The terminal inspector, if active (Zig: `inspector: ?*Inspector`).
    pub(crate) inspector: RefCell<Option<Arc<ghostty_inspector::Shared>>>,

    /// Debug screenshot state (see `debug_screenshot`).
    created: Instant,
    debug_shot_done: Cell<bool>,
}

impl Surface {
    /// Create a new surface (Zig: `Surface.init`). This must be called from
    /// the main thread with the GL context current.
    pub fn new(
        id: SurfaceId,
        app: Rc<AppState>,
        config_original: Arc<Config>,
        rt_app: Rc<dyn RtApp>,
        rt_surface: Rc<dyn RtSurface>,
    ) -> Result<Rc<Surface>, CoreError> {
        // Apply our conditional state. If we fail to apply the conditional
        // state then we log and attempt to move forward with the old config.
        let config: Arc<Config> =
            match config_original.change_conditional_state(app.config_conditional_state.get()) {
                Ok(Some(mut c)) => {
                    // We want to preserve our original working directory. We
                    // preserve this so directory inheritance works.
                    c.working_directory = config_original.working_directory.clone();
                    Arc::new(c)
                }
                Ok(None) => Arc::clone(&config_original),
                Err(err) => {
                    log::warn!("failed to apply conditional state to config err={err}");
                    Arc::clone(&config_original)
                }
            };

        // Get our configuration
        let derived_config = Rc::new(DerivedConfig::new(Arc::clone(&config))?);

        // Initialize our renderer with our initialized surface (Zig:
        // `Renderer.surfaceInit`): load the GL functions for the current
        // context (OpenGL) or attach the Metal layer to the host's view.
        let api = init_graphics_api(&config, rt_surface.as_ref())?;

        // Determine our DPI configurations so we can properly configure
        // font points to pixels and handle other high-DPI scaling factors.
        let content_scale = rt_surface.get_content_scale();
        let x_dpi = content_scale.x * DEFAULT_DPI;
        let y_dpi = content_scale.y * DEFAULT_DPI;
        log::debug!(
            "xscale={} yscale={} xdpi={x_dpi} ydpi={y_dpi}",
            content_scale.x,
            content_scale.y
        );

        // The font size we desire along with the DPI determined for the
        // surface.
        let font_size = DesiredSize {
            points: config.font_size,
            xdpi: x_dpi as u16,
            ydpi: y_dpi as u16,
        };

        // Setup our font group. This will reuse an existing font group if it
        // was already loaded.
        let font_derived = FontDerivedConfig::new(&config);
        let (font_grid_key, font_grid) = app.font_grid_set.ref_(&font_derived, font_size)?;

        // Build our size struct which has all the sizes we need.
        let size = {
            let surface_size = rt_surface.get_size();
            let mut size = Size {
                screen: ScreenSize {
                    width: surface_size.width,
                    height: surface_size.height,
                },
                cell: {
                    let c = font_grid.cell_size();
                    CellSize {
                        width: c.width,
                        height: c.height,
                    }
                },
                padding: Padding::default(),
            };

            let explicit = derived_config.scaled_padding(x_dpi, y_dpi);
            if derived_config.window_padding_balance {
                size.balance_padding(explicit);
            } else {
                size.padding = explicit;
            }
            size
        };

        let app_mailbox = app.mailbox.clone();
        let host: Arc<dyn RendererHost> = Arc::new(HostImpl {
            id,
            mailbox: app_mailbox.clone(),
        });

        // The renderer itself.
        let renderer = Arc::new(Renderer::new(
            RendererOptions {
                config: ghostty_renderer::DerivedConfig::new(&config)?,
                font_grid: Arc::clone(&font_grid),
                size,
                host: Arc::clone(&host),
            },
            api,
        )?);

        // The command we're going to execute
        let command = if app.first.get() {
            config
                .initial_command
                .clone()
                .or_else(|| config.command.clone())
        } else {
            config.command.clone()
        };

        // Start our IO implementation
        let renderer_link = Arc::new(RendererLink::default());
        let renderer_wakeup = {
            let link = Arc::clone(&renderer_link);
            Wakeup::with_hook(Arc::new(move || {
                if let Some(h) = link.handle.get() {
                    h.wakeup();
                }
            }))?
        };
        let io_mailbox = IoMailbox::new()?;
        let surface_mailbox: Arc<dyn ghostty_termio::SurfaceMailbox> = Arc::new(SurfaceMailboxT {
            surface: id,
            app: app_mailbox.clone(),
        });

        let (termio, command_args) = {
            let mut env = rt_surface_env(rt_surface.as_ref());
            // don't leak GHOSTTY_LOG to any subprocesses
            env.remove("GHOSTTY_LOG");

            let mut ecfg = exec::Config::new(
                env,
                ghostty_os::osbytes::EncodedOs::from_encoded(config.term.clone()),
            );
            ecfg.command = command.clone();
            ecfg.env_override = config.env.clone();
            ecfg.shell_integration = config.shell_integration;
            ecfg.shell_integration_features = config.shell_integration_features;
            ecfg.cursor_blink = config.cursor_style_blink;
            ecfg.working_directory = config
                .working_directory
                .as_ref()
                .and_then(|wd| wd.value())
                .cloned();
            ecfg.resources_dir = global::state().resources_dir.host().cloned();
            // The Windows exec backend has no pre-exec/post-fork hooks,
            // cgroup scopes or rlimits (Zig: Linux/POSIX only).
            #[cfg(unix)]
            {
                ecfg.rt_pre_exec_info = ghostty_termio::command::RtPreExecInfo::init(&config);
                ecfg.rt_post_fork_info = ghostty_termio::command::RtPostForkInfo::init(&config);
                ecfg.create_scope = rt_surface.create_scope_fn();
                ecfg.restore_rlimit = global::state().rlimits.nofile;
            }

            let exec = Exec::new(ecfg)?;
            let args = exec.args().to_vec();
            let termio = Termio::init(IoOptions {
                size,
                full_config: &config,
                config: IoDerivedConfig::new(&config),
                backend: Backend::Exec(exec),
                mailbox: io_mailbox.clone(),
                renderer_wakeup,
                renderer_mailbox: Arc::clone(&renderer_link) as Arc<dyn RendererMailbox>,
                surface_mailbox: Arc::clone(&surface_mailbox),
            })?;
            (termio, args)
        };
        let renderer_state = Arc::clone(termio.renderer_state());

        // Create the renderer thread
        let mut render_thread = ghostty_renderer::Thread::new(
            &config,
            Arc::clone(&renderer),
            Arc::clone(&renderer_state),
            host,
        );
        let renderer_handle = render_thread.handle();
        let _ = renderer_link.handle.set(renderer_handle.clone());

        // Create the IO thread
        let mut io_thread = IoThread::new()?;
        let io_stop = io_thread.stop_handle();

        // Crash context shared with the renderer and IO threads.
        let crash_dims = ghostty_crash::SurfaceDims::new();
        render_thread.crash_dims = Some(Arc::clone(&crash_dims));
        io_thread.crash_dims = Some(Arc::clone(&crash_dims));

        let surface = Rc::new(Surface {
            id,
            app: Rc::clone(&app),
            rt_app: Rc::clone(&rt_app),
            rt_surface: Rc::clone(&rt_surface),
            font: RefCell::new(FontState {
                grid_key: font_grid_key,
                size: font_size,
                metrics: font_grid.metrics(),
                adjusted: false,
            }),
            renderer,
            renderer_state,
            renderer_handle,
            renderer_thread: RefCell::new(None),
            io_mailbox,
            io_stop,
            io_thread: RefCell::new(None),
            command_args,
            mouse: RefCell::new(mouse::Mouse::default()),
            keyboard: RefCell::new(keyboard::Keyboard::default()),
            pressed_key: RefCell::new(None),
            size: Cell::new(size),
            crash_dims,
            config: RefCell::new(derived_config),
            config_conditional_state: Cell::new(app.config_conditional_state.get()),
            child_exited: Cell::new(false),
            focused: Cell::new(true),
            selection_scroll_active: Cell::new(false),
            readonly: Cell::new(false),
            command_timer: Cell::new(None),
            search: RefCell::new(None),
            last_bell_time: Cell::new(None),
            inspector: RefCell::new(None),
            created: Instant::now(),
            debug_shot_done: Cell::new(false),
        });

        // Report initial cell size on surface creation
        rt_app.perform_action(
            Target::Surface(id),
            RtAction::CellSize(ActionCellSize {
                width: size.cell.width,
                height: size.cell.height,
            }),
        );

        rt_app.perform_action(
            Target::Surface(id),
            RtAction::SizeLimit(SizeLimit {
                min_width: size.cell.width * MIN_WINDOW_WIDTH_CELLS,
                min_height: size.cell.height * MIN_WINDOW_HEIGHT_CELLS,
                // No max:
                max_width: 0,
                max_height: 0,
            }),
        );

        // Call our size callback which handles all our retina setup
        // Note: this shouldn't be necessary and when we clean up the surface
        // init stuff we should get rid of this. But this is required because
        // sizeCallback does retina-aware stuff we don't do here and don't
        // want to duplicate.
        surface.resize(size.screen);

        // Start our renderer thread
        *surface.renderer_thread.borrow_mut() = Some(render_thread.spawn()?);

        // Start our IO thread
        *surface.io_thread.borrow_mut() = Some(
            std::thread::Builder::new()
                .name("io".into())
                .spawn(move || io_thread.thread_main(termio))?,
        );

        // Determine our initial window size if configured. We need to do
        // this quite late in the process because our height/width are in
        // grid dimensions, so we need to know our cell sizes first.
        //
        // Note: it is important to do this after the renderer is setup
        // above. This allows the apprt to fully initialize the surface
        // before we start messing with the window.
        surface.recompute_initial_size();

        if let Some(title) = &config.title {
            rt_app.perform_action(
                Target::Surface(id),
                RtAction::SetTitle(SetTitle {
                    title: ghostty_os::osbytes::config_str_to_text(title),
                }),
            );
        } else if config.xdg_terminal_exec
            && let Some(cmd) = &command
        {
            // For xdg-terminal-exec execution we special-case and set the
            // window title to the command being executed. This allows window
            // managers to set custom styling based on the command being
            // executed.
            rt_app.perform_action(
                Target::Surface(id),
                RtAction::SetTitle(SetTitle {
                    title: ghostty_os::osbytes::config_str_to_text(&cmd.string()),
                }),
            );
        } else if let Some(cmd) = &command {
            // If a user specifies a command it is appropriate to set the
            // title as argv[0]. We know in the case of a direct command it
            // has been supplied by the user. We won't set the title in the
            // case the shell expands the command as that should typically be
            // used to launch a shell which should set its own titles.
            if let ghostty_config::command::Command::Direct(args) = cmd
                && let Some(first) = args.first()
                && !first.is_empty()
            {
                rt_app.perform_action(
                    Target::Surface(id),
                    RtAction::SetTitle(SetTitle {
                        title: ghostty_os::osbytes::config_str_to_text(first),
                    }),
                );
            }
        }

        // We are no longer the first surface
        app.first.set(false);

        Ok(surface)
    }

    // ------------------------------------------------------------------
    // Small shared helpers used by every submodule

    /// The runtime-assigned id of this surface.
    #[must_use]
    pub fn id(&self) -> SurfaceId {
        self.id
    }

    /// The current derived config (cheap `Rc` clone; no borrow is kept).
    #[must_use]
    pub fn cfg(&self) -> Rc<DerivedConfig> {
        Rc::clone(&self.config.borrow())
    }

    /// Lock the renderer state and borrow the terminal (the Zig
    /// `renderer_state.mutex.lock()` + `io.terminal`). Do not hold across
    /// calls into the runtime.
    pub fn lock_terminal(&self) -> TerminalGuard<'_> {
        self.renderer_state.lock_terminal()
    }

    /// Lock the full render state (terminal + preedit + mouse).
    pub fn lock_state(&self) -> std::sync::MutexGuard<'_, RenderStateData> {
        self.renderer_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Perform a runtime action targeted at this surface, ignoring the
    /// result (most callers only log failures in Zig).
    pub(crate) fn perform(&self, action: RtAction) -> bool {
        self.rt_app.perform_action(Target::Surface(self.id), action)
    }

    /// Returns a mailbox that can be used to send messages to this surface.
    pub(crate) fn surface_mailbox(&self) -> SurfaceMailboxT {
        SurfaceMailboxT {
            surface: self.id,
            app: self.app.mailbox.clone(),
        }
    }

    /// Queue a message for the IO thread.
    ///
    /// We centralize all our logic into this spot so we can intercept
    /// messages for example in readonly mode. Do NOT call this while holding
    /// the terminal lock (use [`Surface::queue_io_locked`]).
    pub(crate) fn queue_io(&self, msg: IoMessage) {
        if self.readonly.get() && matches!(msg, IoMessage::Write(_)) {
            return;
        }
        self.io_mailbox.send(msg);
        self.io_mailbox.notify();
    }

    #[allow(dead_code)]
    /// Like [`Surface::queue_io`] for callers holding the renderer lock: if
    /// the queue is full the lock is released while blocking.
    pub(crate) fn queue_io_locked<'a>(
        &'a self,
        msg: IoMessage,
        guard: std::sync::MutexGuard<'a, RenderStateData>,
    ) -> std::sync::MutexGuard<'a, RenderStateData> {
        if self.readonly.get() && matches!(msg, IoMessage::Write(_)) {
            return guard;
        }
        let guard = self
            .io_mailbox
            .send_locked(msg, &self.renderer_state, guard);
        self.io_mailbox.notify();
        guard
    }

    /// This queues a render operation with the renderer thread. The render
    /// isn't guaranteed to happen immediately but it will happen as soon as
    /// practical.
    pub(crate) fn queue_render(&self) {
        self.renderer_handle.wakeup();
    }

    #[allow(dead_code)]
    /// Push a message to the renderer thread (blocking if full) and wake it.
    pub(crate) fn send_renderer(&self, msg: RendererMessage) {
        self.renderer_handle.send(msg);
    }

    /// Tell the renderer which display the surface is on (the embedded
    /// `ghostty_surface_set_display_id`).
    #[cfg(target_vendor = "apple")]
    pub fn set_macos_display_id(&self, id: u32) {
        self.send_renderer(RendererMessage::MacosDisplayId(id));
    }

    /// A retained `CTFontRef` for the primary font, for macOS quicklook
    /// (the embedded `ghostty_surface_quicklook_font`). `scale` is the
    /// content scale. The caller owns the reference.
    #[cfg(target_vendor = "apple")]
    #[must_use]
    pub fn quicklook_font(&self, scale: f32) -> Option<*mut std::ffi::c_void> {
        let grid = self.renderer.font_grid();
        let mut state = grid.write();
        let face = state
            .resolver
            .collection
            .get_face(ghostty_font::collection::Index::default())
            .ok()?;
        let font = face.quicklook_font(f64::from(scale));
        Some(
            objc2_core_foundation::CFRetained::into_raw(font)
                .as_ptr()
                .cast(),
        )
    }

    /// Current sizes.
    #[must_use]
    pub fn size(&self) -> Size {
        self.size.get()
    }

    /// Convert surface-relative pixel coordinates to a viewport cell
    /// (Zig: `posToViewport`).
    #[must_use]
    pub fn pos_to_viewport(&self, xpos: f64, ypos: f64) -> PointCoordinate {
        // Get our grid cell
        let coord = Coordinate::Surface(ghostty_renderer::size::SurfaceCoord { x: xpos, y: ypos });
        match coord.convert(ghostty_renderer::size::CoordinateTag::Grid, self.size.get()) {
            Coordinate::Grid(g) => PointCoordinate {
                x: g.x,
                y: u32::from(g.y),
            },
            _ => PointCoordinate { x: 0, y: 0 },
        }
    }

    /// Scroll to the bottom of the viewport. The caller holds the terminal
    /// lock.
    pub(crate) fn scroll_to_bottom(&self, t: &mut ghostty_terminal::terminal::Terminal) {
        t.scroll_viewport(ghostty_terminal::terminal::ScrollViewport::Bottom);
        self.queue_render();
    }

    pub(crate) fn hide_mouse(&self) {
        {
            let mut m = self.mouse.borrow_mut();
            if m.hidden {
                return;
            }
            m.hidden = true;
        }
        self.perform(RtAction::MouseVisibility(MouseVisibility::Hidden));
    }

    pub(crate) fn show_mouse(&self) {
        {
            let mut m = self.mouse.borrow_mut();
            if !m.hidden {
                return;
            }
            m.hidden = false;
        }
        self.perform(RtAction::MouseVisibility(MouseVisibility::Visible));
    }

    // ------------------------------------------------------------------
    // Lifecycle

    /// Close this surface. This will trigger the runtime to start the close
    /// process, which should ultimately deinitialize this surface.
    pub fn close(&self) {
        self.rt_surface.close(self.needs_confirm_quit());
    }

    /// True if the surface requires confirmation to quit. This should be
    /// called by apprt to determine if the surface should confirm before
    /// quitting.
    #[must_use]
    pub fn needs_confirm_quit(&self) -> bool {
        // If the surface is in read-only mode, always require confirmation
        if self.readonly.get() {
            return true;
        }

        // If the child has exited, then our process is certainly not alive.
        // We check this first to avoid the locking overhead below.
        if self.child_exited.get() {
            return false;
        }

        // Check the configuration for confirming close behavior.
        match self.cfg().confirm_close_surface {
            ConfirmCloseSurface::Always => true,
            ConfirmCloseSurface::False => false,
            ConfirmCloseSurface::True => !self.lock_terminal().cursor_is_at_prompt(),
        }
    }

    /// Forces the surface to render (Zig: `draw`). Renderers are required
    /// to support `draw_frame` being called from the main thread.
    pub fn draw(&self) -> Result<(), CoreError> {
        self.renderer.draw_frame(true)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Messages (called from the app thread)

    /// Called from the app thread to handle mailbox messages to our specific
    /// surface.
    pub fn handle_message(&self, msg: Message) -> Result<(), CoreError> {
        match msg {
            Message::ChangeConfig(config) => self.update_config(&config)?,

            Message::SetTitle(buf) => {
                // We ignore the message in case the title was set via config.
                if self.cfg().title.is_some() {
                    log::debug!(
                        "ignoring title change request since static title is set via config"
                    );
                    return Ok(());
                }

                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                let title = String::from_utf8_lossy(&buf[..end]).into_owned();
                log::debug!("changing title \"{title}\"");
                self.perform(RtAction::SetTitle(SetTitle { title }));
            }

            Message::ReportTitle(style) => {
                if !self.cfg().title_report {
                    log::info!("report_title requested, but disabled via config");
                    return Ok(());
                }

                let title = self.rt_surface.get_title();
                let data = match style {
                    ghostty_apprt::surface::ReportTitleStyle::Csi21T => {
                        format!("\x1b]l{}\x1b\\", title.unwrap_or_default())
                    }
                };
                self.queue_io(IoMessage::write_req(data.as_bytes()));
            }

            Message::ColorChange(change) => {
                // Notify our apprt, but don't send a mode 2031 DSR report
                // because VT sequences were used to change the color.
                use ghostty_vt::{color::Dynamic as D, osc::color::Target};
                let kind = match change.target {
                    Target::Palette(v) => ColorKind::Palette(i32::from(v)),
                    Target::Dynamic(dyn_) => match dyn_ {
                        D::Foreground => ColorKind::Foreground,
                        D::Background => ColorKind::Background,
                        D::Cursor => ColorKind::Cursor,
                        // Unsupported dynamic color change notification type
                        _ => return Ok(()),
                    },
                    // Special colors aren't supported for change notification
                    Target::Special(_) => return Ok(()),
                };
                self.perform(RtAction::ColorChange(ColorChange {
                    kind,
                    r: change.color.r,
                    g: change.color.g,
                    b: change.color.b,
                }));
            }

            Message::SetMouseShape(shape) => {
                log::debug!("changing mouse shape: {shape:?}");
                self.perform(RtAction::MouseShape(shape));
            }

            Message::ClipboardRead(clipboard) => {
                if self.cfg().clipboard_read == ghostty_config::types_b::ClipboardAccess::Deny {
                    log::info!(
                        "application attempted to read clipboard, but 'clipboard-read' is set to deny"
                    );
                    return Ok(());
                }

                self.start_clipboard_request(
                    ghostty_apprt::structs::Clipboard::Standard,
                    ClipboardRequest::Osc52Read(clipboard),
                );
            }

            Message::ClipboardWrite {
                clipboard_type,
                req,
            } => {
                self.clipboard_write(req.slice(), clipboard_type);
            }

            Message::PwdChange(w) => {
                // Keep the OSC 7 bytes (see `Pwd`).
                let pwd = ghostty_os::osbytes::EncodedOs::from_bytes(w.slice());
                self.perform(RtAction::Pwd(Pwd { pwd }));
            }

            Message::Close => self.close(),

            Message::ChildExited(info) => self.child_exited(info),

            Message::DesktopNotification(notification) => {
                if !self.cfg().desktop_notifications {
                    log::info!(
                        "application attempted to display a desktop notification, but 'desktop-notifications' is disabled"
                    );
                    return Ok(());
                }

                let title = String::from_utf8_lossy(notification.title()).into_owned();
                let body = String::from_utf8_lossy(notification.body()).into_owned();
                self.show_desktop_notification(&title, &body);
            }

            Message::RendererHealth(health) => self.update_renderer_health(health),

            Message::Scrollbar(scrollbar) => self.update_scrollbar(scrollbar),

            Message::PresentSurface => {
                self.perform(RtAction::PresentTerminal);
            }

            Message::PasswordInput(v) => self.password_input(v),

            Message::RingBell => {
                let now = Instant::now();
                if let Some(last) = self.last_bell_time.get()
                    && now.duration_since(last) < StdDuration::from_millis(100)
                {
                    return Ok(());
                }
                self.last_bell_time.set(Some(now));
                self.perform(RtAction::RingBell);
            }

            Message::ProgressReport(v) => {
                self.perform(RtAction::ProgressReport(v));
            }

            Message::SelectionScrollTick(active) => {
                self.selection_scroll_active.set(active);
                self.selection_scroll_tick()?;
            }

            Message::StartCommand => {
                self.command_timer.set(Some(Instant::now()));
            }

            Message::StopCommand(code) => {
                let end = Instant::now();
                let Some(start) = self.command_timer.take() else {
                    return Ok(());
                };

                let duration = ghostty_config::types_b::Duration {
                    duration: end.duration_since(start).as_nanos() as u64,
                };
                log::debug!("command took {duration:?}");

                self.perform(RtAction::CommandFinished(CommandFinished {
                    exit_code: code,
                    duration,
                }));
            }

            Message::SearchTotal(v) => {
                self.perform(RtAction::SearchTotal(ghostty_apprt::action::SearchTotal {
                    total: v,
                }));
            }

            Message::SearchSelected(v) => {
                self.perform(RtAction::SearchSelected(
                    ghostty_apprt::action::SearchSelected { selected: v },
                ));
            }
        }
        Ok(())
    }

    /// The child process exited.
    fn child_exited(&self, info: ghostty_apprt::surface::ChildExited) {
        // Mark our flag that we exited immediately
        self.child_exited.set(true);

        // If our runtime was below some threshold then we assume that this
        // was an abnormal exit and we show an error message.
        if info.runtime_ms <= u64::from(self.cfg().abnormal_command_exit_runtime_ms) {
            // On macOS, our exit code detection doesn't work, possibly
            // because of our `login` wrapper. More investigation required.
            // Elsewhere, if the exit code is 0 then it was a good exit.
            if is_abnormal_exit(info.exit_code, cfg!(target_vendor = "apple")) {
                log::warn!("abnormal process exit detected, showing error message");

                // Try and show a GUI message. If it returns true, don't do
                // anything else.
                if self.perform(RtAction::ShowChildExited(info)) {
                    return;
                }

                // If a native GUI notification was not shown, update our
                // terminal to note the abnormal exit.
                if let Err(err) = self.child_exited_abnormally(info) {
                    log::error!("error handling abnormal child exit err={err}");
                }

                return;
            }
        }

        // We output a message so that the user knows what's going on and
        // doesn't think their terminal just froze. We show this
        // unconditionally on close even if `wait_after_command` is false and
        // the surface closes immediately because if a user does an `undo` to
        // restore a closed surface then they will see this message and know
        // the process has completed.
        //
        // First try and show a native GUI message. If the native GUI can't
        // be shown, display a text message in the terminal.
        if !self.perform(RtAction::ShowChildExited(info)) {
            let mut t = self.lock_terminal();
            t.carriage_return();
            let ok = t.linefeed().is_ok()
                && t.print_string("Process exited. Press any key to close the terminal.")
                    .is_ok();
            if ok {
                t.modes.set(ghostty_vt::modes::Mode::CursorVisible, false);

                // We also want to ensure that normal keyboard encoding is on
                // so that we can close the terminal. We close the terminal
                // on any key press that encodes a character.
                t.modes.set(ghostty_vt::modes::Mode::DisableKeyboard, false);
                t.screen_mut().kitty_keyboard.set(
                    ghostty_vt::kitty::key::SetMode::Set,
                    ghostty_vt::kitty::key::Flags::DISABLED,
                );
            }
        }

        // Waiting after command we stop here. The terminal is updated, our
        // state is updated, and now its up to the user to decide what to do.
        if self.cfg().wait_after_command {
            return;
        }

        // If we aren't waiting after the command, then we exit immediately
        // with no confirmation.
        self.close();
    }

    /// Called when the child process exited abnormally.
    fn child_exited_abnormally(
        &self,
        info: ghostty_apprt::surface::ChildExited,
    ) -> Result<(), ghostty_terminal::page::Error> {
        use ghostty_vt::sgr::Attribute;

        // Build up our command for the error message
        let command = self.command_args.join(" ");
        let runtime_str = format!("{} ms", info.runtime_ms);

        let mut t = self.lock_terminal();

        // No matter what move the cursor back to the column 0.
        t.carriage_return();

        // Reset styles
        t.set_attribute(Attribute::Unset)?;

        // If there is data in the viewport, we want to scroll down a little
        // bit and write a horizontal rule before writing our message. This
        // lets the use see the error message the command may have output.
        let viewport_str = t.plain_string()?;
        if !viewport_str.is_empty() {
            t.linefeed()?;
            for _ in 0..t.cols {
                t.print(0x2501)?;
            }
            t.carriage_return();
            t.linefeed()?;
            t.linefeed()?;
        }

        // Output our error message
        t.set_attribute(Attribute::Fg8(ghostty_vt::color::Name::BRIGHT_RED))?;
        t.set_attribute(Attribute::Bold)?;
        t.print_string("Ghostty failed to launch the requested command:")?;
        t.set_attribute(Attribute::Unset)?;

        t.carriage_return();
        t.linefeed()?;
        t.linefeed()?;
        t.print_string(&command)?;
        t.set_attribute(Attribute::Unset)?;

        t.carriage_return();
        t.linefeed()?;
        t.linefeed()?;
        t.print_string("Runtime: ")?;
        t.set_attribute(Attribute::Fg8(ghostty_vt::color::Name::RED))?;
        t.print_string(&runtime_str)?;
        t.set_attribute(Attribute::Unset)?;

        // We don't print this on macOS because the exit code is always 0
        // due to the way we launch the process.
        if !cfg!(target_vendor = "apple") {
            let exit_code_str = info.exit_code.to_string();
            t.carriage_return();
            t.linefeed()?;
            t.print_string("Exit Code: ")?;
            t.set_attribute(Attribute::Fg8(ghostty_vt::color::Name::RED))?;
            t.print_string(&exit_code_str)?;
            t.set_attribute(Attribute::Unset)?;
        }

        t.carriage_return();
        t.linefeed()?;
        t.linefeed()?;
        t.print_string("Press any key to close the window.")?;

        // Hide the cursor
        t.modes.set(ghostty_vt::modes::Mode::CursorVisible, false);
        Ok(())
    }

    /// Called when the terminal detects there is a password input prompt.
    fn password_input(&self, v: bool) {
        {
            let mut t = self.lock_terminal();

            // If our password input state is unchanged then we don't waste
            // time doing anything more.
            if t.flags.password_input == v {
                return;
            }

            t.flags.password_input = v;
        }

        // Notify our apprt so it can do whatever it wants.
        self.perform(RtAction::SecureInput(if v {
            SecureInput::On
        } else {
            SecureInput::Off
        }));

        self.queue_render();
    }

    /// Called when our renderer health state changes.
    fn update_renderer_health(&self, health: Health) {
        log::warn!("renderer health status change status={health:?}");
        self.perform(RtAction::RendererHealth(health));
    }

    /// Called when the scrollbar state changes.
    fn update_scrollbar(&self, scrollbar: Scrollbar) {
        self.perform(RtAction::Scrollbar(scrollbar));
    }

    /// This should be called anytime `config_conditional_state` changes so
    /// that the apprt can reload the configuration.
    fn notify_config_conditional_state(&self) {
        self.perform(RtAction::ReloadConfig(ReloadConfig { soft: true }));
    }

    // ------------------------------------------------------------------
    // Configuration

    /// Update our configuration at runtime. This can be called by the apprt
    /// to set a surface-specific configuration that differs from the app or
    /// other surfaces.
    pub fn update_config(&self, original: &Arc<Config>) -> Result<(), CoreError> {
        // Apply our conditional state. If we fail to apply the conditional
        // state then we log and attempt to move forward with the old config.
        let config: Arc<Config> =
            match original.change_conditional_state(self.config_conditional_state.get()) {
                Ok(Some(c)) => Arc::new(c),
                Ok(None) => Arc::clone(original),
                Err(err) => {
                    log::warn!("failed to apply conditional state to config err={err}");
                    Arc::clone(original)
                }
            };

        // Update our new derived config immediately
        let derived = match DerivedConfig::new(Arc::clone(&config)) {
            Ok(d) => Rc::new(d),
            Err(err) => {
                // If the derivation fails then we just log and return. We
                // don't hard fail in this case because we don't want to
                // error the surface when config fails we just want to keep
                // using the old config.
                log::error!("error updating configuration err={err}");
                return Ok(());
            }
        };
        *self.config.borrow_mut() = Rc::clone(&derived);

        // If our mouse is hidden but we disabled mouse hiding, then show it
        // again.
        if !derived.mouse_hide_while_typing && self.mouse.borrow().hidden {
            self.show_mouse();
        }

        // If we are in the middle of a key sequence, clear it.
        self.end_key_sequence(keyboard::SequenceAction::Drop, keyboard::SequenceFree::Free);

        // Deactivate all key tables since they may have changed. Importantly,
        // we store references into the config as part of our table stack so
        // we can't keep them active across config changes. But this behavior
        // also matches key sequences.
        self.deactivate_all_key_tables();

        // Before sending any other config changes, we give the renderer a
        // new font grid. We could check to see if there was an actual change
        // to the font, but this is easier and pretty rare so it's not a
        // performance concern.
        //
        // (Calling set_font_size builds and sends a new font grid to the
        // renderer.)
        let font_size = {
            let f = self.font.borrow();
            // If we have manually adjusted the font size, keep it that way.
            if f.adjusted {
                log::info!(
                    "font size manually adjusted, preserving previous size on config reload"
                );
                f.size
            } else {
                // If we haven't, then we update to the configured font size.
                // This allows config changes to update the font size.
                let mut size = f.size;
                size.points = config.font_size.clamp(1.0, 255.0);
                size
            }
        };
        self.set_font_size(font_size)?;

        // We need to store our configs in a heap-allocated pointer so that
        // our messages aren't huge.
        let renderer_message = RendererMessage::init_change_config(&config)?;
        self.renderer_handle.send(renderer_message);
        self.queue_io(IoMessage::ChangeConfig(Box::new(IoDerivedConfig::new(
            &config,
        ))));

        // With mailbox messages sent, we have to wake them up so they
        // process it.
        self.queue_render();

        // If we have a title set then we update our window to have the newly
        // configured title.
        if let Some(title) = &config.title {
            self.perform(RtAction::SetTitle(SetTitle {
                title: ghostty_os::osbytes::config_str_to_text(title),
            }));
        }

        // Notify the window
        self.perform(RtAction::ConfigChange(ConfigChange { config }));
        Ok(())
    }

    /// Recalculate the initial size of the window based on the
    /// configuration and invoke the apprt `initial_size` action if
    /// necessary.
    fn recompute_initial_size(&self) {
        let cfg = self.cfg();

        // Both width and height must be set for this to work, as documented
        // on the config options.
        if cfg.window_height == 0 || cfg.window_width == 0 {
            return;
        }

        let scale = self.rt_surface.get_content_scale();
        let size = self.size.get();
        let height = cfg.window_height.max(MIN_WINDOW_HEIGHT_CELLS) * size.cell.height;
        let width = cfg.window_width.max(MIN_WINDOW_WIDTH_CELLS) * size.cell.width;
        let width_f32 = width as f32;
        let height_f32 = height as f32;

        // The final values are affected by content scale and we need to
        // account for the padding so we get the exact correct grid size.
        let final_width =
            (width_f32 / scale.x).ceil() as u32 + size.padding.left + size.padding.right;
        let final_height =
            (height_f32 / scale.y).ceil() as u32 + size.padding.top + size.padding.bottom;

        self.perform(RtAction::InitialSize(InitialSize {
            width: final_width,
            height: final_height,
        }));
    }

    // ------------------------------------------------------------------
    // Text access

    /// Returns true if the terminal has a selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.lock_terminal().screen().selection.is_some()
    }

    /// Returns the selected text.
    #[must_use]
    pub fn selection_string(&self) -> Option<String> {
        let t = self.lock_terminal();
        let sel = t.screen().selection?;
        Some(
            t.screen()
                .selection_string(ghostty_terminal::screen::SelectionString { sel, trim: false }),
        )
    }

    /// Returns the pwd of the terminal, if any. This is always copied
    /// because the pwd can change at any point from termio.
    #[must_use]
    pub fn pwd(&self) -> Option<ghostty_os::osbytes::EncodedOs> {
        let t = self.lock_terminal();
        let pwd = t.get_pwd()?;
        Some(ghostty_os::osbytes::EncodedOs::from_bytes(pwd))
    }

    /// The raw bytes of [`Surface::pwd`] (Zig returns the OSC 7 bytes as is).
    #[must_use]
    pub fn pwd_bytes(&self) -> Option<Vec<u8>> {
        let t = self.lock_terminal();
        t.get_pwd().map(<[u8]>::to_vec)
    }

    /// Returns the x/y coordinate of where the IME (Input Method Editor)
    /// keyboard should be rendered.
    #[must_use]
    pub fn ime_point(&self) -> ImePos {
        let (cursor_x, cursor_y, preedit_width) = {
            let st = self.lock_state();
            let cursor = &st.terminal.screen().cursor;
            (
                u32::from(cursor.x),
                u32::from(cursor.y),
                st.preedit.as_ref().map_or(0, Preedit::width) as u32,
            )
        };

        // TODO: need to handle when scrolling and the cursor is not in the
        // visible portion of the screen.

        // Our sizes are all scaled so we need to send the unscaled values
        // back.
        let content_scale = self.rt_surface.get_content_scale();
        let size = self.size.get();

        // Simple x * cell width gives the top-left corner, then add padding
        // offset
        let mut x = f64::from(cursor_x * size.cell.width + size.padding.left);
        // We want the midpoint
        x += f64::from(size.cell.width) / 2.0;
        // And scale it
        x /= f64::from(content_scale.x);

        // Simple y * cell height gives the top-left corner, then add padding
        // offset
        let mut y = f64::from(cursor_y * size.cell.height + size.padding.top);
        // We want the bottom
        y += f64::from(size.cell.height);
        // And scale it
        y /= f64::from(content_scale.y);

        // Our height for now is always just the cell height because our
        // preedit rendering only renders in a single line.
        let height = f64::from(size.cell.height) / f64::from(content_scale.y);

        let mut width = f64::from(preedit_width * size.cell.width);

        // Our max width is the remaining screen width after the cursor. We
        // don't have to deal with wrapping because the preedit doesn't wrap
        // right now.
        let screen_width = f64::from(size.terminal().width);
        let x_offset = f64::from((cursor_x + 1) * size.cell.width);
        let max = screen_width - x_offset;
        width = width.min(max);

        ImePos {
            x,
            y,
            width,
            height,
        }
    }

    // ------------------------------------------------------------------
    // Selection

    /// Set the selection contents. This must be called with the renderer
    /// mutex held (pass the locked terminal). Copy-on-select clipboard
    /// writes happen here too.
    pub(crate) fn set_selection(
        &self,
        t: &mut ghostty_terminal::terminal::Terminal,
        sel: Option<Selection>,
    ) -> Result<(), ghostty_terminal::page::Error> {
        // Compare against the previous selection BEFORE replacing it:
        // `select` releases the previous selection's tracked pins, after
        // which they can no longer be resolved (Zig compares through raw
        // pointers that happen to stay readable).
        let unchanged = match (sel, t.screen().selection) {
            (Some(sel), Some(prev)) => sel.eql(&t.screen().pages, &prev),
            _ => false,
        };
        t.screen_mut().select(sel)?;

        let cfg = self.cfg();

        // If copy on select is false then exit early.
        use ghostty_config::types_b::CopyOnSelect;
        if cfg.copy_on_select == CopyOnSelect::False {
            return Ok(());
        }

        // Set our selection clipboard. If the selection is cleared we do not
        // clear the clipboard. If the selection is set, we only set the
        // clipboard again if it changed, since setting the clipboard can be
        // an expensive operation.
        let Some(sel) = sel else {
            return Ok(());
        };
        if unchanged {
            return Ok(());
        }

        use ghostty_apprt::structs::Clipboard;
        match cfg.copy_on_select {
            CopyOnSelect::False => unreachable!("handled above with an early exit"),

            // Both standard and selection clipboards are set.
            CopyOnSelect::Clipboard => self.copy_selection_to_clipboards(
                t,
                sel,
                &[Clipboard::Standard, Clipboard::Selection],
                ghostty_input::binding::CopyToClipboard::Mixed,
            ),

            // The selection clipboard is set if supported, otherwise the
            // standard.
            CopyOnSelect::True => {
                let clipboard = if self.rt_surface.supports_clipboard(Clipboard::Selection) {
                    Clipboard::Selection
                } else {
                    Clipboard::Standard
                };
                self.copy_selection_to_clipboards(
                    t,
                    sel,
                    &[clipboard],
                    ghostty_input::binding::CopyToClipboard::Mixed,
                );
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Fonts and sizes

    /// Publish the current size for crash reports (Zig reads the surface
    /// size at crash time).
    fn sync_crash_dims(&self) {
        let s = self.size.get();
        let g = s.grid();
        self.crash_dims.set(ghostty_crash::DimsSnapshot {
            screen_width: s.screen.width,
            screen_height: s.screen.height,
            grid_columns: u32::from(g.columns),
            grid_rows: u32::from(g.rows),
            cell_width: s.cell.width,
            cell_height: s.cell.height,
        });
    }

    /// Crash context for the main thread while inside a surface callback
    /// (Zig: `crashThreadState`).
    pub(crate) fn crash_guard(&self) -> ghostty_crash::ThreadGuard {
        self.sync_crash_dims();
        ghostty_crash::ThreadGuard::new(
            ghostty_crash::ThreadType::Main,
            Some(Arc::clone(&self.crash_dims)),
        )
    }

    /// Change the cell size for the terminal grid. This can happen as a
    /// result of changing the font size at runtime.
    fn set_cell_size(&self, size: CellSize) {
        // Update our cell size within our size struct
        let mut s = self.size.get();
        s.cell = size;
        self.size.set(s);
        self.balance_padding_if_needed();
        self.sync_crash_dims();

        // Notify the terminal
        self.queue_io(IoMessage::Resize(self.size.get()));

        // Update our terminal default size if necessary.
        self.recompute_initial_size();

        // Notify the window
        self.perform(RtAction::CellSize(ActionCellSize {
            width: size.width,
            height: size.height,
        }));
    }

    /// Change the font size. This can only be called from the main thread.
    pub fn set_font_size(&self, size: DesiredSize) -> Result<(), CoreError> {
        log::debug!("set font size size={}", size.points);

        // Update our font size so future changes work
        self.font.borrow_mut().size = size;

        // We need to build up a new font stack for this font size.
        let derived = FontDerivedConfig::new(&self.cfg().config);
        let (font_grid_key, font_grid) = self.app.font_grid_set.ref_(&derived, size)?;

        // Set our cell size
        let metrics = font_grid.metrics();
        self.set_cell_size(CellSize {
            width: metrics.cell_width,
            height: metrics.cell_height,
        });

        // Notify our render thread of the new font stack. The renderer MUST
        // accept the new font grid and deref the old.
        let old_key = {
            let mut f = self.font.borrow_mut();
            let old = std::mem::replace(&mut f.grid_key, font_grid_key.clone());
            f.metrics = metrics;
            old
        };
        self.renderer_handle
            .send(RendererMessage::FontGrid(Box::new(FontGridChange {
                grid: font_grid,
                set: Arc::clone(&self.app.font_grid_set),
                new_key: font_grid_key,
                old_key,
            })));

        // Schedule render which also drains our mailbox
        self.queue_render();
        Ok(())
    }

    /// The current font size (points + dpi) for runtime queries.
    #[must_use]
    pub fn font_size(&self) -> DesiredSize {
        self.font.borrow().size
    }

    /// Called when the runtime's surface size changes.
    pub fn size_callback(&self, size: SurfaceSize) -> Result<(), CoreError> {
        let new_screen_size = ScreenSize {
            width: size.width,
            height: size.height,
        };

        // Update our screen size, but only if it actually changed. And if
        // the screen size didn't change, then our grid size could not have
        // changed, so we just return.
        if self.size.get().screen.equals(new_screen_size) {
            return Ok(());
        }

        self.resize(new_screen_size);
        Ok(())
    }

    fn resize(&self, size: ScreenSize) {
        // Save our screen size
        let mut s = self.size.get();
        s.screen = size;
        self.size.set(s);
        self.balance_padding_if_needed();
        let s = self.size.get();

        // Recalculate our grid size. Because Ghostty supports fluid
        // resizing, its possible the grid doesn't change at all even if the
        // screen size changes. We have to update the IO thread no matter
        // what because we send pixel-level sizing to the subprocess.
        let grid_size = s.grid();
        if grid_size.columns < 5 && (s.padding.left > 0 || s.padding.right > 0) {
            log::warn!(
                "WARNING: very small terminal grid detected with padding set. Is your padding reasonable?"
            );
        }
        if grid_size.rows < 2 && (s.padding.top > 0 || s.padding.bottom > 0) {
            log::warn!(
                "WARNING: very small terminal grid detected with padding set. Is your padding reasonable?"
            );
        }

        self.sync_crash_dims();

        // Mail the IO thread
        self.queue_io(IoMessage::Resize(s));
    }

    /// Recalculate the balanced padding if needed.
    fn balance_padding_if_needed(&self) {
        let cfg = self.cfg();
        if !cfg.window_padding_balance {
            return;
        }
        let content_scale = self.rt_surface.get_content_scale();
        let x_dpi = content_scale.x * DEFAULT_DPI;
        let y_dpi = content_scale.y * DEFAULT_DPI;
        let mut s = self.size.get();
        s.balance_padding(cfg.scaled_padding(x_dpi, y_dpi));
        self.size.set(s);
    }

    /// Called to set the preedit state for character input. Preedit is used
    /// with dead key states, for example, when typing an accent character.
    /// This should be called with `None` to reset the preedit state.
    ///
    /// The core surface will NOT reset the preedit state on charCallback or
    /// keyCallback and we rely completely on the apprt implementation to
    /// track the preedit state correctly.
    pub fn preedit_callback(&self, preedit: Option<&str>) -> Result<(), CoreError> {
        let cfg = self.cfg();
        let mut st = self.lock_state();

        // We clear our selection when ANY OF:
        // 1. We have an existing preedit
        // 2. We have preedit text
        if (st.preedit.is_some() || preedit.is_some()) && cfg.selection_clear_on_typing {
            let _ = self.set_selection(&mut st.terminal, None);
        }

        // We always clear our prior preedit
        st.preedit = None;

        // Mark preedit dirty flag
        st.terminal.flags.dirty.preedit = true;

        // If we have no text, we're done. We queue a render in case we
        // cleared a prior preedit (likely).
        let Some(text) = preedit else {
            drop(st);
            self.queue_render();
            return Ok(());
        };

        // We convert the UTF-8 text to codepoints.
        let mut codepoints: Vec<PreeditCodepoint> = Vec::new();
        for cp in text.chars() {
            let width = usize::from(ghostty_unicode::table().get(u32::from(cp)).width);

            // I've never seen a preedit text with a zero-width character. In
            // theory its possible but we can't really handle it right now.
            // Let's just ignore it.
            if width == 0 {
                continue;
            }

            codepoints.push(PreeditCodepoint {
                codepoint: u32::from(cp),
                wide: width >= 2,
            });
        }

        if !codepoints.is_empty() {
            st.preedit = Some(Preedit { codepoints });
        }
        drop(st);
        self.queue_render();
        Ok(())
    }

    /// Callback for when the surface is fully visible or not, regardless of
    /// focus state.
    pub fn occlusion_callback(&self, visible: bool) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        self.renderer_handle.send(RendererMessage::Visible(visible));
        self.queue_render();
        Ok(())
    }

    /// Called when the surface gains or loses focus.
    pub fn focus_callback(&self, focused: bool) -> Result<(), CoreError> {
        // If our focus state is the same we do nothing.
        if self.focused.get() == focused {
            return Ok(());
        }
        self.focused.set(focused);

        // Notify our render thread of the new state
        self.renderer_handle.send(RendererMessage::Focus(focused));

        if focused {
            // Notify our app if we gained focus.
            self.app.focus_surface(self.id);
        } else {
            // If we lost focus and we have a keypress, then we want to send
            // a key release event for it.
            self.release_pressed_key_on_unfocus();
        }

        // Schedule render which also drains our mailbox
        self.queue_render();

        // Whenever our focus changes we unhide the mouse. The mouse will be
        // hidden again if the user starts typing. This helps alleviate some
        // buggy behavior upstream in macOS with the mouse never becoming
        // visible again when tabbing between programs (see #2525).
        self.show_mouse();

        // Update the focus state and notify the terminal
        self.lock_terminal().flags.focused = focused;
        self.queue_io(IoMessage::Focused(focused));
        Ok(())
    }

    /// Schedule a render (Zig: `refreshCallback`).
    pub fn refresh_callback(&self) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        // The point of this callback is to schedule a render, so do that.
        self.queue_render();
        Ok(())
    }

    /// The content scale (DPI) of the surface changed.
    pub fn content_scale_callback(&self, content_scale: ContentScale) -> Result<(), CoreError> {
        // Calculate the new DPI
        let x_dpi = content_scale.x * DEFAULT_DPI;
        let y_dpi = content_scale.y * DEFAULT_DPI;

        // Update our font size which is dependent on the DPI
        let current = self.font.borrow().size;
        let mut size = current;
        size.xdpi = x_dpi as u16;
        size.ydpi = y_dpi as u16;

        // If our DPI didn't actually change, save a lot of work by doing
        // nothing.
        if size.xdpi == current.xdpi && size.ydpi == current.ydpi {
            return Ok(());
        }

        self.set_font_size(size)?;

        // Update our padding which is dependent on DPI. We only do this for
        // unbalanced padding since balanced padding is not dependent on DPI.
        let cfg = self.cfg();
        if !cfg.window_padding_balance {
            let mut s = self.size.get();
            s.padding = cfg.scaled_padding(x_dpi, y_dpi);
            self.size.set(s);
        }

        // Force a resize event because the change in padding will affect
        // pixel-level changes to the renderer and viewport.
        self.resize(self.size.get().screen);
        Ok(())
    }

    /// The system color scheme changed.
    pub fn color_scheme_callback(&self, scheme: ColorScheme) -> Result<(), CoreError> {
        let new_scheme = match scheme {
            ColorScheme::Light => conditional::Theme::Light,
            ColorScheme::Dark => conditional::Theme::Dark,
        };

        // If our scheme didn't change, then we don't do anything.
        let mut state = self.config_conditional_state.get();
        if state.theme == new_scheme {
            return Ok(());
        }

        // Setup our conditional state which has the current color theme.
        state.theme = new_scheme;
        self.config_conditional_state.set(state);
        self.notify_config_conditional_state();

        // If mode 2031 is on, then we report the change live.
        self.queue_io(IoMessage::ColorSchemeReport { force: false });
        Ok(())
    }

    /// Tell the surface to present itself to the user.
    pub fn present_surface(&self) {
        self.perform(RtAction::PresentTerminal);
    }

    // ------------------------------------------------------------------
    // Renderer display lifecycle (GTK GLArea realize/unrealize)

    /// Developer aid: if `GHOSTTY_DEBUG_SCREENSHOT=<path.png>` is set, write
    /// the last presented frame to that path once, `GHOSTTY_DEBUG_SCREENSHOT_MS`
    /// (default 2500) after the surface was created. Must be called with the
    /// GL context current (from `draw_frame`).
    fn debug_screenshot(&self) {
        let Some(path) = std::env::var_os("GHOSTTY_DEBUG_SCREENSHOT") else {
            return;
        };
        let delay = std::env::var("GHOSTTY_DEBUG_SCREENSHOT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2500);
        if self.debug_shot_done.get() || self.created.elapsed().as_millis() < u128::from(delay) {
            return;
        }
        if self
            .write_screenshot_png(std::path::Path::new(&path))
            .is_ok()
        {
            self.debug_shot_done.set(true);
        }
    }

    /// Write the last presented frame as a PNG. Requires the GL context to be
    /// current.
    #[cfg(target_vendor = "apple")]
    pub fn write_screenshot_png(&self, _path: &std::path::Path) -> Result<(), CoreError> {
        Err("screenshots are not supported by the Metal renderer".into())
    }

    /// Write the last presented frame as a PNG. Requires the GL context to be
    /// current.
    #[cfg(not(target_vendor = "apple"))]
    pub fn write_screenshot_png(&self, path: &std::path::Path) -> Result<(), CoreError> {
        let Some((w, h, px)) = self.renderer.last_frame_pixels() else {
            return Err("no frame has been presented yet".into());
        };
        // Bottom-up RGBA -> top-down.
        let stride = w * 4;
        let mut flipped = Vec::with_capacity(px.len());
        for row in px.chunks_exact(stride).rev() {
            flipped.extend_from_slice(row);
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&flipped)?;
        log::info!("wrote screenshot to {}", path.display());
        Ok(())
    }

    /// The pixels of the last presented frame (`width`, `height`, RGBA8,
    /// bottom-up). Test/debug hook for headless integration tests; requires
    /// the GL context to be current on the calling thread.
    #[cfg(not(target_vendor = "apple"))]
    pub fn frame_pixels(&self) -> Option<(usize, usize, Vec<u8>)> {
        self.renderer.last_frame_pixels()
    }

    /// The GLArea was (re)realized; the GL context is current.
    #[cfg(not(target_vendor = "apple"))]
    pub fn renderer_realized(&self) -> Result<(), CoreError> {
        let rt = Rc::clone(&self.rt_surface);
        self.renderer
            .display_realized(|name| rt.gl_proc_address(name))?;
        Ok(())
    }

    /// The display was (re)realized (Metal has nothing to reload).
    #[cfg(target_vendor = "apple")]
    pub fn renderer_realized(&self) -> Result<(), CoreError> {
        self.renderer.display_realized_with(|_| {})?;
        Ok(())
    }

    /// The GLArea is being unrealized; the GL context is current.
    pub fn renderer_unrealized(&self) {
        self.renderer.display_unrealized();
    }
}

impl Drop for Surface {
    /// Zig: `Surface.deinit`.
    fn drop(&mut self) {
        // Stop search thread
        if let Some(s) = self.search.get_mut().take() {
            s.deinit();
        }

        // Stop rendering thread
        if let Some(running) = self.renderer_thread.get_mut().take() {
            running.stop_and_join();
        }

        // Stop our IO thread
        if let Err(err) = self.io_stop.notify() {
            log::error!("error notifying io thread to stop, may stall err={err}");
        }
        if let Some(join) = self.io_thread.get_mut().take()
            && join.join().is_err()
        {
            log::warn!("io thread panicked");
        }

        // Clean up our font grid
        let key = self.font.get_mut().grid_key.clone();
        self.app.font_grid_set.deref(&key);

        log::info!("surface closed id={}", self.id);
    }
}

/// The environment for the child: exactly what the runtime provides
/// (`rt_surface.defaultTermioEnv()`, which already removed the variables it
/// does not want to leak); only if it provides nothing (the Zig error path)
/// do we fall back to the process environment (`getEnvMap`).
/// Whether a quick child exit counts as abnormal. On Darwin our exit code
/// detection doesn't work (the `login` wrapper), so every quick exit is
/// treated as abnormal; elsewhere only non-zero exit codes are.
fn is_abnormal_exit(exit_code: u32, darwin: bool) -> bool {
    darwin || exit_code != 0
}

fn rt_surface_env(rt: &dyn RtSurface) -> ghostty_os::EnvMap {
    let rt_env = rt.default_termio_env();
    if rt_env.is_empty() {
        return ghostty_os::env::get_env_map();
    }
    rt_env.into_iter().collect()
}

// ----------------------------------------------------------------------
// CoreSurface (what the runtime calls)

impl CoreSurface for Surface {
    fn config(&self) -> Arc<Config> {
        Arc::clone(&self.cfg().config)
    }

    fn key_callback(&self, event: ghostty_input::KeyEvent<'_>) -> Result<InputEffect, CoreError> {
        let _crash = self.crash_guard();
        Surface::key_callback(self, event)
    }

    fn preedit_callback(&self, text: Option<&str>) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::preedit_callback(self, text)
    }

    fn focus_callback(&self, focused: bool) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::focus_callback(self, focused)
    }

    fn mouse_button_callback(
        &self,
        state: MouseButtonState,
        button: MouseButton,
        mods: Mods,
    ) -> Result<bool, CoreError> {
        let _crash = self.crash_guard();
        Surface::mouse_button_callback(self, state, button, mods)
    }

    fn cursor_pos_callback(&self, pos: CursorPos, mods: Option<Mods>) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::cursor_pos_callback(self, pos, mods)
    }

    fn scroll_callback(&self, xoff: f64, yoff: f64, mods: ScrollMods) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::scroll_callback(self, xoff, yoff, mods)
    }

    fn mouse_pressure_callback(&self, stage: ghostty_input::MousePressureStage, pressure: f64) {
        let _crash = self.crash_guard();
        Surface::mouse_pressure_callback(self, stage, pressure);
    }

    fn size_callback(&self, size: SurfaceSize) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::size_callback(self, size)
    }

    fn content_scale_callback(&self, scale: ContentScale) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::content_scale_callback(self, scale)
    }

    fn ime_point(&self) -> ImePos {
        Surface::ime_point(self)
    }

    fn has_selection(&self) -> bool {
        Surface::has_selection(self)
    }

    fn color_scheme_callback(&self, scheme: ColorScheme) -> Result<(), CoreError> {
        let _crash = self.crash_guard();
        Surface::color_scheme_callback(self, scheme)
    }

    fn update_config(&self, config: &Arc<Config>) -> Result<(), CoreError> {
        Surface::update_config(self, config)
    }

    fn perform_binding_action(
        &self,
        action: &ghostty_input::binding::Action,
    ) -> Result<bool, CoreError> {
        Surface::perform_binding_action(self, action)
    }

    fn complete_clipboard_request(
        &self,
        req: ClipboardRequest,
        data: &[u8],
        confirmed: bool,
    ) -> Result<(), ClipboardError> {
        Surface::complete_clipboard_request(self, req, data, confirmed)
    }

    fn complete_paste_bytes(&self, data: &[u8], confirmed: bool) -> Result<(), ClipboardError> {
        Surface::complete_paste_bytes(self, data, confirmed)
    }

    fn set_clipboard_read_allowed(&self, allow: bool) {
        use ghostty_config::types_b::ClipboardAccess;
        self.set_config_clipboard(
            true,
            if allow {
                ClipboardAccess::Allow
            } else {
                ClipboardAccess::Deny
            },
        );
    }

    fn set_clipboard_write_allowed(&self, allow: bool) {
        use ghostty_config::types_b::ClipboardAccess;
        self.set_config_clipboard(
            false,
            if allow {
                ClipboardAccess::Allow
            } else {
                ClipboardAccess::Deny
            },
        );
    }

    fn needs_confirm_quit(&self) -> bool {
        Surface::needs_confirm_quit(self)
    }

    fn child_exited(&self) -> bool {
        self.child_exited.get()
    }

    fn renderer_realized(&self) -> Result<(), CoreError> {
        Surface::renderer_realized(self)
    }

    fn renderer_unrealized(&self) {
        Surface::renderer_unrealized(self);
    }

    fn draw_frame(&self, _default_framebuffer: u32) -> Result<(), CoreError> {
        // GTK's GLArea binds its own FBO before `render`; the renderer
        // queries GL_FRAMEBUFFER_BINDING itself.
        self.renderer.draw_frame(false)?;
        self.debug_screenshot();
        Ok(())
    }

    fn write_screenshot(&self, path: &str) -> Result<(), CoreError> {
        self.write_screenshot_png(std::path::Path::new(path))
    }

    fn id(&self) -> Option<SurfaceId> {
        Some(self.id)
    }

    fn readonly(&self) -> bool {
        self.readonly.get()
    }

    fn grid_size(&self) -> (u32, u32) {
        let g = self.size.get().grid();
        (u32::from(g.columns), u32::from(g.rows))
    }

    fn font_size(&self) -> DesiredFontSize {
        let s = self.font.borrow().size;
        DesiredFontSize {
            points: s.points,
            xdpi: s.xdpi,
            ydpi: s.ydpi,
        }
    }

    fn activate_inspector(&self) -> Result<(), CoreError> {
        Surface::activate_inspector(self)
    }

    fn deactivate_inspector(&self) {
        Surface::deactivate_inspector(self);
    }

    fn render_inspector(&self, ui: &ghostty_inspector::imgui::Ui) {
        Surface::render_inspector(self, ui);
    }
}

#[cfg(test)]
mod darwin_tests {
    use super::*;

    #[test]
    fn abnormal_exit_detection_per_platform() {
        assert!(!is_abnormal_exit(0, false));
        assert!(is_abnormal_exit(1, false));
        assert!(is_abnormal_exit(0, true));
        assert!(is_abnormal_exit(1, true));
    }
}
