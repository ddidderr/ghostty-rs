//! Primary terminal IO ("termio") state. This maintains the terminal state,
//! pty, subprocess, etc. This is flexible enough to be used in environments
//! that don't have a pty and simply provides the input/output using raw
//! bytes (`termio/Termio.zig`).
//!
//! # Ownership
//!
//! Zig has one `Termio` struct touched by the IO thread (messages), the pty
//! read thread (`processOutput`) and the surface thread (via the mailbox). In
//! Rust it is split along the thread boundaries:
//!
//! * [`Termio`] is owned by the IO thread (`Thread::thread_main` takes it by
//!   value) and holds the backend, derived config and cached size.
//! * [`Output`] (an `Arc`, reachable through [`Termio::output`]) is the part
//!   the pty read thread needs: the shared [`SharedState`], the VT stream and
//!   the renderer/termio notification handles. `Termio::process_output` is
//!   just `output.process_output`.
//! * The `Terminal` itself lives in [`SharedState`]
//!   (`Arc<Mutex<Terminal>>`), shared with the renderer like Zig's
//!   `renderer.State`.
//!
//! Lock order is always terminal mutex first, then the stream mutex inside
//! [`Output`].

use std::io::Read;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ghostty_config::conditional;
use ghostty_config::config::Config;
use ghostty_config::io::ReadableIO;
use ghostty_config::types_a::{Color, CursorStyle, TerminalColor};
use ghostty_config::types_b::{ClipboardAccess, GraphemeWidthMethod, OSCColorReportFormat};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_renderer::message::Message as RendererMessage;
use ghostty_renderer::size::Size;
use ghostty_renderer::state::InspectorHook;
use ghostty_terminal::kitty::graphics_storage::TerminalInfo;
use ghostty_terminal::point::{Coordinate, Point};
use ghostty_terminal::screen::Scroll;
use ghostty_terminal::screen_set::Key as ScreenKey;
use ghostty_terminal::terminal::{Colors, Options as TerminalOptions, ScrollViewport, Terminal};
use ghostty_vt::color::{DynamicPalette, DynamicRgb, Palette};
use ghostty_vt::csi;
use ghostty_vt::kitty::graphics_command::Delete as KittyDelete;
use ghostty_vt::modes::{Mode, ModePacked};
use std::sync::MutexGuard;

use crate::backend::{self, Backend};
use crate::exec;
use crate::mailbox::Mailbox;
use crate::message::{Message, SizeReport};
use crate::options::Options;
use crate::sinks::{RendererMailbox, SurfaceMailbox};
use crate::state::{SharedState, SharedStateExt, State};
use crate::stream_handler::{HandlerOptions, HandlerState, TerminalStream};
use crate::wakeup::Wakeup;

/// Errors from termio initialization and thread setup.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Exec(#[from] exec::Error),
    #[error("terminal error: {0:?}")]
    Terminal(ghostty_terminal::terminal::Error),
    #[error("a configured `input` path was not found")]
    InputNotFound,
    #[error("a configured `input` could not be read or written")]
    InputFailed,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<ghostty_terminal::terminal::Error> for Error {
    fn from(e: ghostty_terminal::terminal::Error) -> Self {
        Error::Terminal(e)
    }
}

/// Mutex state argument for [`Termio::queue_message`] callers that hold the
/// terminal lock (`MutexState`). In Rust the lock is a guard, so this is
/// expressed with `Mailbox::send_locked` instead; the enum is kept for API
/// parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexState {
    Locked,
    Unlocked,
}

/// The configuration for this IO that is derived from the main
/// configuration. This must be exported so that we don't need to
/// pass around Config pointers which makes memory management a pain.
#[derive(Debug, Clone)]
pub struct DerivedConfig {
    pub palette: Palette,
    pub image_storage_limit: usize,
    pub cursor_style: CursorStyle,
    pub cursor_blink: Option<bool>,
    pub cursor_color: Option<TerminalColor>,
    pub foreground: Color,
    pub background: Color,
    pub osc_color_report_format: OSCColorReportFormat,
    pub clipboard_write: ClipboardAccess,
    pub enquiry_response: String,
    pub conditional_state: conditional::State,
}

impl DerivedConfig {
    pub fn new(config: &Config) -> DerivedConfig {
        let palette: Palette = 'palette: {
            if config.palette_generate && config.palette.mask.count() > 0 {
                // If the user didn't set any values manually, then
                // we're using the default palette and we don't need
                // to apply the generation code to it. (Handled by the
                // count check above.)
                break 'palette ghostty_vt::color::generate_256_color(
                    config.palette.value,
                    config.palette.mask,
                    config.background.to_terminal_rgb(),
                    config.foreground.to_terminal_rgb(),
                    config.palette_harmonious,
                );
            }

            config.palette.value
        };

        DerivedConfig {
            palette,
            image_storage_limit: config.image_storage_limit as usize,
            cursor_style: config.cursor_style,
            cursor_blink: config.cursor_style_blink,
            cursor_color: config.cursor_color,
            foreground: config.foreground,
            background: config.background,
            osc_color_report_format: config.osc_color_report_format,
            clipboard_write: config.clipboard_write,
            enquiry_response: config.enquiry_response.clone(),
            conditional_state: config.conditional_state,
        }
    }
}

/// The state we need to keep around only until we enter the IO
/// thread. Then we can throw it all away.
struct ThreadEnterState {
    /// Initial input to send to the subprocess after starting.
    input: Vec<ReadableIO>,
}

impl ThreadEnterState {
    fn create(config: &Config) -> Option<ThreadEnterState> {
        // If we have no input then we have no thread enter state
        if config.input.list.is_empty() {
            return None;
        }

        Some(ThreadEnterState {
            input: config.input.clone_parsed().list,
        })
    }

    /// Prepare the inputs for use: raw strings are used as is, files are read
    /// (up to 10 MiB each).
    fn prepare_input(&self) -> Result<Vec<Vec<u8>>, Error> {
        let mut out = Vec::with_capacity(self.input.len());
        for item in &self.input {
            match item {
                ReadableIO::Raw(v) => out.push(v.clone().into_bytes()),
                ReadableIO::Path(path) => {
                    let f = std::fs::File::open(path).map_err(|err| {
                        log::warn!("failed to open input file={path} err={err}");
                        Error::InputNotFound
                    })?;
                    let mut buf = Vec::new();
                    // 10 MiB max
                    let limit = 10 * 1024 * 1024;
                    f.take(limit + 1).read_to_end(&mut buf).map_err(|err| {
                        log::warn!("failed to read input file err={err}");
                        Error::InputFailed
                    })?;
                    if buf.len() as u64 > limit {
                        log::warn!("failed to read input file err=StreamTooLong");
                        return Err(Error::InputFailed);
                    }
                    out.push(buf);
                }
            }
        }
        Ok(out)
    }
}

/// What the stream needs mutual exclusion for (under the terminal lock).
struct OutputInner {
    stream: TerminalStream,
    /// Last time the cursor was reset. This is used to prevent message
    /// flooding with cursor resets.
    last_cursor_reset: Option<Instant>,
}

/// The part of termio used from the pty read thread: parses the pty output
/// and applies it to the shared terminal.
pub struct Output {
    state: SharedState,
    inner: Mutex<OutputInner>,
    renderer_mailbox: Arc<dyn RendererMailbox>,
    mailbox: Mailbox,
    surface_mailbox: Arc<dyn SurfaceMailbox>,
}

impl Output {
    fn inner(&self) -> MutexGuard<'_, OutputInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Process output from the pty. This is the manual API that users can
    /// call with pty data but it is also called by the read thread when using
    /// an exec subprocess.
    pub fn process_output(&self, buf: &[u8]) {
        // We are modifying terminal state from here on out and we need
        // the lock to grab our read data.
        let outbox = {
            let mut term = self.state.lock_terminal();
            let inspector = term.state().inspector.clone();
            let mut inner = self.inner();
            self.process_output_locked(&mut term, &mut inner, buf, inspector.as_deref());

            // If our stream handling caused messages to be sent to the
            // mailbox thread, then we need to wake it up so that it
            // processes them.
            if inner.stream.state.take_termio_messaged() {
                self.mailbox.notify();
            }
            inner.stream.state.take_outbox()
        };

        // Anything that didn't fit in a full mailbox is delivered now that
        // the locks are released.
        if !outbox.is_empty() {
            outbox.deliver(&self.mailbox, &*self.surface_mailbox);
        }
    }

    /// Process output from the pty but the lock is already held.
    fn process_output_locked(
        &self,
        term: &mut Terminal,
        inner: &mut OutputInner,
        buf: &[u8],
        inspector: Option<&dyn InspectorHook>,
    ) {
        // Schedule a render. We can call this first because we have the lock.
        inner.stream.state.queue_render();

        // Whenever a character is typed, we ensure the cursor is in the
        // non-blink state so it is rendered if visible. If we're under
        // HEAVY read load, we don't want to send a ton of these so we
        // use a timer under the covers
        let now = Instant::now();
        let reset = match inner.last_cursor_reset {
            Some(last) => now.duration_since(last) > Duration::from_millis(500),
            None => true,
        };
        if reset {
            inner.last_cursor_reset = Some(now);
            self.renderer_mailbox
                .push(RendererMessage::ResetCursorBlink, Timeout::Instant);
        }

        // If we have an inspector, we enter SLOW MODE because we need to
        // process a byte at a time alternating between the inspector handler
        // and the termio handler. This is very slow compared to our
        // optimizations below but at least users only pay for it if they're
        // using the inspector.
        if let Some(insp) = inspector {
            for &byte in buf {
                insp.record_pty_read(term, &[byte]);

                if let Err(err) = inner.stream.process_byte(term, byte) {
                    log::error!("error processing terminal data: {err:?}");
                }
            }
        } else if let Err(err) = inner.stream.process(term, buf) {
            log::error!("error processing terminal data: {err:?}");
        }
    }
}

/// See the module docs.
pub struct Termio {
    /// This is the implementation responsible for io.
    backend: Backend,

    /// The derived configuration for this termio implementation.
    config: DerivedConfig,

    /// The shared render state (contains the terminal).
    renderer_state: SharedState,

    /// The part shared with the pty read thread.
    output: Arc<Output>,

    /// A handle to wake up the renderer. This hints to the renderer that that
    /// a repaint should happen.
    renderer_wakeup: Wakeup,

    /// The mailbox for notifying the renderer of things.
    renderer_mailbox: Arc<dyn RendererMailbox>,

    /// The mailbox for communicating with the surface.
    surface_mailbox: Arc<dyn SurfaceMailbox>,

    /// The cached size info
    size: Size,

    /// The mailbox implementation to use.
    mailbox: Mailbox,

    /// State we have for thread enter. This may be None if we don't need
    /// to keep track of any state or if its already been freed.
    thread_enter_state: Option<ThreadEnterState>,
}

/// ThreadData is the data created and stored in the termio thread
/// when the thread is started and destroyed when the thread is
/// stopped.
///
/// All of the fields in this struct should only be read/written by
/// the termio thread. As such, a lock is not necessary.
pub struct ThreadData {
    /// The shared render state
    pub renderer_state: SharedState,

    /// Mailboxes for different threads
    pub surface_mailbox: Arc<dyn SurfaceMailbox>,

    /// Data associated with the backend implementation (i.e. pty/exec state)
    pub backend: backend::ThreadData,
    pub mailbox: Mailbox,
}

impl Termio {
    /// Initialize the termio state.
    ///
    /// This does NOT start the child process: that happens in
    /// [`Termio::thread_enter`] on the IO thread.
    pub fn init(opts: Options<'_>) -> Result<Termio, Error> {
        let Options {
            size,
            full_config,
            config,
            mut backend,
            mailbox,
            renderer_wakeup,
            renderer_mailbox,
            surface_mailbox,
        } = opts;

        // The default terminal modes based on our config.
        let default_modes: ModePacked = {
            let mut modes = ModePacked::default();

            // Setup our initial grapheme cluster support if enabled.
            match full_config.grapheme_width_method {
                GraphemeWidthMethod::Unicode => modes.set(Mode::GraphemeCluster, true),
                GraphemeWidthMethod::Legacy => {}
            }

            // Set default cursor blink settings
            modes.set(Mode::CursorBlinking, config.cursor_blink.unwrap_or(true));

            modes
        };

        // Create our terminal
        let grid_size = size.grid();
        let mut term = Terminal::new(TerminalOptions {
            cols: grid_size.columns,
            rows: grid_size.rows,
            max_scrollback: full_config.scrollback_limit,
            default_modes,
            colors: Colors {
                background: DynamicRgb::init(config.background.to_terminal_rgb()),
                foreground: DynamicRgb::init(config.foreground.to_terminal_rgb()),
                cursor: config
                    .cursor_color
                    .and_then(TerminalColor::to_terminal_rgb)
                    .map_or(DynamicRgb::UNSET, DynamicRgb::init),
                palette: DynamicPalette::init(config.palette),
            },
        })?;

        // Set the image size limits
        set_image_limits(&mut term, config.image_storage_limit);

        // Set our default cursor style
        term.screen_mut().cursor.cursor_style = config.cursor_style;

        // Setup our terminal size in pixels for certain requests.
        term.width_px = u32::from(term.cols) * size.cell.width;
        term.height_px = u32::from(term.rows) * size.cell.height;

        // Setup our backend.
        backend.init_terminal(&mut term);

        // Create our stream handler.
        let handler = HandlerState::new(HandlerOptions {
            size,
            termio_mailbox: mailbox.clone(),
            surface_mailbox: Arc::clone(&surface_mailbox),
            renderer_wakeup: renderer_wakeup.clone(),
            default_cursor_style: config.cursor_style,
            default_cursor_blink: config.cursor_blink,
            enquiry_response: config.enquiry_response.clone().into_bytes(),
            osc_color_report_format: config.osc_color_report_format,
            clipboard_write: config.clipboard_write,
        });

        let thread_enter_state = ThreadEnterState::create(full_config);

        let renderer_state = State::new(term).into_shared();
        let output = Arc::new(Output {
            state: Arc::clone(&renderer_state),
            inner: Mutex::new(OutputInner {
                stream: TerminalStream::new(handler),
                last_cursor_reset: None,
            }),
            renderer_mailbox: Arc::clone(&renderer_mailbox),
            mailbox: mailbox.clone(),
            surface_mailbox: Arc::clone(&surface_mailbox),
        });

        Ok(Termio {
            backend,
            config,
            renderer_state,
            output,
            renderer_wakeup,
            renderer_mailbox,
            surface_mailbox,
            size,
            mailbox,
            thread_enter_state,
        })
    }

    /// The shared render state (it holds the terminal). Hand this to the
    /// renderer.
    pub fn renderer_state(&self) -> &SharedState {
        &self.renderer_state
    }

    /// The termio mailbox (clone it to send messages to the IO thread).
    pub fn mailbox(&self) -> &Mailbox {
        &self.mailbox
    }

    /// The part of termio shared with the pty read thread; also the manual
    /// API to feed output (`process_output`).
    pub fn output(&self) -> &Arc<Output> {
        &self.output
    }

    /// The surface mailbox.
    pub fn surface_mailbox(&self) -> &Arc<dyn SurfaceMailbox> {
        &self.surface_mailbox
    }

    /// The derived config currently in effect.
    pub fn config(&self) -> &DerivedConfig {
        &self.config
    }

    /// The current size.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The renderer wakeup handle.
    pub fn renderer_wakeup(&self) -> &Wakeup {
        &self.renderer_wakeup
    }

    pub fn thread_enter(&mut self) -> Result<ThreadData, Error> {
        // Always free our thread enter state when we're done.
        let enter_state = self.thread_enter_state.take();

        // If we have thread enter state then we're going to validate
        // and set that all up now so that we can error before we actually
        // start the command and pty.
        let inputs: Vec<Vec<u8>> = match &enter_state {
            Some(v) => v.prepare_input()?,
            None => Vec::new(),
        };

        // Setup our backend
        let backend = self.backend.thread_enter(
            Arc::clone(&self.output),
            Arc::clone(&self.surface_mailbox),
            Arc::clone(&self.renderer_state),
        )?;
        let mut data = ThreadData {
            renderer_state: Arc::clone(&self.renderer_state),
            surface_mailbox: Arc::clone(&self.surface_mailbox),
            backend,
            mailbox: self.mailbox.clone(),
        };

        // If we have inputs, then queue them all up.
        for input in inputs {
            self.queue_write(&mut data, &input, false);
        }

        Ok(data)
    }

    pub fn thread_exit(&mut self, data: &mut ThreadData) {
        self.backend.thread_exit(&mut data.backend);
    }

    /// Send a message to the the mailbox and notify the IO thread.
    ///
    /// Do not call this while holding the terminal lock if the mailbox may
    /// be full; use `Mailbox::send_locked` + `Mailbox::notify` in that case.
    pub fn queue_message(&self, msg: Message) {
        self.mailbox.send(msg);
        self.mailbox.notify();
    }

    /// Queue a write directly to the pty.
    ///
    /// If you're using `Thread`, this must ONLY be called from the mailbox
    /// thread. If you're not on the thread, use `queue_message` with
    /// mailbox messages instead.
    pub fn queue_write(&mut self, td: &mut ThreadData, data: &[u8], linefeed: bool) {
        self.backend.queue_write(&mut td.backend, data, linefeed);
    }

    /// Update the configuration.
    pub fn change_config(&mut self, td: &mut ThreadData, config: DerivedConfig) {
        // The remainder of this function is modifying terminal state or
        // the read thread data, all of which requires holding the renderer
        // state lock.
        let mut term = self.renderer_state.lock_terminal();

        // Update our stream handler. The stream handler uses the same
        // renderer mutex so this is safe to do despite being executed
        // from another thread.
        self.output.inner().stream.change_config(&mut term, &config);
        td.backend.change_config(&config);

        // Update the configuration that we know about.
        //
        // Specific things we don't update:
        //   - command, working-directory: we never restart the underlying
        //   process so we don't care or need to know about these.

        // Update the default palette.
        term.colors.palette.change_default(config.palette);
        term.flags.dirty.palette = true;

        // Update all our other colors
        term.colors.background.default = Some(config.background.to_terminal_rgb());
        term.colors.foreground.default = Some(config.foreground.to_terminal_rgb());
        term.colors.cursor.default = config.cursor_color.and_then(TerminalColor::to_terminal_rgb);

        // Set the image size limits
        set_image_limits(&mut term, config.image_storage_limit);

        self.config = config;
    }

    /// Resize the terminal.
    pub fn resize(&mut self, td: &mut ThreadData, size: Size) -> Result<(), Error> {
        self.size = size;
        let grid_size = size.grid();

        // Update the size of our pty.
        self.backend.resize(grid_size, size.terminal())?;

        // Enter the critical area that we want to keep small
        {
            let state = Arc::clone(&self.renderer_state);
            let mut term = state.lock_terminal();
            self.output().inner().stream.state.size = size;

            // Update the size of our terminal state
            term.resize(grid_size.columns, grid_size.rows)?;

            // Update our pixel sizes
            term.width_px = u32::from(grid_size.columns) * size.cell.width;
            term.height_px = u32::from(grid_size.rows) * size.cell.height;

            // Disable synchronized output mode so that we show changes
            // immediately for a resize. This is allowed by the spec.
            term.modes.set(Mode::SynchronizedOutput, false);

            // If we have size reporting enabled we need to send a report.
            if term.modes.get(Mode::InBandSizeReports) {
                self.size_report_locked(td, SizeReport::Mode2048);
            }
        }

        // Mail the renderer so that it can update the GPU and re-render
        self.renderer_mailbox
            .push(RendererMessage::Resize(size), Timeout::Forever);
        if let Err(err) = self.renderer_wakeup.notify() {
            log::warn!("failed to notify renderer err={err}");
        }
        Ok(())
    }

    /// Make a size report.
    pub fn size_report(&mut self, td: &mut ThreadData, style: SizeReport) {
        let state = Arc::clone(&self.renderer_state);
        let _term = state.lock_terminal();
        self.size_report_locked(td, style);
    }

    fn size_report_locked(&mut self, td: &mut ThreadData, style: SizeReport) {
        let grid_size = self.size.grid();
        let cell = self.size.cell;
        let columns = u32::from(grid_size.columns);
        let rows = u32::from(grid_size.rows);

        let message = match style {
            SizeReport::Mode2048 => format!(
                "\x1B[48;{};{};{};{}t",
                rows,
                columns,
                rows * cell.height,
                columns * cell.width,
            ),
            SizeReport::Csi14T => {
                format!("\x1b[4;{};{}t", rows * cell.height, columns * cell.width)
            }
            SizeReport::Csi16T => format!("\x1b[6;{};{}t", cell.height, cell.width),
            SizeReport::Csi18T => format!("\x1b[8;{rows};{columns}t"),
        };

        self.queue_write(td, message.as_bytes(), false);
    }

    /// Reset the synchronized output mode. This is usually called by timer
    /// expiration from the termio thread.
    pub fn reset_synchronized_output(&mut self) {
        let mut term = self.renderer_state.lock_terminal();
        term.modes.set(Mode::SynchronizedOutput, false);
        if let Err(err) = self.renderer_wakeup.notify() {
            log::warn!("failed to notify renderer err={err}");
        }
    }

    /// Clear the screen.
    pub fn clear_screen(&mut self, td: &mut ThreadData, history: bool) {
        {
            let mut term = self.renderer_state.lock_terminal();

            // If we're on the alternate screen, we do not clear. Since this is an
            // emulator-level screen clear, this messes up the running programs
            // knowledge of where the cursor is and causes rendering issues. So,
            // for alt screen, we do nothing.
            if term.screens.active_key == ScreenKey::Alternate {
                return;
            }

            // Clear our selection
            term.screen_mut().clear_selection();

            // Clear our scrollback
            if history {
                term.erase_display(csi::EraseDisplay::Scrollback, false);
            }

            // If we're not at a prompt, we just delete above the cursor.
            if !term.cursor_is_at_prompt() {
                let y = term.screen().cursor.y;
                if y > 0 {
                    term.screen_mut().erase_rows(
                        Point::Active(Coordinate { x: 0, y: 0 }),
                        Some(Point::Active(Coordinate {
                            x: 0,
                            y: u32::from(y) - 1,
                        })),
                    );
                }

                // Clear all Kitty graphics state for this screen. This copies
                // Kitty's behavior when Cmd+K deletes all Kitty graphics. I
                // didn't spend time researching whether it only deletes Kitty
                // graphics that are placed above the cursor or if it deletes
                // all of them. We delete all of them for now but if this behavior
                // isn't fully correct we should fix this later.
                let info = TerminalInfo::from_terminal(&term);
                let screen = term.screen_mut();
                screen
                    .kitty_images
                    .delete(&mut screen.pages, &info, KittyDelete::All(true));

                return;
            }

            // At a prompt, we want to first fully clear the screen, and then after
            // send a FF (0x0C) to the shell so that it can repaint the screen.
            term.erase_display(csi::EraseDisplay::Complete, false);
        }

        // If we reached here it means we're at a prompt, so we send a form-feed.
        self.queue_write(td, &[0x0C], false);
    }

    /// Scroll the viewport
    pub fn scroll_viewport(&mut self, scroll: ScrollViewport) {
        self.renderer_state.lock_terminal().scroll_viewport(scroll);
    }

    /// Jump the viewport to the prompt.
    pub fn jump_to_prompt(&mut self, delta: isize) {
        {
            let mut term = self.renderer_state.lock_terminal();
            term.screen_mut().scroll(Scroll::DeltaPrompt(delta));
        }

        if let Err(err) = self.renderer_wakeup.notify() {
            log::warn!("failed to notify renderer err={err}");
        }
    }

    /// Called when focus is gained or lost (when focus events are enabled)
    pub fn focus_gained(&mut self, td: &mut ThreadData, focused: bool) {
        let focus_event = self
            .renderer_state
            .lock_terminal()
            .modes
            .get(Mode::FocusEvent);

        // If we have focus events enabled, we send the focus event.
        if focus_event {
            let seq: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
            self.queue_write(td, seq, false);
        }

        // We always notify our backend of focus changes.
        self.backend.focus_gained(&mut td.backend, focused);
    }

    /// Process output from the pty. See [`Output::process_output`].
    pub fn process_output(&self, buf: &[u8]) {
        self.output.process_output(buf);
    }

    /// Sends a DSR response for the current color scheme to the pty.
    pub fn color_scheme_report(&mut self, td: &mut ThreadData, force: bool) {
        let state = Arc::clone(&self.renderer_state);
        let term = state.lock_terminal();
        self.color_scheme_report_locked(&term, td, force);
    }

    pub fn color_scheme_report_locked(
        &mut self,
        term: &Terminal,
        td: &mut ThreadData,
        force: bool,
    ) {
        if !force && !term.modes.get(Mode::ReportColorScheme) {
            return;
        }
        let output: &[u8] = match self.config.conditional_state.theme {
            conditional::Theme::Light => b"\x1B[?997;2n",
            conditional::Theme::Dark => b"\x1B[?997;1n",
        };
        self.queue_write(td, output, false);
    }
}

/// Apply `image-storage-limit` to every initialized screen.
fn set_image_limits(term: &mut Terminal, limit: usize) {
    for key in [ScreenKey::Primary, ScreenKey::Alternate] {
        if let Some(screen) = term.screens.get_mut(key)
            && let Err(err) = screen.kitty_images.set_limit(&mut screen.pages, limit)
        {
            log::warn!("failed to set kitty image storage limit: {err}");
        }
    }
}
