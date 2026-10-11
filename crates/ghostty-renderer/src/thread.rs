//! The renderer thread (`renderer/Thread.zig`).
//!
//! # Threading model (GTK / OpenGL)
//!
//! Ghostty's OpenGL renderer on GTK cannot draw from the renderer thread: the
//! `GtkGLArea` context is only current on the GTK main thread, inside the
//! `render` signal. So (`must_draw_from_app_thread = true`):
//!
//! ```text
//!   termio thread --(wakeup)--> renderer thread --(redraw_surface)--> GTK main thread
//!                                  |                                      |
//!                    update_frame: lock terminal,                 GLArea::render ->
//!                    shape + rebuild CPU cell buffers             Renderer::draw_frame:
//!                    (no GL calls)                                upload + draw + present
//! ```
//!
//! * The thread owns timers (cursor blink 600ms, draw timer 8ms for custom
//!   shader animation), drains its [`Mailbox`], and on every wakeup runs
//!   `update_frame` followed by a *draw request*.
//! * A draw request is [`Host::redraw_surface`]; the apprt answers by calling
//!   `queue_render()` on the GLArea, whose `render` handler calls
//!   [`Renderer::draw_frame`]. Both threads synchronise through the renderer's
//!   draw mutex.
//! * [`Handle::draw_now`] is the "draw now" async of Zig (used by the apprt for
//!   vsync-driven renderers; unused for OpenGL on Linux).
//!
//! For headless use (tests, no GLArea) [`Thread::draw_on_app_thread`] can be
//! set to `false`, in which case the thread calls `draw_frame` itself; the GL
//! context must then be current on the renderer thread.
//!
//! The event loop (libxev in Zig) is replaced by a `crossbeam_channel::select!`
//! over the wakeup / stop / draw-now channels and the next timer deadline.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, never, select};
use ghostty_config::config::Config;
use ghostty_config::types_a::CustomShaderAnimation;
use ghostty_datastruct::blocking_queue::{BlockingQueue, Timeout};

use crate::generic::{Host, Renderer};
use crate::message::Message;
use crate::state::SharedState;

/// 120 FPS
pub const DRAW_INTERVAL: Duration = Duration::from_millis(8);
pub const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(600);

/// The type used for sending messages to the renderer thread. For now this is
/// hardcoded with a capacity. We can make this a comptime parameter in
/// the future if we want it configurable.
pub type Mailbox = BlockingQueue<Message, 64>;

/// Configuration we need derived from the main config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedConfig {
    pub custom_shader_animation: CustomShaderAnimation,
}

impl DerivedConfig {
    pub fn new(config: &Config) -> DerivedConfig {
        DerivedConfig {
            custom_shader_animation: config.custom_shader_animation,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    /// This is true when a blinking cursor should be visible and false
    /// when it should not be visible. This is toggled on a timer by the
    /// thread automatically.
    cursor_blink_visible: bool,

    /// This is true when the inspector is active.
    has_inspector: bool,

    /// This is true when the view is visible. This is used to determine
    /// if we should be rendering or not.
    visible: bool,

    /// This is true when the view is focused. This defaults to true
    /// and it is up to the apprt to set the correct value.
    focused: bool,
}

/// A cloneable handle to a (possibly running) renderer thread. This is what
/// the surface and the apprt keep to talk to the thread from any thread.
#[derive(Clone)]
pub struct Handle {
    /// The mailbox that can be used to send this thread messages. Note
    /// this is a blocking queue so if it is full you will get errors (or block).
    pub mailbox: Arc<Mailbox>,
    wakeup: Sender<()>,
    stop: Sender<()>,
    draw_now: Sender<()>,
}

impl Handle {
    /// Push a message. Returns false if the mailbox was full (and `timeout`
    /// elapsed). This does NOT wake the thread; call [`Handle::wakeup`]
    /// (Zig: `mailbox.push(msg, .{.forever}); thread.wakeup.notify()`).
    pub fn push(&self, msg: Message, timeout: Timeout) -> bool {
        self.mailbox.push(msg, timeout) > 0
    }

    /// Push a message (blocking if full) and wake the thread.
    pub fn send(&self, msg: Message) -> bool {
        let ok = self.push(msg, Timeout::Forever);
        self.wakeup();
        ok
    }

    /// Wake the renderer up and force a render safely from any thread.
    /// Wakeups coalesce.
    pub fn wakeup(&self) {
        let _ = self.wakeup.try_send(());
    }

    /// Force a draw request immediately (does not update frame data).
    pub fn draw_now(&self) {
        let _ = self.draw_now.try_send(());
    }

    /// Stop the renderer on the next loop iteration.
    pub fn stop(&self) {
        let _ = self.stop.try_send(());
    }
}

/// A started renderer thread.
pub struct Running {
    pub handle: Handle,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Stop the thread and wait for it to exit. After this returns the
    /// thread has dropped its reference to the renderer, so the caller's
    /// `Arc<Renderer>` is the last one (and can be dropped on the GL thread).
    pub fn stop_and_join(mut self) {
        self.handle.stop();
        if let Some(j) = self.join.take()
            && j.join().is_err()
        {
            log::warn!("renderer thread panicked");
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            self.handle.stop();
            let _ = j.join();
        }
    }
}

/// The renderer thread state. Construct with [`Thread::new`] then start it
/// with [`Thread::spawn`] (or run [`Thread::thread_main`] on a thread you
/// manage).
pub struct Thread {
    /// The underlying renderer implementation.
    renderer: Arc<Renderer>,

    /// Pointer to the shared state that is used to generate the final render.
    state: SharedState,

    /// The surface/apprt callbacks.
    host: Arc<dyn Host>,

    /// Configuration we need derived from the main config.
    config: DerivedConfig,

    /// If true (the default, GTK), draws are requested from the apprt via
    /// [`Host::redraw_surface`] instead of being performed on this thread
    /// (`must_draw_from_app_thread`).
    pub draw_on_app_thread: bool,

    flags: Flags,

    handle: Handle,
    wakeup_rx: Receiver<()>,
    wakeup_tx: Sender<()>,
    stop_rx: Receiver<()>,
    draw_now_rx: Receiver<()>,

    /// Deadline of the cursor blink timer; None when it isn't running.
    cursor_deadline: Option<Instant>,

    /// Whether the draw timer should keep re-arming, and its next deadline.
    draw_active: bool,
    draw_deadline: Option<Instant>,
}

impl Thread {
    /// Initialize the thread. This does not START the thread. This only sets
    /// up all the internal state necessary prior to starting the thread. It
    /// is up to the caller to start the thread with [`Thread::spawn`].
    pub fn new(
        config: &Config,
        renderer: Arc<Renderer>,
        state: SharedState,
        host: Arc<dyn Host>,
    ) -> Thread {
        // Wakeups, stop and draw-now coalesce (xev.Async semantics).
        let (wakeup_tx, wakeup_rx) = bounded(1);
        let (stop_tx, stop_rx) = bounded(1);
        let (draw_now_tx, draw_now_rx) = bounded(1);

        let handle = Handle {
            mailbox: Arc::new(Mailbox::new()),
            wakeup: wakeup_tx.clone(),
            stop: stop_tx,
            draw_now: draw_now_tx,
        };

        Thread {
            renderer,
            state,
            host,
            config: DerivedConfig::new(config),
            draw_on_app_thread: true,
            flags: Flags {
                cursor_blink_visible: false,
                has_inspector: false,
                visible: true,
                focused: true,
            },
            handle,
            wakeup_rx,
            wakeup_tx,
            stop_rx,
            draw_now_rx,
            cursor_deadline: None,
            draw_active: false,
            draw_deadline: None,
        }
    }

    /// The handle used to message this thread. Available before the thread
    /// is started so the surface can queue messages.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Start the thread (named "renderer"). The returned [`Running`] keeps
    /// the [`Handle`].
    pub fn spawn(self) -> std::io::Result<Running> {
        let handle = self.handle.clone();
        let join = std::thread::Builder::new()
            .name("renderer".into())
            .spawn(move || self.thread_main())?;
        Ok(Running {
            handle,
            join: Some(join),
        })
    }

    /// The main entrypoint for the thread.
    pub fn thread_main(mut self) {
        log::debug!("starting renderer thread");

        // Run our thread start/end callbacks. For OpenGL on GTK these are
        // no-ops: all GL work is on the main thread.

        // Send an initial wakeup message so that we render right away.
        let _ = self.wakeup_tx.try_send(());

        // Start blinking the cursor.
        self.cursor_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);

        // Start the draw timer
        self.sync_draw_timer();

        // Run
        loop {
            let next = [self.cursor_deadline, self.draw_deadline]
                .into_iter()
                .flatten()
                .min();
            let timer = next.map_or_else(never, crossbeam_channel::at);

            select! {
                recv(self.wakeup_rx) -> _ => {
                    // When we wake up, we check the mailbox. Mailbox producers
                    // should wake up our thread after publishing.
                    self.drain_mailbox();

                    // Render immediately
                    self.render_callback();
                }
                recv(self.stop_rx) -> _ => break,
                recv(self.draw_now_rx) -> _ => {
                    // Draw immediately
                    self.draw_frame(true);
                }
                recv(timer) -> _ => self.timers_due(),
            }
        }

        log::debug!("starting renderer thread shutdown");
    }

    /// Fire whichever timers are due.
    fn timers_due(&mut self) {
        let now = Instant::now();
        if self.cursor_deadline.is_some_and(|d| d <= now) {
            self.cursor_timer_callback();
        }
        if self.draw_deadline.is_some_and(|d| d <= now) {
            self.draw_callback();
        }
    }

    fn sync_draw_timer(&mut self) {
        if !should_draw_timer_run(
            self.renderer.has_animations(),
            self.config.custom_shader_animation,
            self.flags.focused,
        ) {
            // We're skipping the draw timer. Stop it on the next iteration.
            self.draw_active = false;
            return;
        }

        // Set our active state so it knows we're running. We set this before
        // even checking the active state in case we have a pending shutdown.
        self.draw_active = true;

        // If our draw timer is already active, then we don't have to do anything.
        if self.draw_deadline.is_some() {
            return;
        }

        // Start the timer which loops
        self.draw_deadline = Some(Instant::now() + DRAW_INTERVAL);
    }

    /// Drain the mailbox.
    fn drain_mailbox(&mut self) {
        while let Some(message) = self.handle.mailbox.pop() {
            match message {
                Message::Crash => panic!("crash request, crashing intentionally"),

                Message::Visible(v) => {
                    // If our state didn't change we do nothing.
                    if self.flags.visible == v {
                        continue;
                    }

                    // Set our visible state
                    self.flags.visible = v;

                    // If we became visible then we immediately trigger a draw.
                    // We don't need to update frame data because that should
                    // still be happening.
                    if v {
                        self.draw_frame(false);
                    }

                    // Notify the renderer so it can update any state.
                    self.renderer.set_visible(v);

                    // Note that we're explicitly today not stopping any
                    // cursor timers, draw timers, etc. These things have very
                    // little resource cost and properly maintaining their active
                    // state across different transitions is going to be bug-prone,
                    // so its easier to just let them keep firing and have them
                    // check the visible state themselves to control their behavior.
                }

                Message::Focus(v) => {
                    // If our state didn't change we do nothing.
                    if self.flags.focused == v {
                        continue;
                    }

                    // Set our state
                    self.flags.focused = v;

                    // Set it on the renderer
                    self.renderer.set_focus(v);

                    // We always resync our draw timer (may disable it)
                    self.sync_draw_timer();

                    if v {
                        // If we're focused, we immediately show the cursor again
                        // and then restart the timer.
                        if self.cursor_deadline.is_none() {
                            self.flags.cursor_blink_visible = true;
                            self.cursor_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
                        }
                    } else {
                        // If we're not focused, then we stop the cursor blink
                        self.cursor_deadline = None;
                    }
                }

                Message::ResetCursorBlink => {
                    self.flags.cursor_blink_visible = true;
                    if self.cursor_deadline.is_some() {
                        self.cursor_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
                    }
                }

                Message::FontGrid(grid) => {
                    let grid = *grid;
                    self.renderer.set_font_grid(grid.grid);
                    grid.set.deref(&grid.old_key);
                }

                Message::Resize(v) => self.renderer.set_screen_size(v),

                Message::ChangeConfig { thread, r#impl } => {
                    self.config = *thread;
                    if let Err(err) = self.renderer.change_config(*r#impl) {
                        log::warn!("error changing renderer config err={err}");
                    }

                    // Stop and start the draw timer to capture the new
                    // has_animations value.
                    self.sync_draw_timer();
                }

                Message::SearchViewportMatches(v) => {
                    self.renderer.set_search_viewport_matches(v);
                }

                Message::SearchSelectedMatch(v) => {
                    self.renderer.set_search_selected_match(v);
                }

                Message::Inspector(v) => {
                    self.flags.has_inspector = v;
                }

                Message::MacosDisplayId(_) => {}
            }
        }
    }

    /// Trigger a draw. This will not update frame data or anything, it will
    /// just trigger a draw/paint.
    fn draw_frame(&self, now: bool) {
        // If we're invisible, we do not draw.
        if !self.flags.visible {
            return;
        }

        // If the renderer is managing a vsync on its own, we only draw
        // when we're forced to via `now`.
        if !now && self.renderer.has_vsync() {
            return;
        }

        if self.draw_on_app_thread {
            let _ = self.host.redraw_surface();
        } else if let Err(err) = self.renderer.draw_frame(false) {
            log::warn!("error drawing err={err}");
        }
    }

    fn render_callback(&self) {
        // Update our frame data
        if let Err(err) = self
            .renderer
            .update_frame(&self.state, self.flags.cursor_blink_visible)
        {
            log::warn!("error rendering err={err}");
        }

        // Draw
        self.draw_frame(false);
    }

    fn draw_callback(&mut self) {
        // Draw
        self.draw_frame(false);

        // Only continue if we're still active
        self.draw_deadline = if self.draw_active {
            Some(Instant::now() + DRAW_INTERVAL)
        } else {
            None
        };
    }

    fn cursor_timer_callback(&mut self) {
        self.flags.cursor_blink_visible = !self.flags.cursor_blink_visible;
        let _ = self.wakeup_tx.try_send(());

        self.cursor_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
    }
}

/// Whether the draw timer (used for custom shader animation) should run.
fn should_draw_timer_run(
    has_animations: bool,
    animation: CustomShaderAnimation,
    focused: bool,
) -> bool {
    // If our renderer supports animations and has them, then we
    // can apply draw timer based on custom shader animation configuration.
    if has_animations {
        match animation {
            // Always animate
            CustomShaderAnimation::Always => return true,
            // Only when focused
            CustomShaderAnimation::True => {
                if focused {
                    return true;
                }
            }
            // Never animate
            CustomShaderAnimation::False => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_timer_policy() {
        use CustomShaderAnimation::{Always, False, True};
        // No animations: never runs.
        for a in [Always, True, False] {
            for f in [true, false] {
                assert!(!should_draw_timer_run(false, a, f));
            }
        }
        // Animations: always / only-when-focused / never.
        assert!(should_draw_timer_run(true, Always, false));
        assert!(should_draw_timer_run(true, Always, true));
        assert!(should_draw_timer_run(true, True, true));
        assert!(!should_draw_timer_run(true, True, false));
        assert!(!should_draw_timer_run(true, False, true));
    }

    #[test]
    fn intervals_match_zig() {
        assert_eq!(DRAW_INTERVAL, Duration::from_millis(8));
        assert_eq!(CURSOR_BLINK_INTERVAL, Duration::from_millis(600));
    }

    #[test]
    fn derived_config_from_config() {
        let c = Config {
            custom_shader_animation: CustomShaderAnimation::Always,
            ..Config::default()
        };
        assert_eq!(
            DerivedConfig::new(&c).custom_shader_animation,
            CustomShaderAnimation::Always
        );
    }
}
