//! App is the primary GUI application for ghostty (port of `App.zig`).
//!
//! The Zig `App` is a heap object shared by the runtime and every surface.
//! In Rust the runtime holds an [`App`] (it implements
//! [`ghostty_apprt::runtime_api::CoreApp`]) and surfaces hold a shared
//! [`AppState`]. Everything here is main-thread only except the
//! [`Mailbox`], which is how other threads (renderer, IO, search) talk to
//! the app thread.
//!
//! Surfaces are identified by the [`SurfaceId`] the runtime allocates; the
//! app keeps weak references to them (the runtime owns the strong
//! `Rc<dyn CoreSurface>`).

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::Arc,
    time::Instant,
};

use crossbeam_channel::{Receiver, Sender};
use ghostty_apprt::{
    action::{Action as RtAction, QuitTimer, ReloadConfig, Target},
    runtime_api::{
        AppMessage,
        CellSize as RtCellSize,
        CoreApp,
        CoreError,
        CoreSurface,
        DesiredFontSize,
        RtApp,
        RtSurface,
        SurfaceId,
        Waker,
    },
    structs::ColorScheme,
    surface::{AppMailbox, Message as SurfaceMessage},
};
use ghostty_config::{conditional, config::Config};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_font::SharedGridSet;
use ghostty_input::{
    KeyEvent,
    binding::{Action as BindingAction, Scope, set::Value as BindingValue},
};

use crate::surface::Surface;

/// Capacity of the app mailbox (Zig: `BlockingQueue(Message, 64)`).
const MAILBOX_CAPACITY: usize = 64;

/// The message types that can be sent to the app thread.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    /// Open the configuration file.
    OpenConfig,

    /// Create a new terminal window.
    NewWindow {
        /// The parent surface.
        parent: Option<SurfaceId>,
    },

    /// Close a surface. This notifies the runtime that a surface should
    /// close.
    Close(SurfaceId),

    /// Quit.
    Quit,

    /// A message for a specific surface.
    SurfaceMessage {
        surface: SurfaceId,
        message: SurfaceMessage,
    },

    /// Redraw a surface. This only has an effect for runtimes that use
    /// single-threaded draws. To redraw a surface for all runtimes, wake up
    /// the renderer thread. The renderer thread will send this message if
    /// it needs to.
    RedrawSurface(SurfaceId),
}

/// The way that other threads send the app thread messages (Zig:
/// `App.Mailbox`). Cheap to clone; every push wakes the runtime.
#[derive(Clone)]
pub struct Mailbox {
    tx: Sender<Message>,
    waker: Arc<dyn Waker>,
    cancelled: Receiver<()>,
}

impl std::fmt::Debug for Mailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailbox").finish_non_exhaustive()
    }
}

/// Owns one surface's permission to send into the shared UI queue. Dropping
/// the sole cancellation sender broadcasts to every worker, including workers
/// already blocked on a full queue. Other surfaces keep their own scopes.
#[derive(Debug)]
pub(crate) struct MailboxScope {
    mailbox: Mailbox,
    cancel: Option<Sender<()>>,
}

impl MailboxScope {
    pub(crate) fn mailbox(&self) -> Mailbox {
        self.mailbox.clone()
    }

    pub(crate) fn close(&mut self) {
        self.cancel.take();
    }
}

impl Mailbox {
    pub(crate) fn scoped(&self) -> MailboxScope {
        let (cancel, cancelled) = crossbeam_channel::bounded(0);
        MailboxScope {
            mailbox: Mailbox {
                cancelled,
                ..self.clone()
            },
            cancel: Some(cancel),
        }
    }

    /// Send a message to the app. Returns the queue size after the push;
    /// zero means the push failed (closed scope, full queue, or timeout).
    /// Sends selected concurrently with scope closure may still enqueue, but
    /// cannot remain blocked. Calls after closure always prefer cancellation.
    pub fn push(&self, msg: Message, timeout: Timeout) -> usize {
        let ok = match timeout {
            Timeout::Instant => crossbeam_channel::select_biased! {
                recv(self.cancelled) -> _ => false,
                send(self.tx, msg) -> result => result.is_ok(),
                default => false,
            },
            Timeout::Forever => crossbeam_channel::select_biased! {
                recv(self.cancelled) -> _ => false,
                send(self.tx, msg) -> result => result.is_ok(),
            },
            Timeout::Ns(ns) => crossbeam_channel::select_biased! {
                recv(self.cancelled) -> _ => false,
                send(self.tx, msg) -> result => result.is_ok(),
                default(std::time::Duration::from_nanos(ns)) => false,
            },
        };
        if !ok {
            return 0;
        }

        // Wake up our app loop
        self.waker.wake();
        self.tx.len().max(1)
    }
}

impl AppMailbox<SurfaceId> for Mailbox {
    fn push_surface_message(
        &self,
        surface: SurfaceId,
        message: SurfaceMessage,
        timeout: Timeout,
    ) -> usize {
        self.push(Message::SurfaceMessage { surface, message }, timeout)
    }
}

/// State shared between the [`App`] and its surfaces (main thread only).
pub struct AppState {
    /// The list of surfaces that are currently active, in creation order.
    surfaces: RefCell<Vec<(SurfaceId, Weak<Surface>)>>,

    /// This is true if the app that Ghostty is in is focused. This may mean
    /// that no surfaces (terminals) are focused but the app is still
    /// focused, i.e. maybe an about window.
    focused: Cell<bool>,

    /// The last focused surface. This surface may not be valid; you must
    /// always validate it with [`AppState::focused_surface`].
    focused_surface: Cell<Option<SurfaceId>>,

    /// The mailbox other threads use to send this thread messages.
    pub mailbox: Mailbox,
    mailbox_rx: Receiver<Message>,

    /// The set of font grids shared by surfaces with the same font
    /// configuration.
    pub font_grid_set: Arc<SharedGridSet>,

    /// Used to rate limit desktop notifications.
    pub last_notification_time: Cell<Option<Instant>>,
    pub last_notification_digest: Cell<u64>,

    /// The conditional state of the configuration. This applies to the
    /// app-level config and as a default for new surfaces.
    pub config_conditional_state: Cell<conditional::State>,

    /// The runtime app, remembered from the first surface so the quit timer
    /// can be started when the last surface goes away.
    rt_app: RefCell<Option<Rc<dyn RtApp>>>,

    /// Set to false once we've created at least one surface. This never
    /// goes true again. Surfaces use it to determine if they are the first.
    pub first: Cell<bool>,
}

impl AppState {
    /// Ids and strong references of all live surfaces.
    pub fn surfaces(&self) -> Vec<Rc<Surface>> {
        self.surfaces
            .borrow()
            .iter()
            .filter_map(|(_, w)| w.upgrade())
            .collect()
    }

    /// Look up a live surface.
    pub fn surface(&self, id: SurfaceId) -> Option<Rc<Surface>> {
        self.surfaces
            .borrow()
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, w)| w.upgrade())
    }

    /// Whether the surface is known to the app (Zig: `hasSurface`).
    pub fn has_surface(&self, id: SurfaceId) -> bool {
        self.surfaces.borrow().iter().any(|(i, _)| *i == id)
    }

    /// The last focused surface (Zig: `focusedSurface`). Only valid on the
    /// main thread.
    pub fn focused_surface(&self) -> Option<Rc<Surface>> {
        self.surface(self.focused_surface.get()?)
    }

    /// Mark a surface as the focused one (Zig: `focusSurface`).
    pub fn focus_surface(&self, id: SurfaceId) {
        if !self.has_surface(id) {
            return;
        }
        self.focused_surface.set(Some(id));
    }

    /// Whether the whole app is focused.
    pub fn focused(&self) -> bool {
        self.focused.get()
    }
}

/// The core application. Implements [`CoreApp`] for the runtime.
pub struct App {
    state: Rc<AppState>,
}

impl App {
    /// Create a new app instance (Zig: `App.create`). `waker` is how
    /// non-main threads request an app `tick`.
    pub fn new(waker: Arc<dyn Waker>) -> Result<App, ghostty_font::shared_grid_set::InitError> {
        let font_grid_set = Arc::new(SharedGridSet::new()?);
        let (tx, rx) = crossbeam_channel::bounded(MAILBOX_CAPACITY);
        Ok(App {
            state: Rc::new(AppState {
                surfaces: RefCell::new(Vec::new()),
                focused: Cell::new(true),
                focused_surface: Cell::new(None),
                mailbox: Mailbox {
                    tx,
                    waker,
                    cancelled: crossbeam_channel::never(),
                },
                mailbox_rx: rx,
                font_grid_set,
                last_notification_time: Cell::new(None),
                last_notification_digest: Cell::new(0),
                config_conditional_state: Cell::new(conditional::State::default()),
                rt_app: RefCell::new(None),
                first: Cell::new(true),
            }),
        })
    }

    /// The shared state surfaces use.
    #[must_use]
    pub fn state(&self) -> &Rc<AppState> {
        &self.state
    }

    /// A clone of the mailbox for other threads.
    #[must_use]
    pub fn mailbox(&self) -> Mailbox {
        self.state.mailbox.clone()
    }

    /// Handle an app-level focus event. This should be called whenever the
    /// focus state of the entire app containing Ghostty changes.
    pub fn focus_event(&self, focused: bool) {
        // Prevent redundant focus events
        if self.state.focused.get() == focused {
            return;
        }

        log::debug!("focus event focused={focused}");
        self.state.focused.set(focused);
    }

    /// Returns true if the given key event would trigger a keybinding if it
    /// were to be processed (Zig: `keyEventIsBinding`).
    #[must_use]
    pub fn key_event_is_binding(&self, config: &Config, event: &KeyEvent<'_>) -> bool {
        match event.action {
            ghostty_input::Action::Release => return false,
            ghostty_input::Action::Press | ghostty_input::Action::Repeat => {}
        }

        // If we have a keybinding for this event then we return true.
        config.keybind.set.get_event(event).is_some()
    }

    /// Handle a key event at the app-scope. If this key event is used, this
    /// will return true and the caller shouldn't continue processing the
    /// event (Zig: `keyEvent`).
    ///
    /// If the app currently has focus then all key events are processed. If
    /// the app does not have focus then only global key events are
    /// processed.
    pub fn key_event(&self, rt: &dyn RtApp, config: &Config, event: &KeyEvent<'_>) -> bool {
        match event.action {
            // We don't care about key release events.
            ghostty_input::Action::Release => return false,
            // Continue processing key press events.
            ghostty_input::Action::Press | ghostty_input::Action::Repeat => {}
        }

        // Get the keybind entry for this event. We don't support key
        // sequences so we can look directly in the top-level set.
        let Some((_, value)) = config.keybind.set.get_event(event) else {
            return false;
        };
        let (actions, flags) = match value {
            // Sequences aren't supported. Our configuration parser verifies
            // this for global keybinds but we may still get an entry for a
            // non-global keybind.
            BindingValue::Leader(_) => return false,
            BindingValue::Leaf(leaf) => (vec![leaf.action.clone()], leaf.flags),
            BindingValue::LeafChained(leaf) => (leaf.actions.clone(), leaf.flags),
        };

        // If we aren't focused, then we only process global keybinds.
        if !self.state.focused.get() && !flags.global {
            return false;
        }

        // Global keybinds are done using performAll so that they can target
        // all surfaces too.
        if flags.global {
            self.perform_all_chained_action(rt, &actions);
            return true;
        }

        // If we are focused, then we process keybinds only if they are
        // app-scoped. Otherwise, we do nothing. Surface-scoped should be
        // processed by Surface.keyEvent. For chained actions, all actions
        // must be app-scoped.
        if actions.iter().any(|a| a.scope() != Scope::App) {
            return false;
        }
        for action in &actions {
            if let Err(err) = self.perform_action(rt, action) {
                log::warn!(
                    "error performing app keybind action action={} err={err}",
                    action.key().name()
                );
            }
        }

        true
    }

    /// Perform a binding action. This only accepts actions that are scoped
    /// to the app. Callers can use [`App::perform_all_action`] to perform
    /// any action and any non-app-scoped actions will be performed on all
    /// surfaces.
    pub fn perform_action(&self, rt: &dyn RtApp, action: &BindingAction) -> Result<(), CoreError> {
        match action {
            BindingAction::Unbind => unreachable!("unbind is never performed"),
            BindingAction::Ignore => {}
            BindingAction::Quit => {
                rt.perform_action(Target::App, RtAction::Quit);
            }
            BindingAction::NewWindow => self.new_window(rt, None),
            BindingAction::OpenConfig => {
                rt.perform_action(Target::App, RtAction::OpenConfig);
            }
            BindingAction::ReloadConfig => {
                rt.perform_action(Target::App, RtAction::ReloadConfig(ReloadConfig::default()));
            }
            BindingAction::CloseAllWindows => {
                rt.perform_action(Target::App, RtAction::CloseAllWindows);
            }
            BindingAction::ToggleQuickTerminal => {
                rt.perform_action(Target::App, RtAction::ToggleQuickTerminal);
            }
            BindingAction::ToggleVisibility => {
                rt.perform_action(Target::App, RtAction::ToggleVisibility);
            }
            BindingAction::CheckForUpdates => {
                rt.perform_action(Target::App, RtAction::CheckForUpdates);
            }
            BindingAction::ShowGtkInspector => {
                rt.perform_action(Target::App, RtAction::ShowGtkInspector);
            }
            BindingAction::Undo => {
                rt.perform_action(Target::App, RtAction::Undo);
            }
            BindingAction::Redo => {
                rt.perform_action(Target::App, RtAction::Redo);
            }
            other => {
                // Not app-scoped; the caller is expected to have checked.
                return Err(format!("not an app-scoped action: {}", other.key().name()).into());
            }
        }
        Ok(())
    }

    /// Performs a chained action. We will continue executing each action
    /// even if there is a failure in a prior action.
    pub fn perform_all_chained_action(&self, rt: &dyn RtApp, actions: &[BindingAction]) {
        for action in actions {
            if let Err(err) = self.perform_all_action(rt, action) {
                log::warn!(
                    "error performing chained action action={} err={err}",
                    action.key().name()
                );
            }
        }
    }

    /// Perform an app-wide binding action. If the action is surface-specific
    /// then it will be performed on all surfaces. To perform only
    /// app-scoped actions, use [`App::perform_action`].
    pub fn perform_all_action(
        &self,
        rt: &dyn RtApp,
        action: &BindingAction,
    ) -> Result<(), CoreError> {
        match action.scope() {
            // App-scoped actions are handled by the app so that they aren't
            // repeated for each surface (since each surface forwards
            // app-scoped actions back up).
            Scope::App => self.perform_action(rt, action),

            // Surface-scoped actions are performed on all surfaces. Errors
            // are logged but processing continues.
            Scope::Surface => {
                for surface in self.state.surfaces() {
                    if let Err(err) = surface.perform_binding_action(action) {
                        log::warn!(
                            "error performing binding action on surface id={} err={err}",
                            surface.id()
                        );
                    }
                }
                Ok(())
            }
        }
    }

    /// Create a new window (Zig: `newWindow`).
    pub fn new_window(&self, rt: &dyn RtApp, parent: Option<SurfaceId>) {
        let target = match parent {
            Some(p) if self.state.has_surface(p) => Target::Surface(p),
            _ => Target::App,
        };

        rt.perform_action(target, RtAction::NewWindow);
    }

    /// Process at most one mailbox capacity per tick, then return to the
    /// runtime so it can dispatch input, timers, and drawing. Producers can
    /// refill the queue continuously under heavy terminal output.
    fn drain_mailbox(&self, rt: &dyn RtApp) {
        for _ in 0..MAILBOX_CAPACITY {
            let Ok(message) = self.state.mailbox_rx.try_recv() else {
                return;
            };
            match message {
                Message::OpenConfig => {
                    let _ = self.perform_action(rt, &BindingAction::OpenConfig);
                }
                Message::NewWindow { parent } => self.new_window(rt, parent),
                Message::Close(id) => self.close_surface(id),
                Message::SurfaceMessage { surface, message } => {
                    self.surface_message(surface, message);
                }
                Message::RedrawSurface(id) => {
                    if self.state.has_surface(id) {
                        rt.perform_action(Target::Surface(id), RtAction::Render);
                    }
                }

                // If we're quitting, then we set the quit flag and stop
                // draining the mailbox immediately. This lets us defer
                // mailbox processing to the next tick so that the apprt can
                // try to quit as quickly as possible.
                Message::Quit => {
                    log::info!("quit message received, short circuiting mailbox drain");
                    let _ = self.perform_action(rt, &BindingAction::Quit);
                    return;
                }
            }
        }

        // Producer wakeups may have coalesced during this tick. Ensure a
        // fresh iteration for deferred messages even if producers stop now.
        if !self.state.mailbox_rx.is_empty() {
            self.state.mailbox.waker.wake();
        }
    }

    /// Close a surface (Zig: `closeSurface`).
    pub fn close_surface(&self, id: SurfaceId) {
        if let Some(surface) = self.state.surface(id) {
            surface.close();
        }
    }

    /// Handle a surface message. Messages for surfaces that are gone are
    /// dropped: the window probably quit before we handled the message.
    fn surface_message(&self, id: SurfaceId, msg: SurfaceMessage) {
        if let Some(surface) = self.state.surface(id)
            && let Err(err) = surface.handle_message(msg)
        {
            log::warn!("error handling surface message id={id} err={err}");
        }
    }

    /// Create a surface and add it to the app (`CoreApp::new_surface` with
    /// the concrete surface type, for runtimes that need more than the
    /// [`CoreSurface`] trait, e.g. the embedded C API).
    pub fn create_surface(
        &self,
        id: SurfaceId,
        config: Config,
        rt_app: Rc<dyn RtApp>,
        rt_surface: Rc<dyn RtSurface>,
    ) -> Result<Rc<Surface>, CoreError> {
        let surface = Surface::new(
            id,
            Rc::clone(&self.state),
            Arc::new(config),
            Rc::clone(&rt_app),
            rt_surface,
        )?;
        self.add_surface(id, &surface, &rt_app);
        Ok(surface)
    }

    /// Add an initialized surface. Since we have non-zero surfaces, the quit
    /// timer is cancelled.
    pub(crate) fn add_surface(&self, id: SurfaceId, surface: &Rc<Surface>, rt: &Rc<dyn RtApp>) {
        *self.state.rt_app.borrow_mut() = Some(Rc::clone(rt));
        self.state
            .surfaces
            .borrow_mut()
            .push((id, Rc::downgrade(surface)));

        // It is up to the apprt if there is a quit timer at all and if it
        // should be canceled.
        rt.perform_action(Target::App, RtAction::QuitTimer(QuitTimer::Stop));
    }
}

impl CoreApp for App {
    fn tick(&self, rt: &dyn RtApp) -> Result<(), CoreError> {
        // Drain our mailbox
        self.drain_mailbox(rt);
        Ok(())
    }

    fn update_config(&self, rt: &dyn RtApp, config: &Arc<Config>) -> Result<(), CoreError> {
        // Go through and update all of the surface configurations.
        for surface in self.state.surfaces() {
            surface.handle_message(SurfaceMessage::ChangeConfig(Arc::clone(config)))?;
        }

        // Apply our conditional state. If we fail to apply the conditional
        // state then we log and attempt to move forward with the old config.
        // We only apply this to the app-level config because the surface
        // config applies its own conditional state.
        let applied =
            match config.change_conditional_state(self.state.config_conditional_state.get()) {
                Ok(Some(c)) => Arc::new(c),
                Ok(None) => Arc::clone(config),
                Err(err) => {
                    log::warn!("failed to apply conditional state to config err={err}");
                    Arc::clone(config)
                }
            };

        // Notify the apprt that the app has changed configuration.
        rt.perform_action(
            Target::App,
            RtAction::ConfigChange(ghostty_apprt::action::ConfigChange { config: applied }),
        );
        Ok(())
    }

    fn needs_confirm_quit(&self) -> bool {
        self.state.surfaces().iter().any(|s| s.needs_confirm_quit())
    }

    fn color_scheme_event(&self, rt: &dyn RtApp, scheme: ColorScheme) {
        let new_scheme = match scheme {
            ColorScheme::Light => conditional::Theme::Light,
            ColorScheme::Dark => conditional::Theme::Dark,
        };

        // If our scheme didn't change, then we don't do anything.
        let mut state = self.state.config_conditional_state.get();
        if state.theme == new_scheme {
            return;
        }

        // Setup our conditional state which has the current color theme.
        state.theme = new_scheme;
        self.state.config_conditional_state.set(state);

        // Request our configuration be reloaded because the new scheme may
        // impact the colors of the app.
        rt.perform_action(
            Target::App,
            RtAction::ReloadConfig(ReloadConfig { soft: true }),
        );
    }

    fn perform_all_action(&self, rt: &dyn RtApp, action: &BindingAction) -> Result<(), CoreError> {
        App::perform_all_action(self, rt, action)
    }

    fn push_message(&self, msg: AppMessage) {
        let m = match msg {
            AppMessage::OpenConfig => Message::OpenConfig,
            AppMessage::NewWindow => Message::NewWindow { parent: None },
            AppMessage::PresentSurface(id) => Message::SurfaceMessage {
                surface: id,
                message: SurfaceMessage::PresentSurface,
            },
        };
        self.state.mailbox.push(m, Timeout::Forever);
    }

    fn new_surface(
        &self,
        id: SurfaceId,
        config: Config,
        rt_app: Rc<dyn RtApp>,
        rt_surface: Rc<dyn RtSurface>,
    ) -> Result<Rc<dyn CoreSurface>, CoreError> {
        Ok(self.create_surface(id, config, rt_app, rt_surface)?)
    }

    fn delete_surface(&self, id: SurfaceId) {
        // If this surface is the focused surface then we need to clear it.
        if self.state.focused_surface.get() == Some(id) {
            self.state.focused_surface.set(None);
        }

        let empty = {
            let mut surfaces = self.state.surfaces.borrow_mut();
            surfaces.retain(|(i, _)| *i != id);
            surfaces.is_empty()
        };

        // If we have no surfaces, we can start the quit timer. It is up to
        // the apprt to determine if this is necessary.
        if empty && let Some(rt) = self.state.rt_app.borrow().clone() {
            rt.perform_action(Target::App, RtAction::QuitTimer(QuitTimer::Start));
        }
    }

    fn surface_ids(&self) -> Vec<SurfaceId> {
        self.state
            .surfaces
            .borrow()
            .iter()
            .map(|(i, _)| *i)
            .collect()
    }

    fn cell_size(&self, config: &Config, font_size: DesiredFontSize) -> Option<RtCellSize> {
        let derived = ghostty_font::shared_grid_set::DerivedConfig::new(config);
        let size = ghostty_font::face::DesiredSize {
            points: font_size.points,
            xdpi: font_size.xdpi,
            ydpi: font_size.ydpi,
        };
        let (key, grid) = self.state.font_grid_set.ref_(&derived, size).ok()?;
        let cell = grid.cell_size();
        self.state.font_grid_set.deref(&key);
        Some(RtCellSize {
            width: cell.width,
            height: cell.height,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct CountingWaker(AtomicUsize);

    impl Waker for CountingWaker {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn closing_scope_releases_all_senders_without_draining_ui_queue() {
        use std::time::Duration;
        let (tx, rx) = crossbeam_channel::bounded(MAILBOX_CAPACITY);
        let mailbox = Mailbox {
            tx,
            waker: Arc::new(CountingWaker::default()),
            cancelled: crossbeam_channel::never(),
        };
        let mut closing = mailbox.scoped();
        let other = mailbox.scoped();
        for _ in 0..MAILBOX_CAPACITY {
            assert_ne!(mailbox.push(Message::Quit, Timeout::Instant), 0);
        }
        let (done_tx, done_rx) = crossbeam_channel::unbounded();
        let workers: Vec<_> = [Timeout::Forever, Timeout::Ns(60_000_000_000)]
            .into_iter()
            .map(|timeout| {
                let producer = closing.mailbox();
                let done = done_tx.clone();
                std::thread::spawn(move || {
                    let result = producer.push(Message::Quit, timeout);
                    done.send(result).unwrap();
                })
            })
            .collect();
        // Neither sender can complete while the UI queue remains full.
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        closing.close();
        closing.close(); // idempotent, including across cloned producers
        for _ in &workers {
            assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(rx.len(), MAILBOX_CAPACITY);
        rx.try_iter().for_each(drop);
        for timeout in [Timeout::Instant, Timeout::Forever, Timeout::Ns(1)] {
            assert_eq!(closing.mailbox().push(Message::Quit, timeout), 0);
        }
        // Closing one surface cannot disable another or the global mailbox.
        assert_ne!(
            other.mailbox().push(Message::OpenConfig, Timeout::Instant),
            0
        );
        assert_ne!(mailbox.push(Message::Quit, Timeout::Instant), 0);
        assert!(matches!(rx.try_recv(), Ok(Message::OpenConfig)));
        assert!(matches!(rx.try_recv(), Ok(Message::Quit)));

        let abandoned = other.mailbox();
        drop(other); // also cancels failed/partial surface initialization
        assert_eq!(abandoned.push(Message::Quit, Timeout::Forever), 0);
    }

    /// Refill the mailbox during each callback, like a renderer producing
    /// redraw requests while the app processes earlier requests.
    struct RefillingRuntime {
        mailbox: Mailbox,
        calls: Cell<usize>,
    }

    impl RtApp for RefillingRuntime {
        fn perform_action(&self, _: Target<SurfaceId>, action: RtAction) -> bool {
            assert!(matches!(action, RtAction::NewWindow));
            let calls = self.calls.get() + 1;
            self.calls.set(calls);
            // Finite even with an unbounded drain, so regressions fail
            // without hanging the test runner.
            if calls < MAILBOX_CAPACITY * 2 {
                self.mailbox
                    .push(Message::NewWindow { parent: None }, Timeout::Instant);
            }
            true
        }
    }

    #[test]
    fn mailbox_tick_yields_and_rearms_when_producer_keeps_refilling() {
        let waker = Arc::new(CountingWaker::default());
        let app = App::new(waker.clone()).unwrap();
        let rt = RefillingRuntime {
            mailbox: app.mailbox(),
            calls: Cell::new(0),
        };
        rt.mailbox
            .push(Message::NewWindow { parent: None }, Timeout::Instant);

        app.tick(&rt).unwrap();
        assert_eq!(rt.calls.get(), MAILBOX_CAPACITY);
        assert_eq!(app.state.mailbox_rx.len(), 1);
        // Initial push, one push per callback, and a wake to resume the
        // pending work after the runtime has handled input and timers.
        assert_eq!(waker.0.load(Ordering::Relaxed), MAILBOX_CAPACITY + 2);

        app.tick(&rt).unwrap();
        assert_eq!(rt.calls.get(), MAILBOX_CAPACITY * 2);
        assert!(app.state.mailbox_rx.is_empty());
        assert_eq!(waker.0.load(Ordering::Relaxed), MAILBOX_CAPACITY * 2 + 1);
    }
}
