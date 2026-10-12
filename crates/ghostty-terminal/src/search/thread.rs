//! Search thread that handles searching a terminal for a string match.
//! This is expected to run on a dedicated thread to try to prevent too much
//! overhead to other terminal read/write operations.
//!
//! The current architecture of search does acquire global locks for accessing
//! terminal data, so there's still added contention, but we do our best to
//! minimize this by trading off memory usage (copying data to minimize lock
//! time).
//!
//! Deviation: Zig drives this from a libxev loop (async wakeup/stop handles,
//! a refresh timer). Here the loop is built from `crossbeam-channel`: a
//! bounded mailbox, a control channel for wakeup/stop, and a deadline
//! based refresh timer. The terminal and its mutex are combined into an
//! `Arc<Mutex<Terminal>>`.

use std::{
    ops::DerefMut,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, TrySendError};

use super::{
    screen::{ScreenSearch, Select, TickError},
    viewport::ViewportSearch,
};
use crate::{
    highlight::{Flattened, Untracked},
    page_list::{Chunk, Direction as ListDirection},
    point::{Coordinate, Point},
    screen_set::Key as ScreenKey,
    terminal::Terminal,
};

// TODO: Some stuff that could be improved:
// - pause the refresh timer when the terminal isn't focused
// - we probably want to know our progress through the search
//   for viewport matches so we can show n/total UI.
// - notifications should be coalesced to avoid spamming a massive
//   amount of events if the terminal is changing rapidly.

/// The interval at which we refresh the terminal state to check if
/// there are any changes that require us to re-search. This should be
/// balanced to be fast enough to be responsive but not so fast that
/// we hold the terminal lock too often.
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(24); // 40 FPS

/// Capacity of the mailbox (Zig: `BlockingQueue(Message, 64)`).
pub const MAILBOX_CAPACITY: usize = 64;

/// Callback for events from the search thread.
pub type EventCallback = Arc<dyn Fn(Event) + Send + Sync>;

/// The options used to initialize the thread.
#[derive(Clone)]
pub struct Options {
    /// The terminal data to search. The mutex must be held while
    /// reading/writing the terminal.
    pub terminal: Arc<dyn TerminalLock>,

    /// The callback for events from the search thread. This can be `None`
    /// if you don't want to receive events, which could be useful for a
    /// one-time search (although, odd, you should use our search
    /// structures directly then).
    pub event_cb: Option<EventCallback>,
}

/// The messages that can be sent to the thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Change the search term. If no prior search term is given this
    /// will start a search. If an existing search term is given this will
    /// stop the prior search and start a new one.
    ChangeNeedle(Vec<u8>),

    /// Select a search result.
    Select(Select),
}

/// Events that can be emitted from the search thread. The caller
/// chooses to handle these as they see fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Search is quitting. The search thread is exiting.
    Quit,

    /// Search is complete for the given needle on all screens.
    Complete,

    /// Total matches on the current active screen have changed.
    TotalMatches(usize),

    /// Selected match changed.
    SelectedMatch(Option<SelectedMatch>),

    /// Matches in the viewport have changed.
    ViewportMatches(Vec<Flattened>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMatch {
    pub idx: usize,
    pub highlight: Flattened,
}

#[derive(Debug)]
enum Ctl {
    Wakeup,
    Stop,
}

/// A cloneable handle used to talk to the thread from other threads
/// (Zig: `thread.mailbox`, `thread.wakeup`, `thread.stop`).
#[derive(Clone)]
pub struct Handle {
    mailbox: Sender<Message>,
    ctl: Sender<Ctl>,
}

impl Handle {
    /// Push a message to the mailbox, blocking while it is full (Zig:
    /// `mailbox.push(msg, .forever)`), and wake the thread. Returns false
    /// if the thread is gone.
    pub fn send(&self, msg: Message) -> bool {
        if self.mailbox.send(msg).is_err() {
            return false;
        }
        self.wakeup();
        true
    }

    /// Push a message without blocking. Returns false if the mailbox is
    /// full or the thread is gone.
    pub fn try_send(&self, msg: Message) -> bool {
        match self.mailbox.try_send(msg) {
            Ok(()) => {
                self.wakeup();
                true
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        }
    }

    /// Wake up the thread so it drains its mailbox.
    pub fn wakeup(&self) {
        let _ = self.ctl.try_send(Ctl::Wakeup);
    }

    /// Stop the thread on the next loop iteration.
    pub fn stop(&self) {
        let _ = self.ctl.send(Ctl::Stop);
    }
}

/// A locked terminal (any guard that derefs to a [`Terminal`]).
pub trait TerminalRef: DerefMut<Target = Terminal> {}
impl<T: DerefMut<Target = Terminal>> TerminalRef for T {}

/// Something that owns a terminal behind a lock. Implemented for
/// `Mutex<Terminal>`; the core implements it for the shared renderer state
/// (where the terminal lives next to preedit/mouse state).
pub trait TerminalLock: Send + Sync {
    /// Lock and borrow the terminal (poisoning is ignored).
    fn lock_terminal(&self) -> Box<dyn TerminalRef + '_>;
}

impl TerminalLock for Mutex<Terminal> {
    fn lock_terminal(&self) -> Box<dyn TerminalRef + '_> {
        Box::new(self.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

fn lock(m: &Arc<dyn TerminalLock>) -> Box<dyn TerminalRef + '_> {
    m.lock_terminal()
}

/// The search thread state.
pub struct Thread {
    opts: Options,
    mailbox_tx: Sender<Message>,
    mailbox_rx: Receiver<Message>,
    ctl_tx: Sender<Ctl>,
    ctl_rx: Receiver<Ctl>,

    /// Search state. Starts as `None` and is populated when a search is
    /// started (a needle is given).
    search: Option<Search>,

    stopped: bool,
    next_refresh: Instant,
}

impl Thread {
    /// Initialize the thread. This does not START the thread. This only sets
    /// up all the internal state necessary prior to starting the thread. It
    /// is up to the caller to run `thread_main` on a dedicated thread (or
    /// use `spawn`).
    pub fn new(opts: Options) -> Self {
        let (mailbox_tx, mailbox_rx) = crossbeam_channel::bounded(MAILBOX_CAPACITY);
        let (ctl_tx, ctl_rx) = crossbeam_channel::unbounded();
        Self {
            opts,
            mailbox_tx,
            mailbox_rx,
            ctl_tx,
            ctl_rx,
            search: None,
            stopped: false,
            next_refresh: Instant::now() + REFRESH_INTERVAL,
        }
    }

    /// A handle to message/stop this thread.
    pub fn handle(&self) -> Handle {
        Handle {
            mailbox: self.mailbox_tx.clone(),
            ctl: self.ctl_tx.clone(),
        }
    }

    /// Create the thread state and run it on a new OS thread named
    /// "search". Returns the handle and the join handle.
    pub fn spawn(opts: Options) -> std::io::Result<(Handle, std::thread::JoinHandle<()>)> {
        let thread = Thread::new(opts);
        let handle = thread.handle();
        let join = std::thread::Builder::new()
            .name("search".into())
            .spawn(move || thread.thread_main())?;
        Ok((handle, join))
    }

    /// The main entrypoint for the thread.
    pub fn thread_main(mut self) {
        log::debug!("starting search thread");

        // Drain the mailbox immediately (Zig sends an initial wakeup).
        self.drain_mailbox();
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;

        // Unlike some of our other threads, we interleave search work
        // with our loop so that we can try to make forward search progress
        // while also listening for messages.
        loop {
            // If our loop is canceled then we drain our messages and quit.
            if self.stopped {
                while let Ok(message) = self.mailbox_rx.try_recv() {
                    log::debug!("mailbox message ignored during shutdown={message:?}");
                }
                break;
            }

            let Some(s) = self.search.as_mut() else {
                // If we're not actively searching, we can block the loop
                // until it does some work.
                self.run_once();
                continue;
            };

            // If we have an active search, we always send any pending
            // notifications. Even if the search is complete, there may be
            // notifications to send.
            if let Some(cb) = &self.opts.event_cb {
                s.notify(&self.opts.terminal, cb);
            }

            if s.is_complete() {
                // If our search is complete, there's no more work to do, we
                // can block until we have an event.
                self.run_once();
                continue;
            }

            // Tick the search. This will trigger any event callbacks, lock
            // for data loading, etc.
            match s.tick() {
                // We're complete now when we were not before. Notify!
                Tick::Complete => {}

                // Forward progress was made.
                Tick::Progress => {}

                // All searches are blocked. Let's grab the lock and feed data.
                Tick::Blocked => {
                    let mut t = lock(&self.opts.terminal);
                    s.feed(&mut t);
                }
            }

            // We have an active search, so we only want to process messages
            // we have but otherwise return immediately so we can continue the
            // search.
            self.run_no_wait();
        }

        log::debug!("starting search thread shutdown");
        if let Some(mut s) = self.search.take() {
            let mut t = lock(&self.opts.terminal);
            s.deinit(&mut t);
        }

        // Send the quit message
        if let Some(cb) = &self.opts.event_cb {
            cb(Event::Quit);
        }
        log::debug!("search thread exited");
    }

    /// Block until something happens (message, control event, or the
    /// refresh timer firing) and process it.
    fn run_once(&mut self) {
        let timeout = self.next_refresh.saturating_duration_since(Instant::now());
        crossbeam_channel::select! {
            recv(self.mailbox_rx) -> msg => {
                if let Ok(msg) = msg {
                    self.handle_message(msg);
                }
            }
            recv(self.ctl_rx) -> ctl => {
                match ctl {
                    Ok(Ctl::Stop) | Err(_) => self.stopped = true,
                    Ok(Ctl::Wakeup) => {}
                }
            }
            default(timeout) => {}
        }
        self.run_no_wait();
    }

    /// Process anything already pending without blocking.
    fn run_no_wait(&mut self) {
        while let Ok(ctl) = self.ctl_rx.try_recv() {
            match ctl {
                Ctl::Stop => self.stopped = true,
                Ctl::Wakeup => {}
            }
        }
        if self.stopped {
            return;
        }
        self.drain_mailbox();
        if Instant::now() >= self.next_refresh {
            self.refresh();
        }
    }

    /// Drain the mailbox.
    fn drain_mailbox(&mut self) {
        while let Ok(message) = self.mailbox_rx.try_recv() {
            self.handle_message(message);
        }
    }

    fn handle_message(&mut self, message: Message) {
        log::debug!("mailbox message={message:?}");
        match message {
            Message::ChangeNeedle(v) => self.change_needle(&v),
            Message::Select(v) => self.select(v),
        }
    }

    /// The refresh timer callback.
    fn refresh(&mut self) {
        // Run our feed if we have a search active.
        if let Some(s) = self.search.as_mut() {
            let mut t = lock(&self.opts.terminal);
            s.feed(&mut t);
        }
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
    }

    fn select(&mut self, sel: Select) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        let key = s.last_screen.key;
        let Some(screen_search) = s.screens[key_index(key)].as_mut() else {
            return;
        };

        let mut t = lock(&self.opts.terminal);
        let Some(screen) = t.screens.get_mut(key) else {
            return;
        };

        // Make the selection. Ignore the result because we don't
        // care if the selection didn't change.
        if let Err(e) = screen_search.select(screen, sel) {
            log::warn!("error selecting search match err={e:?}");
            return;
        }

        // Grab our match if we have one. If we don't have a selection
        // then we do nothing.
        let Some(flattened) = screen_search.selected_match() else {
            return;
        };

        // No matter what we reset our selected match cache. This will
        // trigger a callback which will trigger the renderer to wake up
        // so it can be notified the screen scrolled.
        s.last_screen.selected = None;

        // Grab the viewport. Viewports and selections are usually small
        // so this check isn't very expensive, despite appearing O(N^2),
        // both Ns are usually equal to 1.
        let overlaps = {
            let it = screen.pages.page_iterator(
                ListDirection::RightDown,
                Point::Viewport(Coordinate { x: 0, y: 0 }),
                None,
            );
            let mut overlaps = false;
            'outer: for chunk in it {
                for hl_chunk in &flattened.chunks {
                    if chunk.overlaps(Chunk {
                        node: hl_chunk.node,
                        start: hl_chunk.start,
                        end: hl_chunk.end,
                    }) {
                        overlaps = true;
                        break 'outer;
                    }
                }
            }
            overlaps
        };
        if overlaps {
            return;
        }

        screen.scroll(crate::screen::Scroll::Pin(flattened.start_pin()));
    }

    /// Change the search term to the given value.
    fn change_needle(&mut self, needle: &[u8]) {
        log::debug!(
            "changing search needle to '{}'",
            String::from_utf8_lossy(needle)
        );

        // Stop the previous search
        if let Some(s) = self.search.as_mut() {
            // If our search is unchanged, do nothing.
            if s.viewport.needle().eq_ignore_ascii_case(needle) {
                return;
            }

            {
                let mut t = lock(&self.opts.terminal);
                s.deinit(&mut t);
            }
            self.search = None;

            // When the search changes then we need to emit that it stopped.
            if let Some(cb) = &self.opts.event_cb {
                cb(Event::TotalMatches(0));
                cb(Event::SelectedMatch(None));
                cb(Event::ViewportMatches(Vec::new()));
            }
        }

        // No needle means stop the search.
        if needle.is_empty() {
            return;
        }

        // Setup our search state.
        let mut search = Search::new(needle);

        // We need to grab the terminal lock and do an initial feed.
        {
            let mut t = lock(&self.opts.terminal);
            search.feed(&mut t);
        }
        self.search = Some(search);
    }
}

fn key_index(key: ScreenKey) -> usize {
    match key {
        ScreenKey::Primary => 0,
        ScreenKey::Alternate => 1,
    }
}

const KEYS: [ScreenKey; 2] = [ScreenKey::Primary, ScreenKey::Alternate];

/// Search state.
struct Search {
    /// Active viewport search for the active screen.
    viewport: ViewportSearch,

    /// The searchers for all the screens (indexed by screen key order).
    screens: [Option<ScreenSearch>; 2],

    /// All state related to screen switches, collected so that when
    /// we switch screens it makes everything related stale, too.
    last_screen: ScreenState,

    /// True if we sent the complete notification yet.
    last_complete: bool,

    /// The last viewport matches we found.
    stale_viewport_matches: bool,
}

struct ScreenState {
    /// Last active screen key
    key: ScreenKey,

    /// Last notified total matches count
    total: Option<usize>,

    /// Last notified selected match index
    selected: Option<ScreenStateSelected>,
}

struct ScreenStateSelected {
    idx: usize,
    highlight: Untracked,
}

impl ScreenState {
    fn new(key: ScreenKey) -> Self {
        Self {
            key,
            total: None,
            selected: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tick {
    /// All searches are complete.
    Complete,

    /// Progress was made on at least one screen.
    Progress,

    /// All incomplete searches are blocked on feed.
    Blocked,
}

impl Search {
    fn new(needle: &[u8]) -> Self {
        let mut vp = ViewportSearch::new(needle);

        // We use dirty tracking for active area changes. Start with it
        // dirty so the first change is re-searched.
        vp.active_dirty = Some(true);

        Self {
            viewport: vp,
            screens: [None, None],
            last_screen: ScreenState::new(ScreenKey::Primary),
            last_complete: false,
            stale_viewport_matches: true,
        }
    }

    /// Release tracked pins. Requires the terminal (and therefore its lock).
    fn deinit(&mut self, t: &mut Terminal) {
        for key in KEYS {
            if let Some(mut ss) = self.screens[key_index(key)].take()
                && let Some(screen) = t.screens.get_mut(key)
                && ss.is_for_screen(screen)
            {
                ss.deinit(screen);
            }
        }
    }

    /// Returns true if all searches on all screens are complete.
    fn is_complete(&self) -> bool {
        self.screens.iter().flatten().all(|s| s.state.is_complete())
    }

    /// Tick the search forward as much as possible without acquiring
    /// the big lock. Returns the overall tick progress.
    fn tick(&mut self) -> Tick {
        let mut result = Tick::Complete;
        for (i, ss) in self.screens.iter_mut().enumerate() {
            let Some(ss) = ss else { continue };
            match ss.tick() {
                Ok(()) => result = Tick::Progress,

                // Ignore... nothing we can do.
                Err(TickError::Page(e)) => {
                    log::warn!("error ticking screen search key={:?} err={e:?}", KEYS[i]);
                }

                // Ignore, good for us. State remains whatever it is.
                Err(TickError::SearchComplete) => {}

                // Ignore, too, progressed
                Err(TickError::FeedRequired) => {
                    // If we think we're complete, we're not because we're
                    // blocked now (nothing made progress). If we made some
                    // progress, we remain in progress since blocked means
                    // no progress at all.
                    if result == Tick::Complete {
                        result = Tick::Blocked;
                    }
                }
            }
        }

        result
    }

    /// Update any state that requires the terminal lock, such as
    /// feeding additional data to the searches or updating the active screen.
    fn feed(&mut self, t: &mut Terminal) {
        // Update our active screen
        if t.screens.active_key != self.last_screen.key {
            // The default values will force resets of a bunch of other
            // state too to force recalculations and notifications.
            self.last_screen = ScreenState::new(t.screens.active_key);
        }

        // Reconcile our screens with the terminal screens. Remove
        // searchers for screens that no longer exist and add searchers
        // for screens that do exist but we don't have yet.
        for key in KEYS {
            let i = key_index(key);
            let remove = match (&self.screens[i], t.screens.get(key)) {
                (None, _) => false,
                // If the screen doesn't exist at all, remove it.
                (Some(_), None) => true,
                // If the screen pointer changed, remove it, the screen
                // was totally reinitialized.
                (Some(ss), Some(actual)) => !ss.is_for_screen(actual),
            };
            if remove {
                // The old screen (and its tracked pins) is gone, so there
                // is nothing to untrack.
                self.screens[i] = None;
            }
        }
        for key in KEYS {
            let i = key_index(key);
            if self.screens[i].is_some() {
                continue;
            }
            let Some(screen) = t.screens.get_mut(key) else {
                continue;
            };
            match ScreenSearch::new(screen, self.viewport.needle()) {
                Ok(ss) => self.screens[i] = Some(ss),
                Err(e) => {
                    // OOM is probably going to sink the entire ship but
                    // we can just ignore it and wait on the next
                    // reconciliation to try again.
                    log::warn!("error initializing screen search for key={key:?} err={e:?}");
                }
            }
        }

        // See the `search_viewport_dirty` flag on the terminal to know
        // what exactly this is for. But, if this is set, we know the renderer
        // found the viewport/active area dirty, so we should mark it as
        // dirty in our viewport searcher so it forces a re-search.
        if t.flags.search_viewport_dirty {
            t.flags.search_viewport_dirty = false;

            // Mark our viewport dirty so it researches the active
            self.viewport.active_dirty = Some(true);

            // Reload our active area for our active screen
            let key = t.screens.active_key;
            if let Some(ss) = self.screens[key_index(key)].as_mut()
                && let Err(e) = ss.reload_active(t.screens.active_mut())
            {
                log::warn!("error reloading active area for screen key={key:?} err={e:?}");
            }
        }

        // Check our viewport for changes.
        if self.viewport.update(&t.screens.active().pages) {
            self.stale_viewport_matches = true;
        }

        // Feed data
        for key in KEYS {
            let i = key_index(key);
            let Some(ss) = self.screens[i].as_mut() else {
                continue;
            };
            if ss.state.needs_feed()
                && let Some(screen) = t.screens.get_mut(key)
                && let Err(e) = ss.feed(screen)
            {
                log::warn!("error feeding screen search key={key:?} err={e:?}");
            }
        }
    }

    /// Notify about any changes to the search state.
    ///
    /// This doesn't require any locking as it only reads internal state
    /// (the terminal mutex is only passed through for API symmetry and is
    /// not locked).
    fn notify(&mut self, _terminal: &Arc<dyn TerminalLock>, cb: &EventCallback) {
        let Some(screen_search) = self.screens[key_index(self.last_screen.key)].as_ref() else {
            return;
        };

        // Check our total match data
        let total = screen_search.matches_len();
        if Some(total) != self.last_screen.total {
            log::debug!("notifying total matches={total}");
            self.last_screen.total = Some(total);
            cb(Event::TotalMatches(total));
        }

        // Check our viewport matches. If they're stale, we do the
        // viewport search now. We do this as part of notify and not
        // tick because the viewport search is very fast and doesn't
        // require ticked progress or feeds.
        if self.stale_viewport_matches {
            // We always make stale as false. Even if we fail below
            // we require a re-feed to re-search the viewport. The feed
            // process will make it stale again.
            self.stale_viewport_matches = false;

            let mut results: Vec<Flattened> = Vec::new();
            while let Some(hl) = self.viewport.next() {
                results.push(hl);
            }

            log::debug!("notifying viewport matches len={}", results.len());
            cb(Event::ViewportMatches(results));
        }

        // Check our last selected match data.
        let flattened = screen_search
            .selected
            .as_ref()
            .and_then(|m| screen_search.selected_match().map(|f| (m.idx, f)));
        if let Some((idx, flattened)) = flattened {
            let untracked = flattened.untracked();
            if let Some(prev) = &self.last_screen.selected
                && prev.idx == idx
                && prev.highlight.eql(untracked)
            {
                // Same selection, don't update it.
            } else {
                // New selection, notify!
                self.last_screen.selected = Some(ScreenStateSelected {
                    idx,
                    highlight: untracked,
                });

                log::debug!("notifying selection updated idx={idx}");
                cb(Event::SelectedMatch(Some(SelectedMatch {
                    idx,
                    highlight: flattened,
                })));
            }
        } else if self.last_screen.selected.is_some() {
            log::debug!("notifying selection cleared");
            self.last_screen.selected = None;
            cb(Event::SelectedMatch(None));
        }

        // Send our complete notification if we just completed.
        if !self.last_complete && self.is_complete() {
            log::debug!("notifying search complete");
            self.last_complete = true;
            cb(Event::Complete);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{super::test_util::*, *};
    use crate::point::Tag;

    #[derive(Default)]
    struct TestUserData {
        total: usize,
        selected: Option<SelectedMatch>,
        viewport: Vec<Flattened>,
    }

    #[test]
    fn search_thread_basic() {
        let mut t = terminal(20, 2);
        feed(&mut t, b"Hello, world");
        let terminal = Arc::new(Mutex::new(t));

        let ud = Arc::new(Mutex::new(TestUserData::default()));
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let done_tx = Mutex::new(done_tx);
        let ud_cb = Arc::clone(&ud);
        let cb: EventCallback = Arc::new(move |event| {
            let mut ud = ud_cb.lock().unwrap();
            match event {
                Event::Quit => {}
                Event::Complete => {
                    let _ = done_tx.lock().unwrap().send(());
                }
                Event::TotalMatches(v) => ud.total = v,
                Event::SelectedMatch(v) => ud.selected = v,
                Event::ViewportMatches(v) => ud.viewport = v,
            }
        });

        let (handle, join) = Thread::spawn(Options {
            terminal: terminal.clone(),
            event_cb: Some(cb),
        })
        .unwrap();

        // Start our search
        assert!(handle.send(Message::ChangeNeedle(b"world".to_vec())));

        // Wait for completion
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // Stop the thread
        handle.stop();
        join.join().unwrap();

        // 1 total matches
        let ud = ud.lock().unwrap();
        assert_eq!(1, ud.total);
        assert_eq!(1, ud.viewport.len());
        let sel = ud.viewport[0].untracked();
        let t = terminal.lock().unwrap();
        assert_eq!(
            t.screen().pages.point_from_pin(Tag::Screen, sel.start),
            Some(screen_pt(7, 0))
        );
        assert_eq!(
            t.screen().pages.point_from_pin(Tag::Screen, sel.end),
            Some(screen_pt(11, 0))
        );
    }

    #[test]
    fn search_thread_select_and_state() {
        // Extra coverage: selection round trip through the mailbox.
        let mut t = terminal(20, 2);
        feed(&mut t, b"foo\r\nbar foo\r\nfoo");
        let terminal = Arc::new(Mutex::new(t));

        let ud = Arc::new(Mutex::new(TestUserData::default()));
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let done_tx = Mutex::new(done_tx);
        let ud_cb = Arc::clone(&ud);
        let cb: EventCallback = Arc::new(move |event| {
            let mut ud = ud_cb.lock().unwrap();
            match event {
                Event::Complete => {
                    let _ = done_tx.lock().unwrap().send(());
                }
                Event::TotalMatches(v) => ud.total = v,
                Event::SelectedMatch(v) => {
                    ud.selected = v;
                    let _ = done_tx.lock().unwrap().send(());
                }
                _ => {}
            }
        });

        let (handle, join) = Thread::spawn(Options {
            terminal: terminal.clone(),
            event_cb: Some(cb),
        })
        .unwrap();
        handle.send(Message::ChangeNeedle(b"FOO".to_vec()));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.send(Message::Select(Select::Next));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // Clearing the needle stops the search and resets state.
        handle.send(Message::ChangeNeedle(Vec::new()));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.stop();
        join.join().unwrap();

        let ud = ud.lock().unwrap();
        assert_eq!(0, ud.total);
        assert!(ud.selected.is_none());
    }
}
