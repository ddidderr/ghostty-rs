//! Represents the "writer" thread for terminal IO. The reader side is
//! handled by the Termio struct itself and dependent on the underlying
//! implementation (i.e. if its a pty, manual, etc.) (`termio/Thread.zig`).
//!
//! The writer thread does handle writing bytes to the pty but also handles
//! different events such as starting synchronized output, changing some
//! modes (like linefeed), etc. The goal is to offload as much from the
//! reader thread as possible since it is the hot path in parsing VT
//! sequences and updating terminal state.
//!
//! This thread state can only be used by one thread at a time.
//!
//! # Event loop
//!
//! libxev is replaced by a single `poll(2)` loop over: the mailbox wakeup
//! eventfd, the stop eventfd, the child's pidfd (or the Flatpak exit
//! wakeup), the pty (`POLLOUT`, only while writes are queued) and a set of
//! deadlines (selection scroll, resize coalescing, synchronized-output reset,
//! termios polling). See [`Thread::thread_main`].

use std::io;
use std::time::{Duration, Instant};

use ghostty_apprt::surface::Message as SurfaceMessage;
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_renderer::size::Size;
use ghostty_vt::csi;
use ghostty_vt::modes::Mode;
use rustix::event::{PollFd, PollFlags};

use crate::backend::ThreadData as BackendThreadData;
use crate::exec;
use crate::message::Message;
use crate::state::SharedStateExt;
use crate::termio::{Error, Termio, ThreadData};
use crate::wakeup::Wakeup;

/// The number of milliseconds to coalesce certain messages like resize for.
/// Not all message types are coalesced.
pub const COALESCE_MIN_MS: u64 = 25;

/// The number of milliseconds before we reset the synchronized output flag
/// if the running program hasn't already.
pub const SYNC_RESET_MS: u64 = 1000;

/// The number of milliseconds between each movement during selection scrolling.
pub const SELECTION_SCROLL_MS: u64 = 15;

#[derive(Debug, Default)]
struct Flags {
    /// This is set to true only when an abnormal exit is detected. It
    /// tells our mailbox system to drain and ignore all messages.
    drain: bool,

    /// True if linefeed mode is enabled. This is duplicated here so that the
    /// write thread doesn't need to grab a lock to check this on every write.
    linefeed_mode: bool,

    /// This is true when the inspector is active.
    has_inspector: bool,
}

/// See the module docs.
pub struct Thread {
    /// This can be used to stop the thread on the next loop iteration.
    stop: Wakeup,

    /// This is used for timer-based selection scrolling.
    scroll_active: bool,
    scroll_deadline: Option<Instant>,

    /// This is used to coalesce resize events.
    coalesce_deadline: Option<Instant>,
    coalesce_resize: Option<Size>,

    /// This timer is used to reset synchronized output modes so that
    /// the terminal doesn't freeze with a bad actor.
    sync_reset_deadline: Option<Instant>,

    flags: Flags,
}

impl Thread {
    /// Initialize the thread. This does not START the thread. This only sets
    /// up all the internal state necessary prior to starting the thread. It
    /// is up to the caller to run [`Thread::thread_main`] on a thread.
    pub fn new() -> io::Result<Thread> {
        Ok(Thread {
            stop: Wakeup::new()?,
            scroll_active: false,
            scroll_deadline: None,
            coalesce_deadline: None,
            coalesce_resize: None,
            sync_reset_deadline: None,
            flags: Flags::default(),
        })
    }

    /// A handle that, when notified, stops the thread on the next loop
    /// iteration (the Zig `stop` async).
    pub fn stop_handle(&self) -> Wakeup {
        self.stop.clone()
    }

    /// The main entrypoint for the thread. Consumes the [`Termio`]. Returns
    /// once the stop handle was notified; dropping the termio afterwards
    /// cleans everything up.
    pub fn thread_main(mut self, mut io: Termio) {
        // Call child function so we can use errors...
        let result = self.thread_main_(&mut io);
        if let Err(err) = result {
            log::warn!("error in io thread err={err}");
            report_start_error(&io, &err);

            // If our loop is not stopped, then we need to keep running so that
            // messages are drained and we can wait for the surface to send a stop
            // message.
            log::warn!("abrupt io thread exit detected, draining mailbox until stopped");
            self.flags.drain = true;
            self.drain_until_stop(&io);
            log::debug!("io thread fully exiting after abnormal failure");
        }
    }

    fn thread_main_(&mut self, io: &mut Termio) -> Result<(), Error> {
        // Run our thread start/end callbacks. This allows the implementation
        // to hook into the event loop as needed.
        let mut data = io.thread_enter()?;

        log::debug!("starting IO thread");
        self.run(io, &mut data);
        log::debug!("starting IO thread shutdown");

        io.thread_exit(&mut data);
        log::debug!("IO thread exited");
        Ok(())
    }

    /// After an abnormal failure: discard messages until the stop signal.
    fn drain_until_stop(&mut self, io: &Termio) {
        let mailbox = io.mailbox().clone();
        loop {
            let mut fds = [
                PollFd::from_borrowed_fd(mailbox.wakeup().as_fd(), PollFlags::IN),
                PollFd::from_borrowed_fd(self.stop.as_fd(), PollFlags::IN),
            ];
            match rustix::event::poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(err) => {
                    log::error!("failed to poll for draining err={err}");
                    return;
                }
            }
            let stop = fds[1].revents().contains(PollFlags::IN);
            let msgs = fds[0].revents().contains(PollFlags::IN);
            if msgs {
                mailbox.wakeup().drain();
                while mailbox.pop().is_some() {}
            }
            if stop {
                self.stop.drain();
                return;
            }
        }
    }

    fn next_deadline(&self, td: &ThreadData) -> Option<Instant> {
        [
            self.scroll_deadline,
            self.coalesce_deadline,
            self.sync_reset_deadline,
            match &td.backend {
                BackendThreadData::Exec(e) => e.next_deadline(),
            },
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The event loop. Returns when stopped.
    fn run(&mut self, io: &mut Termio, td: &mut ThreadData) {
        let mailbox = io.mailbox().clone();

        loop {
            let timeout = self.next_deadline(td).map(|d| {
                let dur = d.saturating_duration_since(Instant::now());
                rustix::time::Timespec {
                    tv_sec: dur.as_secs() as _,
                    tv_nsec: i64::from(dur.subsec_nanos()) as _,
                }
            });

            // Build the poll set: fixed entries first, optional ones after.
            let mut fds: Vec<PollFd<'_>> = Vec::with_capacity(5);
            fds.push(PollFd::from_borrowed_fd(
                mailbox.wakeup().as_fd(),
                PollFlags::IN,
            ));
            fds.push(PollFd::from_borrowed_fd(self.stop.as_fd(), PollFlags::IN));
            let exec = td.backend.as_exec();
            let proc_idx = exec.process_fd().map(|fd| {
                fds.push(PollFd::from_borrowed_fd(fd, PollFlags::IN));
                fds.len() - 1
            });
            let fp_idx = exec.flatpak_fd().map(|fd| {
                fds.push(PollFd::from_borrowed_fd(fd, PollFlags::IN));
                fds.len() - 1
            });
            let out_idx = exec.wants_write().then(|| {
                fds.push(PollFd::from_borrowed_fd(exec.pty_fd(), PollFlags::OUT));
                fds.len() - 1
            });

            match rustix::event::poll(&mut fds, timeout.as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(err) => {
                    log::error!("io thread poll failed err={err}");
                    return;
                }
            }
            let rev: Vec<PollFlags> = fds.iter().map(PollFd::revents).collect();
            drop(fds);

            let ready = |i: usize| !rev[i].is_empty();

            // Stop requested.
            if ready(1) {
                self.stop.drain();
                return;
            }

            // The mailbox has messages.
            if ready(0) {
                // When we wake up, we check the mailbox. Mailbox producers
                // should wake up our thread after publishing.
                mailbox.wakeup().drain();
                self.drain_mailbox(io, td);
            }

            if proc_idx.is_some_and(ready) {
                td.backend.as_exec_mut().on_process_ready(None);
            }
            if fp_idx.is_some_and(ready) {
                td.backend.as_exec_mut().on_flatpak_ready();
            }
            if out_idx.is_some_and(ready) {
                td.backend.as_exec_mut().flush_writes();
            }

            self.run_timers(io, td, Instant::now());
        }
    }

    fn run_timers(&mut self, io: &mut Termio, td: &mut ThreadData, now: Instant) {
        td.backend.as_exec_mut().on_timer(now);

        if self.sync_reset_deadline.is_some_and(|d| now >= d) {
            self.sync_reset_deadline = None;
            io.reset_synchronized_output();
        }

        if self.coalesce_deadline.is_some_and(|d| now >= d) {
            self.coalesce_deadline = None;
            if let Some(v) = self.coalesce_resize.take()
                && let Err(err) = io.resize(td, v)
            {
                log::warn!("error during resize err={err}");
            }
        }

        if self.scroll_deadline.is_some_and(|d| now >= d) {
            self.scroll_deadline = None;
            self.selection_scroll_tick(td, now);
        }
    }

    /// Drain the mailbox, handling all the messages in our terminal
    /// implementation.
    fn drain_mailbox(&mut self, io: &mut Termio, td: &mut ThreadData) {
        // If we're draining, we just drain the mailbox and return.
        if self.flags.drain {
            while io.mailbox().pop().is_some() {}
            return;
        }

        let mut redraw = false;
        while let Some(message) = io.mailbox().pop() {
            // If we have a message we always redraw
            redraw = true;

            match message {
                Message::ColorSchemeReport { force } => io.color_scheme_report(td, force),
                Message::Crash => panic!("crash request, crashing intentionally"),
                Message::ChangeConfig(config) => io.change_config(td, *config),
                Message::Inspector(v) => self.flags.has_inspector = v,
                Message::Resize(v) => self.handle_resize(v),
                Message::SizeReport(v) => io.size_report(td, v),
                Message::ClearScreen { history } => io.clear_screen(td, history),
                Message::ScrollViewport(v) => io.scroll_viewport(v),
                Message::SelectionScroll(v) => {
                    if v {
                        self.start_scroll_timer();
                    } else {
                        self.stop_scroll_timer();
                    }
                }
                Message::JumpToPrompt(v) => io.jump_to_prompt(v),
                Message::StartSynchronizedOutput => self.start_synchronized_output(),
                Message::LinefeedMode(v) => self.flags.linefeed_mode = v,
                Message::Focused(v) => io.focus_gained(td, v),
                Message::Write(w) => io.queue_write(td, w.slice(), self.flags.linefeed_mode),
            }
        }

        // Trigger a redraw after we've drained so we don't waste cyces
        // messaging a redraw.
        if redraw && let Err(err) = io.renderer_wakeup().notify() {
            log::warn!("failed to notify renderer err={err}");
        }
    }

    fn start_synchronized_output(&mut self) {
        // (Re)start the timer: xev's `Timer.reset` cancels a running timer.
        self.sync_reset_deadline = Some(Instant::now() + Duration::from_millis(SYNC_RESET_MS));
    }

    fn handle_resize(&mut self, resize: Size) {
        self.coalesce_resize = Some(resize);

        // If the timer is already active we just return. In the future we want
        // to reset the timer up to a maximum wait time but for now this ensures
        // relatively smooth resizing.
        if self.coalesce_deadline.is_some() {
            return;
        }

        self.coalesce_deadline = Some(Instant::now() + Duration::from_millis(COALESCE_MIN_MS));
    }

    fn start_scroll_timer(&mut self) {
        self.scroll_active = true;

        // If it is already active, e.g. start_scroll_timer is called multiple
        // times, then we just return. We can't simply check `scroll_active`
        // because its possible that `stop_scroll_timer` was called but there
        // was no loop tick between then and now to halt out completion.
        if self.scroll_deadline.is_some() {
            return;
        }
        self.scroll_deadline = Some(Instant::now() + Duration::from_millis(SELECTION_SCROLL_MS));
    }

    fn stop_scroll_timer(&mut self) {
        // This will stop the scrolling on the next iteration.
        self.scroll_active = false;
    }

    fn selection_scroll_tick(&mut self, td: &ThreadData, now: Instant) {
        // Send the tick to the main surface
        td.surface_mailbox.push(
            SurfaceMessage::SelectionScrollTick(self.scroll_active),
            Timeout::Instant,
        );

        if self.scroll_active {
            self.scroll_deadline = Some(now + Duration::from_millis(SELECTION_SCROLL_MS));
        }
    }
}

/// If there is an error starting the IO thread, we replace our terminal
/// screen with the error message. It might be better in the future to send
/// the error to the surface thread and let the apprt deal with it
/// in some way but this works for now. Without this, the user would
/// just see a blank terminal window.
fn report_start_error(io: &Termio, err: &Error) {
    let mut t = io.renderer_state().lock_terminal();

    // Hide the cursor
    t.modes.set(Mode::CursorVisible, false);

    let msg: String = match err {
        Error::Exec(exec::Error::Pty(crate::pty::Error::OpenptyFailed)) => "\
Your system cannot allocate any more pty devices.

Ghostty requires a pty device to launch a new terminal.
This error is usually due to having too many terminal
windows open or having another program that is using too
many pty devices.

Please free up some pty devices and try again."
            .to_owned(),

        Error::InputNotFound | Error::InputFailed => "\
A configured `input` path was not found, was not readable,
was too large, or the underlying pty failed to accept
the write.

Ghostty can't continue since it can't guarantee that
initial terminal state will be as desired. Please review
the value of `input` in your configuration file and
ensure that all the path values exist and are readable."
            .to_owned(),

        err => format!(
            "\
error starting IO thread: {err}

The underlying shell or command was unable to be started.
This error is usually due to exhausting a system resource.
If this looks like a bug, please report it.

This terminal is non-functional. Please close it and try again."
        ),
    };

    t.erase_display(csi::EraseDisplay::Complete, false);
    let _ = t.print_string(&msg);
}
