//! Exec implements the logic for starting and stopping a subprocess with a
//! pty as well as spinning up the necessary read thread to read from the
//! pty and forward it to the Termio instance (`termio/Exec.zig`).
//!
//! # Event loop mapping
//!
//! libxev completions (process watcher, write stream, termios timer) are
//! replaced by state in [`ThreadData`] that the IO thread's `poll(2)` loop
//! (see [`crate::thread`]) consults:
//!
//! * process exit: a `pidfd` (or, if `pidfd_open` is unavailable, a 50ms
//!   `waitpid(WNOHANG)` poll) is polled for readability; Flatpak host
//!   commands report through a channel + wakeup instead,
//! * pty writes: a queue of byte chunks flushed with non-blocking `write`;
//!   `POLLOUT` is polled only while the queue is non-empty,
//! * termios timer: a plain deadline.

use std::{
    collections::VecDeque,
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::Receiver;
use ghostty_apprt::surface::{ChildExited, Message as SurfaceMessage};
use ghostty_config::{
    command::Command as ConfigCommand,
    repeatable_string_map::RepeatableStringMap,
    types_b::{ShellIntegration, ShellIntegrationFeatures},
};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_os::file::Rlimit;
use ghostty_renderer::size::{GridSize, ScreenSize};
use ghostty_terminal::terminal::Terminal;
use rustix::{
    event::{PollFd, PollFlags},
    process::{Pid, PidfdFlags, Signal, WaitOptions},
};

use crate::{
    command::{
        self,
        Command,
        Exit,
        PostForkError,
        RtPostForkInfo,
        RtPreExecInfo,
        linux_cgroup_post_fork,
        linux_cgroup_pre_exec,
    },
    pty::{self, Mode, Pty, Winsize},
    shell_integration::{self, EnvMap, Shell},
    sinks::SurfaceMailbox,
    state::{SharedState, SharedStateExt},
    termio::Output,
    wakeup::Wakeup,
};

/// The termios poll rate in milliseconds.
pub const TERMIOS_POLL_MS: u64 = 200;

/// Poll interval of the `waitpid` fallback process watcher.
const PROCESS_POLL_MS: u64 = 50;

/// `build_config.version_string`.
pub use crate::stream_handler::VERSION_STRING;

/// Errors from starting and running the exec backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Pty(#[from] pty::Error),
    #[error(transparent)]
    Start(#[from] command::StartError),
    #[error(transparent)]
    ShellIntegration(#[from] shell_integration::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("os error: {0}")]
    Errno(#[from] rustix::io::Errno),
    #[error("flatpak host command failed: {0}")]
    Flatpak(ghostty_os::flatpak::Error),
    #[error("process not started")]
    ProcessNotStarted,
    #[error("process has no pid")]
    ProcessNoPid,
    #[error("failed to kill the process group")]
    KillFailed,
}

/// The function that moves a new child into a transient systemd scope
/// (`cgroup.createScope`): `(pid, memory_high, tasks_max)`. Returns `None`
/// if there is no D-Bus connection and `Some(Err(msg))` on failure.
/// TODO(integration): provided by ghostty-apprt-gtk (it owns the D-Bus
/// connection).
pub type CreateScopeFn =
    Arc<dyn Fn(u32, Option<u64>, Option<u64>) -> Option<Result<(), String>> + Send + Sync>;

/// Configuration for the exec backend (`Exec.Config`).
pub struct Config {
    pub command: Option<ConfigCommand>,
    pub env: EnvMap,
    pub env_override: RepeatableStringMap,
    pub shell_integration: ShellIntegration,
    pub shell_integration_features: ShellIntegrationFeatures,
    pub cursor_blink: Option<bool>,
    pub working_directory: Option<String>,
    pub resources_dir: Option<String>,
    pub term: String,

    pub rt_pre_exec_info: RtPreExecInfo,
    pub rt_post_fork_info: RtPostForkInfo,

    /// See [`CreateScopeFn`]. Only used if `rt_post_fork_info` asks for a
    /// cgroup transition.
    pub create_scope: Option<CreateScopeFn>,

    /// The rlimit to restore in the child (Zig: `global_state.rlimits`).
    pub restore_rlimit: Option<Rlimit>,
}

impl Config {
    /// A config with Zig-struct-style defaults for everything optional.
    pub fn new(env: EnvMap, term: impl Into<String>) -> Config {
        Config {
            command: None,
            env,
            env_override: RepeatableStringMap::default(),
            shell_integration: ShellIntegration::Detect,
            shell_integration_features: ShellIntegrationFeatures::default(),
            cursor_blink: None,
            working_directory: None,
            resources_dir: None,
            term: term.into(),
            rt_pre_exec_info: RtPreExecInfo::default(),
            rt_post_fork_info: RtPostForkInfo::default(),
            create_scope: None,
            restore_rlimit: None,
        }
    }
}

/// The exec backend. Owns the (not yet started) subprocess state.
pub struct Exec {
    subprocess: Subprocess,
}

impl Exec {
    /// Initialize the exec state. This will NOT start it, this only sets
    /// up the internal state necessary to start it later.
    pub fn new(cfg: Config) -> Result<Exec, Error> {
        Ok(Exec {
            subprocess: Subprocess::new(cfg)?,
        })
    }

    /// The working directory the subprocess will be started in, if any.
    pub fn cwd(&self) -> Option<&str> {
        self.subprocess.cwd.as_deref()
    }

    /// The argv that will be executed.
    pub fn args(&self) -> &[String] {
        &self.subprocess.args
    }

    /// The environment the subprocess will get (before it is started).
    pub fn env(&self) -> Option<&EnvMap> {
        self.subprocess.env.as_ref()
    }

    /// Call to initialize the terminal state as necessary for this backend.
    /// This is called before any termio begins. This should not be called
    /// after termio begins because it may put the internal terminal state
    /// into a bad state.
    pub fn init_terminal(&mut self, term: &mut Terminal) {
        // If we have an initial pwd requested by the subprocess, then we
        // set that on the terminal now. This allows rapidly initializing
        // new surfaces to use the proper pwd.
        if let Some(cwd) = &self.subprocess.cwd
            && let Err(err) = term.set_pwd(cwd.as_bytes())
        {
            log::warn!("error setting initial pwd err={err:?}");
        }

        // Setup our initial grid/screen size from the terminal. This
        // can't fail because the pty should not exist at this point.
        let _ = self.resize(
            GridSize {
                columns: term.cols,
                rows: term.rows,
            },
            ScreenSize {
                width: term.width_px,
                height: term.height_px,
            },
        );
    }

    /// Start the subprocess and the read thread. `output` is where the read
    /// thread delivers pty output.
    pub fn thread_enter(
        &mut self,
        output: Arc<Output>,
        surface_mailbox: Arc<dyn SurfaceMailbox>,
        renderer_state: SharedState,
    ) -> Result<ThreadData, Error> {
        // Start our subprocess
        let (read_fd, write_fd) = self.subprocess.start()?;

        // Watcher to detect subprocess exit
        let (watch, flatpak_events) = match self.subprocess.process.as_mut() {
            Some(Process::ForkExec(cmd)) => {
                let pid = cmd.pid.ok_or(Error::ProcessNoPid)?;
                (ProcessWatch::for_pid(pid), None)
            }

            // If we're executing via Flatpak then we can't do
            // traditional process watching (its implemented
            // as a special case in os/flatpak) since the
            // command is on the host.
            Some(Process::Flatpak(fp)) => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let wakeup = Wakeup::new()?;
                if let Some(cmd) = fp.cmd.take() {
                    // Keep a signal handle so stop() can still SIGHUP the
                    // host process group after the watcher owns the command.
                    fp.signal = Some(cmd.signal_handle());
                    let w = wakeup.clone();
                    cmd.wait_callback(move |r| {
                        let _ = tx.send(r.map_err(|e| format!("{e}")));
                        let _ = w.notify();
                    })?;
                }
                (ProcessWatch::Flatpak, Some(FlatpakEvents { rx, wakeup }))
            }

            None => return Err(Error::ProcessNotStarted),
        };

        // Track our process start time for abnormal exits
        let start = Instant::now();

        // Create our pipe that we'll use to kill our read thread.
        // pipe.0 is the read end, pipe.1 is the write end.
        let (quit_read, quit_write) = ghostty_os::pipe::pipe()?;

        // The pty fd is shared (same open file description) with the read
        // thread, which sets it to non-blocking; we need non-blocking writes
        // too, so set it before the thread starts.
        set_nonblocking(&read_fd);

        // Start our read thread
        let read_thread = std::thread::Builder::new()
            .name("io-reader".into())
            .spawn(move || read_thread_main(&read_fd, &output, &quit_read))?;

        Ok(ThreadData {
            start,
            exited: false,
            pty_fd: write_fd,
            write_queue: VecDeque::new(),
            write_offset: 0,
            watch,
            flatpak_events,
            read_thread: Some(read_thread),
            read_thread_pipe: Some(quit_write),
            // Start our termios timer. This is used to detect things such
            // as when password input is being done so we can render the
            // terminal in a different way.
            termios_deadline: Some(Instant::now() + Duration::from_millis(TERMIOS_POLL_MS)),
            termios_timer_running: true,
            termios_mode: Mode::default(),
            surface_mailbox,
            renderer_state,
        })
    }

    pub fn thread_exit(&mut self, td: &mut ThreadData) {
        if td.exited {
            self.subprocess.external_exit();
        }
        self.subprocess.stop();

        // Quit our read thread after exiting the subprocess so that
        // we don't get stuck waiting for data to stop flowing if it is
        // a particularly noisy process.
        if let Some(pipe) = td.read_thread_pipe.take() {
            match rustix::io::write(&pipe, b"x") {
                Ok(_) => {}
                // BrokenPipe means that our read thread is closed already,
                // which is completely fine since that is what we were trying
                // to achieve.
                Err(rustix::io::Errno::PIPE) => {}
                Err(err) => log::warn!("error writing to read thread quit pipe err={err}"),
            }
        }

        if let Some(t) = td.read_thread.take()
            && t.join().is_err()
        {
            log::warn!("read thread panicked");
        }
    }

    pub fn focus_gained(&mut self, td: &mut ThreadData, focused: bool) {
        if focused {
            // Always set this to true. There is a race condition if we lose
            // focus and regain focus before the termios timer ticks where
            // if we don't set this unconditionally the timer will end on
            // the next iteration.
            td.termios_timer_running = true;

            // If we're focused, we want to start our termios timer. We
            // only do this if it isn't already running. We use the termios
            // callback because that'll trigger an immediate state check AND
            // start the timer.
            if td.termios_deadline.is_none() {
                td.termios_tick();
            }
        } else {
            // Flag the timer to end on the next iteration. This is
            // a lot cheaper than doing full timer cancellation.
            td.termios_timer_running = false;
        }
    }

    pub fn resize(&mut self, grid_size: GridSize, screen_size: ScreenSize) -> Result<(), Error> {
        self.subprocess.resize(grid_size, screen_size)
    }

    pub fn queue_write(&mut self, td: &mut ThreadData, data: &[u8], linefeed: bool) {
        td.queue_write(data, linefeed);
    }
}

/// How we notice the child exiting.
enum ProcessWatch {
    /// A pidfd that becomes readable when the process exits.
    PidFd(OwnedFd, i32),
    /// Fallback: `waitpid(WNOHANG)` every [`PROCESS_POLL_MS`].
    Poll(i32, Instant),
    /// Flatpak host command: events arrive through `flatpak_events`.
    Flatpak,
    /// Already exited (or never watched).
    Done,
}

impl ProcessWatch {
    fn for_pid(pid: i32) -> ProcessWatch {
        let Some(p) = Pid::from_raw(pid) else {
            return ProcessWatch::Done;
        };
        match rustix::process::pidfd_open(p, PidfdFlags::empty()) {
            Ok(fd) => ProcessWatch::PidFd(fd, pid),
            Err(err) => {
                log::warn!("pidfd_open failed ({err}), falling back to polling waitpid");
                ProcessWatch::Poll(pid, Instant::now() + Duration::from_millis(PROCESS_POLL_MS))
            }
        }
    }
}

struct FlatpakEvents {
    rx: Receiver<Result<u8, String>>,
    wakeup: Wakeup,
}

/// The thread local data for the exec implementation.
pub struct ThreadData {
    /// Process start time and boolean of whether its already exited.
    start: Instant,
    pub exited: bool,

    /// A dup of the pty master used for writes and termios queries.
    pty_fd: OwnedFd,

    /// Pending writes to the pty (the write queue of the `xev.Stream`) and
    /// the offset into the front chunk.
    write_queue: VecDeque<Vec<u8>>,
    write_offset: usize,

    watch: ProcessWatch,
    flatpak_events: Option<FlatpakEvents>,

    /// Reader thread state
    read_thread: Option<JoinHandle<()>>,
    read_thread_pipe: Option<OwnedFd>,

    /// The termios timer: `None` when not armed.
    termios_deadline: Option<Instant>,
    termios_timer_running: bool,

    /// The last known termios mode. Used for change detection
    /// to prevent unnecessary locking of expensive mutexes.
    termios_mode: Mode,

    surface_mailbox: Arc<dyn SurfaceMailbox>,
    renderer_state: SharedState,
}

impl ThreadData {
    /// The pty fd, for `POLLOUT` polling and tests.
    pub fn pty_fd(&self) -> BorrowedFd<'_> {
        self.pty_fd.as_fd()
    }

    /// True if there are queued writes that need a `POLLOUT` wakeup.
    pub fn wants_write(&self) -> bool {
        !self.exited && !self.write_queue.is_empty()
    }

    /// An fd that becomes readable when the child exited.
    pub fn process_fd(&self) -> Option<BorrowedFd<'_>> {
        match &self.watch {
            ProcessWatch::PidFd(fd, _) => Some(fd.as_fd()),
            _ => None,
        }
    }

    /// The wakeup of the Flatpak exit notification, if any.
    pub fn flatpak_fd(&self) -> Option<BorrowedFd<'_>> {
        self.flatpak_events.as_ref().map(|e| e.wakeup.as_fd())
    }

    /// The next time [`ThreadData::on_timer`] needs to run.
    pub fn next_deadline(&self) -> Option<Instant> {
        let poll = match &self.watch {
            ProcessWatch::Poll(_, at) => Some(*at),
            _ => None,
        };
        [self.termios_deadline, poll].into_iter().flatten().min()
    }

    /// Queue a write directly to the pty. If the process exited no more
    /// writes are sent. With `linefeed` every `\r` is replaced by `\r\n`.
    pub fn queue_write(&mut self, data: &[u8], linefeed: bool) {
        // If our process is exited then we don't send any more writes.
        if self.exited || data.is_empty() {
            return;
        }

        if linefeed {
            let mut buf = Vec::with_capacity(data.len() + 8);
            for &ch in data {
                buf.push(ch);
                if ch == b'\r' {
                    buf.push(b'\n');
                }
            }
            self.write_queue.push_back(buf);
        } else {
            self.write_queue.push_back(data.to_vec());
        }

        self.flush_writes();
    }

    /// Write as much of the queue as the pty accepts right now.
    pub fn flush_writes(&mut self) {
        while let Some(front) = self.write_queue.front() {
            match rustix::io::write(&self.pty_fd, &front[self.write_offset..]) {
                Ok(n) => {
                    self.write_offset += n;
                    if self.write_offset >= front.len() {
                        self.write_queue.pop_front();
                        self.write_offset = 0;
                    }
                }
                Err(rustix::io::Errno::INTR) => {}
                // The pty buffer is full: wait for POLLOUT.
                Err(rustix::io::Errno::AGAIN) => return,
                Err(err) => {
                    log::error!("write error: {err}");
                    self.write_queue.pop_front();
                    self.write_offset = 0;
                }
            }
        }
    }

    /// The number of bytes queued for writing (for tests/diagnostics).
    pub fn queued_bytes(&self) -> usize {
        self.write_queue.iter().map(Vec::len).sum::<usize>() - self.write_offset
    }

    fn process_exit_common(&mut self, exit_code: u32) {
        self.exited = true;
        self.watch = ProcessWatch::Done;
        // Nothing can be written anymore.
        self.write_queue.clear();
        self.write_offset = 0;

        // Determine how long the process was running for.
        let runtime_ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        log::debug!("child process exited status={exit_code} runtime={runtime_ms}ms");

        // We always notify the surface immediately that the child has
        // exited and some metadata about the exit.
        self.surface_mailbox.push(
            SurfaceMessage::ChildExited(ChildExited {
                exit_code,
                runtime_ms,
            }),
            Timeout::Forever,
        );
    }

    /// Called when `process_fd()` is readable (or the poll fallback fired):
    /// reap the child and report the exit.
    pub fn on_process_ready(&mut self, cmd_pid: Option<i32>) {
        let pid = match &self.watch {
            ProcessWatch::PidFd(_, pid) | ProcessWatch::Poll(pid, _) => *pid,
            _ => return,
        };
        let _ = cmd_pid;
        let Some(p) = Pid::from_raw(pid) else {
            return;
        };
        match rustix::process::waitpid(Some(p), WaitOptions::NOHANG) {
            Ok(Some((_, status))) => {
                let code = exit_code(Exit::init(status.as_raw() as u32));
                self.process_exit_common(code);
            }
            // Not exited yet (only possible for the polling fallback).
            Ok(None) => {
                if let ProcessWatch::Poll(_, at) = &mut self.watch {
                    *at = Instant::now() + Duration::from_millis(PROCESS_POLL_MS);
                }
            }
            Err(err) => {
                // Somebody else reaped it. Report a generic exit.
                log::warn!("waitpid failed on exit notification err={err}");
                self.process_exit_common(0);
            }
        }
    }

    /// Called when `flatpak_fd()` is readable.
    pub fn on_flatpak_ready(&mut self) {
        let Some(ev) = &self.flatpak_events else {
            return;
        };
        ev.wakeup.drain();
        match ev.rx.try_recv() {
            Ok(Ok(code)) => self.process_exit_common(u32::from(code)),
            Ok(Err(err)) => {
                log::error!("error waiting for flatpak host command err={err}");
                self.process_exit_common(1);
            }
            Err(_) => {}
        }
    }

    /// Run due timers.
    pub fn on_timer(&mut self, now: Instant) {
        if let Some(at) = self.termios_deadline
            && now >= at
        {
            self.termios_deadline = None;
            self.termios_tick();
        }
        if let ProcessWatch::Poll(_, at) = &self.watch
            && now >= *at
        {
            self.on_process_ready(None);
        }
    }

    /// The termios timer body (`termiosTimer`).
    fn termios_tick(&mut self) {
        // This is kind of hacky but we only need the master fd to get the
        // termios data.
        let mode = match pty::get_mode_fd(&self.pty_fd) {
            Ok(m) => m,
            Err(err) => {
                log::warn!("error getting termios mode err={err}");
                // If we have an error we return the default mode values
                // which are the likely values.
                Mode::default()
            }
        };

        // If the mode changed, then we process it.
        if mode != self.termios_mode {
            log::debug!("termios change mode={mode:?}");
            self.termios_mode = mode;

            // We assume we're in some sort of password input if we're
            // in canonical mode and not echoing. This is a heuristic.
            let password_input = mode.canonical && !mode.echo;

            // If our password input state changed on the terminal then
            // we notify the surface.
            let changed = {
                let t = self.renderer_state.lock_terminal();
                t.flags.password_input != password_input
            };

            if changed {
                // We have to notify the surface that we're in password input.
                // We must block on this because the balanced true/false state
                // of this is critical to apprt behavior.
                self.surface_mailbox.push(
                    SurfaceMessage::PasswordInput(password_input),
                    Timeout::Forever,
                );
            }
        }

        // Repeat the timer
        if self.termios_timer_running {
            self.termios_deadline = Some(Instant::now() + Duration::from_millis(TERMIOS_POLL_MS));
        }
    }
}

/// Convert the way a child exited to the `u32` exit code reported to the
/// surface. libxev reports the exit status for normal exits and the signal
/// number if the child was killed by a signal.
pub fn exit_code(exit: Exit) -> u32 {
    match exit {
        Exit::Exited(c) => u32::from(c),
        Exit::Signal(s) | Exit::Stopped(s) | Exit::Unknown(s) => s,
    }
}

fn set_nonblocking(fd: &OwnedFd) {
    let res = rustix::fs::fcntl_getfl(fd)
        .and_then(|flags| rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK));
    if let Err(err) = res {
        log::warn!("read thread failed to set flags err={err}");
        log::warn!("this isn't a fatal error, but may cause performance issues");
    }
}

/// The read thread sits in a loop doing the following pseudo code:
///
///   while (true) { blocking_read(); exit_if_eof(); process(); }
///
/// Almost all terminal-modifying activity is from the pty read, so
/// putting this on a dedicated thread keeps performance very predictable
/// while also almost optimal. "Locking is fast, lock contention is slow."
/// and since we rarely have contention, this is fast.
///
/// This is also empirically fast compared to putting the read into
/// an async mechanism like io_uring/epoll because the reads are generally
/// small.
///
/// We use a basic poll syscall here because we are only monitoring two
/// fds and this is still much faster and lower overhead than any async
/// mechanism.
pub fn read_thread_main(fd: &OwnedFd, output: &Output, quit: &OwnedFd) {
    // The pty fd is already non-blocking (set before spawning). We try to
    // read from the fd in a tight loop and only check the quit fd
    // occasionally.
    let mut buf = [0u8; 1024];
    let mut hup = false;
    loop {
        // We try to read from the file descriptor as long as possible
        // to maximize performance. We only check the quit fd if the
        // main fd blocks. This optimizes for the realistic scenario that
        // the data will eventually stop while we're trying to quit. This
        // is always true because we kill the process.
        loop {
            match rustix::io::read(fd, &mut buf) {
                // This happens on macOS instead of WouldBlock when the
                // child process dies. To be safe, we just break the loop
                // and let our poll happen.
                Ok(0) => break,
                Ok(n) => output.process_output(&buf[..n]),
                Err(rustix::io::Errno::INTR) => {}
                // No more data, fall back to poll and check for
                // exit conditions.
                Err(rustix::io::Errno::AGAIN) => break,
                // This means our pty is closed. We're probably
                // gracefully shutting down.
                Err(rustix::io::Errno::IO | rustix::io::Errno::BADF) => {
                    log::info!("io reader exiting");
                    return;
                }
                Err(err) => {
                    log::error!("io reader error err={err}");
                    return;
                }
            }
        }

        // The pty hung up and we drained everything that was left.
        if hup {
            log::info!("pty fd closed, read thread exiting");
            return;
        }

        // Wait for data.
        let mut pollfds = [
            PollFd::new(fd, PollFlags::IN),
            PollFd::new(quit, PollFlags::IN),
        ];
        loop {
            match rustix::event::poll(&mut pollfds, None) {
                Ok(_) => break,
                Err(rustix::io::Errno::INTR) => {}
                Err(err) => {
                    log::warn!("poll failed on read thread, exiting early err={err}");
                    return;
                }
            }
        }

        // If our quit fd is set, we're done.
        if pollfds[1].revents().contains(PollFlags::IN) {
            log::info!("read thread got quit signal");
            return;
        }

        // If our pty fd is closed, then we're also done with our
        // read thread. Zig exits immediately here; we first make one more
        // pass over the fd so output that raced with the hangup is not lost.
        if pollfds[0].revents().contains(PollFlags::HUP) {
            hup = true;
        }
    }
}

//-------------------------------------------------------------------------
// Subprocess

/// Union that represents the running process type.
enum Process {
    /// Standard POSIX fork/exec
    ForkExec(Command),

    /// Flatpak DBus command
    Flatpak(FlatpakProcess),
}

struct FlatpakProcess {
    pid: u32,
    /// Taken by the exit watcher (`wait_callback` consumes the command).
    cmd: Option<ghostty_os::flatpak::FlatpakHostCommand>,
    /// Signal handle that outlives the command's move into the watcher.
    signal: Option<ghostty_os::flatpak::FlatpakSignalHandle>,
}

struct Subprocess {
    cwd: Option<String>,
    env: Option<EnvMap>,
    args: Vec<String>,
    grid_size: GridSize,
    screen_size: ScreenSize,
    pty: Option<Pty>,
    process: Option<Process>,

    rt_pre_exec_info: RtPreExecInfo,
    rt_post_fork_info: RtPostForkInfo,
    create_scope: Option<CreateScopeFn>,
    restore_rlimit: Option<Rlimit>,
}

impl Subprocess {
    /// Initialize the subprocess. This will NOT start it, this only sets
    /// up the internal state necessary to start it later.
    fn new(cfg: Config) -> Result<Subprocess, Error> {
        // Get our env. If a default env isn't provided by the caller
        // then we get it ourselves.
        let mut env = cfg.env;

        // If we have a resources dir then set our env var
        if let Some(dir) = &cfg.resources_dir {
            log::info!("found Ghostty resources dir: {dir}");
            env.insert("GHOSTTY_RESOURCES_DIR", dir.clone());
        }

        // Set our TERM var. This is a bit complicated because we want to use
        // the ghostty TERM value but we want to only do that if we have
        // ghostty in the TERMINFO database.
        //
        // For now, we just look up a bundled dir but in the future we should
        // also load the terminfo database and look for it.
        if let Some(base) = &cfg.resources_dir {
            env.insert("TERM", cfg.term.clone());
            env.insert("COLORTERM", "truecolor");

            // Assume that the resources directory is adjacent to the terminfo
            // database
            //
            // Rust-specific: the in-repo dev resources dir (`<ws>/resources`)
            // carries its compiled terminfo inside itself, so prefer that
            // when present.
            let own = format!("{base}/terminfo");
            let terminfo = if std::path::Path::new(&format!("{own}/x/xterm-ghostty")).exists() {
                own
            } else {
                let parent = std::path::Path::new(base)
                    .parent()
                    .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
                format!("{parent}/terminfo")
            };
            env.insert("TERMINFO", terminfo);
        } else {
            env.insert("TERM", "xterm-256color");
            env.insert("COLORTERM", "truecolor");
        }

        // Add our binary to the path if we can find it.
        add_ghostty_path(&mut env);

        // Set environment variables used by some programs (such as neovim) to detect
        // which terminal emulator and version they're running under.
        env.insert("TERM_PROGRAM", "ghostty");
        env.insert("TERM_PROGRAM_VERSION", VERSION_STRING);

        // VTE_VERSION is set by gnome-terminal and other VTE-based terminals.
        // We don't want our child processes to think we're running under VTE.
        // This is not apprt-specific, so we do it here.
        env.remove("VTE_VERSION");

        // Setup our shell integration, if we can.
        let shell_command: ConfigCommand = 'shell: {
            let default_shell_command = cfg
                .command
                .clone()
                .unwrap_or_else(|| ConfigCommand::Shell("sh".into()));

            // Always set up shell features (GHOSTTY_SHELL_FEATURES). These are
            // used by both automatic and manual shell integrations.
            shell_integration::setup_features(
                &mut env,
                cfg.shell_integration_features,
                cfg.cursor_blink.unwrap_or(true),
            );

            let force: Option<Shell> = match cfg.shell_integration {
                ShellIntegration::None => {
                    // This is a source of confusion for users despite being
                    // opt-in since it results in some Ghostty features not
                    // working. We always want to log it.
                    log::info!("shell integration disabled by configuration");
                    break 'shell default_shell_command;
                }
                ShellIntegration::Detect => None,
                ShellIntegration::Bash => Some(Shell::Bash),
                ShellIntegration::Elvish => Some(Shell::Elvish),
                ShellIntegration::Fish => Some(Shell::Fish),
                ShellIntegration::Nushell => Some(Shell::Nushell),
                ShellIntegration::Zsh => Some(Shell::Zsh),
            };

            let Some(dir) = &cfg.resources_dir else {
                log::warn!("no resources dir set, shell integration disabled");
                break 'shell default_shell_command;
            };

            let Some(integration) =
                shell_integration::setup(dir, &default_shell_command, &mut env, force)?
            else {
                log::warn!(
                    "shell could not be detected, no automatic shell integration will be injected"
                );
                break 'shell default_shell_command;
            };

            log::info!(
                "shell integration automatically injected shell={}",
                integration.shell.tag_name()
            );

            integration.command
        };

        // Add the environment variables that override any others.
        for (k, v) in cfg.env_override.iter() {
            env.insert(k, v);
        }

        // Build our args list
        let args = exec_command(&shell_command);

        // We have to copy the cwd because there is no guarantee that
        // pointers in full_config remain valid.
        let cwd = cfg.working_directory;

        // Propagate the current working directory (CWD) to the shell, enabling
        // the shell to display the current directory name rather than the
        // resolved path for symbolic links. This is important and based
        // on the same behavior in Konsole and Kitty (see the linked issues):
        // https://bugs.kde.org/show_bug.cgi?id=242114
        // https://github.com/kovidgoyal/kitty/issues/1595
        // https://github.com/ghostty-org/ghostty/discussions/7769
        if let Some(pwd) = &cwd {
            env.insert("PWD", pwd.clone());
        }

        Ok(Subprocess {
            env: Some(env),
            cwd,
            args,

            rt_pre_exec_info: cfg.rt_pre_exec_info,
            rt_post_fork_info: cfg.rt_post_fork_info,
            create_scope: cfg.create_scope,
            restore_rlimit: cfg.restore_rlimit,

            // Should be initialized with init_terminal call.
            grid_size: GridSize::default(),
            screen_size: ScreenSize {
                width: 1,
                height: 1,
            },
            pty: None,
            process: None,
        })
    }

    /// Start the subprocess. If the subprocess is already started this
    /// will panic. Returns `(read, write)` fds (dups of the pty master).
    fn start(&mut self) -> Result<(OwnedFd, OwnedFd), Error> {
        assert!(self.pty.is_none() && self.process.is_none());

        // Create our pty
        let pty = Pty::open(Winsize {
            ws_row: clamp_u16(u32::from(self.grid_size.rows)),
            ws_col: clamp_u16(u32::from(self.grid_size.columns)),
            ws_xpixel: clamp_u16(self.screen_size.width),
            ws_ypixel: clamp_u16(self.screen_size.height),
        })?;
        self.pty = Some(pty);

        match self.start_process() {
            Ok(fds) => {
                // Cleanup we only run in our parent when we successfully
                // start the process. Once our subcommand is started we can
                // close the slave side. This prevents the slave fd from
                // being leaked to future children.
                if let Some(p) = &mut self.pty {
                    p.close_slave();
                }
                // Successful start we can clear out some memory.
                self.env = None;
                Ok(fds)
            }
            Err(err) => {
                // Dropping the pty closes both sides.
                self.pty = None;
                Err(err)
            }
        }
    }

    fn start_process(&mut self) -> Result<(OwnedFd, OwnedFd), Error> {
        let pty = self.pty.as_ref().ok_or(Error::ProcessNotStarted)?;
        let slave = pty.slave.as_ref().ok_or(Error::ProcessNotStarted)?;
        let (master_raw, slave_raw) = pty.raw_fds();
        let slave_raw = slave_raw.unwrap_or(-1);

        log::debug!(
            "starting command command={}",
            self.args
                .iter()
                .map(|a| format!("`{a}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let flatpak = ghostty_os::flatpak::is_flatpak();

        // If we can't access the cwd, then don't set any cwd and inherit.
        // This is important because our cwd can be set by the shell (OSC 7)
        // and we don't want to break new windows.
        let cwd: Option<String> = match &self.cwd {
            None => None,
            Some(proposed) if flatpak => {
                // Flatpak sandboxing prevents access to certain reserved paths
                // regardless of configured permissions. Perform a test spawn
                // to get around this problem
                //
                // https://docs.flatpak.org/en/latest/sandbox-permissions.html#reserved-paths
                log::info!("flatpak detected, will use host command to verify cwd access");
                let dev_null = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/null")?;
                let mk = || dev_null.try_clone().map(OwnedFd::from);
                let mut cmd = ghostty_os::flatpak::FlatpakHostCommand::new(
                    vec!["/bin/sh".into(), "-c".into(), ":".into()],
                    mk()?,
                    mk()?,
                    mk()?,
                );
                cmd.cwd = Some(proposed.clone());
                match cmd.spawn() {
                    Ok(_) => {
                        let _ = cmd.wait();
                        Some(proposed.clone())
                    }
                    Err(err) => {
                        log::warn!("cannot spawn command at cwd, ignoring: {err}");
                        None
                    }
                }
            }
            Some(proposed) => {
                if std::path::Path::new(proposed).exists() {
                    Some(proposed.clone())
                } else {
                    log::warn!("cannot access cwd, ignoring: {proposed}");
                    None
                }
            }
        };

        // In flatpak, we use the HostCommand to execute our shell.
        if flatpak {
            let mut cmd = ghostty_os::flatpak::FlatpakHostCommand::new(
                self.args.clone(),
                slave.try_clone()?,
                slave.try_clone()?,
                slave.try_clone()?,
            );
            cmd.cwd = cwd;
            cmd.env.clone_from(&self.env);
            let pid = cmd.spawn().map_err(Error::Flatpak)?;
            log::info!(
                "started subcommand on host via flatpak API path={} pid={pid}",
                self.args[0]
            );
            self.process = Some(Process::Flatpak(FlatpakProcess {
                pid,
                signal: Some(cmd.signal_handle()),
                cmd: Some(cmd),
            }));
            return Ok((pty.master.try_clone()?, pty.master.try_clone()?));
        }

        // Build our subcommand
        let pre_info = self.rt_pre_exec_info;
        let post_info = self.rt_post_fork_info;
        let create_scope = self.create_scope.clone();
        let mut cmd = Command {
            path: self.args[0].clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            cwd,
            stdin: Some(slave.try_clone()?),
            stdout: Some(slave.try_clone()?),
            stderr: Some(slave.try_clone()?),
            restore_rlimit: self.restore_rlimit,
            os_pre_exec: Some(Box::new(move || {
                // Runs in the forked child (see `Subprocess.childPreExec`).
                // Zig logs the error and continues.
                let _ = pty::child_pre_exec(master_raw, slave_raw);
                None
            })),
            rt_pre_exec: Some(Box::new(move || linux_cgroup_pre_exec(&pre_info))),
            rt_post_fork: Some(Box::new(move |c: &Command| {
                let create = |pid, mem, tasks| match &create_scope {
                    Some(f) => f(pid, mem, tasks),
                    None => None,
                };
                linux_cgroup_post_fork(c, &post_info, create)
                    .map_err(|_: PostForkError| PostForkError)
            })),
            pid: None,
        };

        cmd.start()?;
        // The parent must not keep the slave open through the Command
        // (Zig stored raw handles that are closed with the slave).
        cmd.stdin = None;
        cmd.stdout = None;
        cmd.stderr = None;
        log::info!("started subcommand path={} pid={:?}", self.args[0], cmd.pid);

        let fds = (pty.master.try_clone()?, pty.master.try_clone()?);
        self.process = Some(Process::ForkExec(cmd));
        Ok(fds)
    }

    /// Called to notify that we exited externally so we can unset our
    /// running state.
    fn external_exit(&mut self) {
        self.process = None;
    }

    /// Stop the subprocess. This is safe to call anytime. This will wait
    /// for the subprocess to register that it has been signalled, but not
    /// for it to terminate, so it will not block.
    /// This does not close the pty.
    fn stop(&mut self) {
        let Some(process) = self.process.take() else {
            return;
        };
        match process {
            Process::ForkExec(cmd) => {
                // Note: this will also wait for the command to exit, so
                // DO NOT call cmd.wait
                if let Err(err) = kill_command(&cmd) {
                    log::error!("error sending SIGHUP to command, may hang: {err}");
                }
            }
            Process::Flatpak(fp) => {
                let res = match (&fp.cmd, &fp.signal) {
                    (Some(cmd), _) => cmd.signal(1 /* SIGHUP */, true),
                    (None, Some(h)) => h.signal(1 /* SIGHUP */, true),
                    (None, None) => {
                        log::warn!("no signal handle for flatpak host command pid={}", fp.pid);
                        Ok(())
                    }
                };
                if let Err(err) = res {
                    log::error!("error sending SIGHUP to command, may hang: {err}");
                }
            }
        }
    }

    /// Resize the pty subprocess. This is safe to call anytime.
    fn resize(&mut self, grid_size: GridSize, screen_size: ScreenSize) -> Result<(), Error> {
        self.grid_size = grid_size;
        self.screen_size = screen_size;

        if let Some(pty) = &mut self.pty {
            // It is theoretically possible for the grid or screen size to
            // exceed u16, although the terminal in that case isn't very
            // usable. This should be protected upstream but we still clamp
            // in case there is a bad caller which has happened before.
            pty.set_size(Winsize {
                ws_row: clamp_u16(u32::from(grid_size.rows)),
                ws_col: clamp_u16(u32::from(grid_size.columns)),
                ws_xpixel: clamp_u16(screen_size.width),
                ws_ypixel: clamp_u16(screen_size.height),
            })?;
        }
        Ok(())
    }
}

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

/// Add the directory of our own binary to `PATH` and set `GHOSTTY_BIN_DIR`.
fn add_ghostty_path(env: &mut EnvMap) {
    // Skip this for flatpak since host cannot reach them
    if ghostty_os::flatpak::is_flatpak() {
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            log::warn!("failed to get ghostty exe path err={err}");
            return;
        }
    };
    let Some(exe_dir) = exe.parent().map(|p| p.to_string_lossy().into_owned()) else {
        return;
    };
    log::debug!("appending ghostty bin to path dir={exe_dir}");

    // We always set this so that if the shell overwrites the path
    // scripts still have a way to find the Ghostty binary when
    // running in Ghostty.
    env.insert("GHOSTTY_BIN_DIR", exe_dir.clone());

    // Append if we have a path. We want to append so that ghostty is
    // the last priority in the path. If we don't have a path set
    // then we just set it to the directory of the binary.
    if let Some(path) = env.get("PATH").map(std::ffi::OsStr::to_owned) {
        // Verify that our path doesn't already contain this entry
        use std::os::unix::ffi::OsStrExt;
        if path
            .as_bytes()
            .split(|&b| b == b':')
            .any(|entry| !entry.is_empty() && entry == exe_dir.as_bytes())
        {
            return;
        }
        env.insert(
            "PATH",
            ghostty_os::env::append_env_os(&path, std::ffi::OsStr::new(&exe_dir)),
        );
    } else {
        env.insert("PATH", exe_dir);
    }
}

/// Kill the underlying subprocess. This sends a SIGHUP to the child
/// process group. This also waits for the command to exit.
fn kill_command(command: &Command) -> Result<(), Error> {
    if let Some(pid) = command.pid {
        kill_pid(pid)?;
    }
    Ok(())
}

fn kill_pid(pid_raw: i32) -> Result<(), Error> {
    let Some(pid) = Pid::from_raw(pid_raw) else {
        return Ok(());
    };
    let Some(pgid) = getpgid(pid) else {
        return Ok(());
    };

    // It is possible to send a killpg between the time that
    // our child process calls setsid but before or simultaneous
    // to calling execve. In this case, the direct child dies
    // but grandchildren survive. To work around this, we loop
    // and repeatedly kill the process group until all
    // descendents are well and truly dead. We will not rest
    // until the entire family tree is obliterated.
    loop {
        match rustix::process::kill_process_group(pgid, Signal::HUP) {
            Ok(()) => log::debug!("process group killed pgid={}", pgid.as_raw_nonzero()),
            Err(err) => {
                log::warn!(
                    "error killing process group pgid={} err={err}",
                    pgid.as_raw_nonzero()
                );
                return Err(Error::KillFailed);
            }
        }

        // See Command::wait for why we specify WNOHANG.
        // The gist is that it lets us detect when children
        // are still alive without blocking so that we can
        // kill them again.
        match rustix::process::waitpid(Some(pid), WaitOptions::NOHANG) {
            Ok(Some(_)) => {
                log::debug!("waitpid result={pid_raw}");
                break;
            }
            Ok(None) => {
                log::debug!("waitpid result=0");
            }
            // Already reaped by someone else (e.g. our exit watcher).
            Err(rustix::io::Errno::CHILD) => break,
            Err(err) => {
                log::warn!("waitpid failed err={err}");
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn getpgid(pid: Pid) -> Option<Pid> {
    // Get our process group ID. Before the child pid calls setsid
    // the pgid will be ours because we forked it. Its possible that
    // we may be calling this before setsid if we are killing a surface
    // VERY quickly after starting it.
    let my_pgid = rustix::process::getpgid(None).ok();

    // We loop while pgid == my_pgid. The expectation if we have a valid
    // pid is that setsid will eventually be called because it is the
    // FIRST thing the child process does and as far as I can tell,
    // setsid cannot fail. (Zig loops forever; we give up after ~5s.)
    for _ in 0..500 {
        match rustix::process::getpgid(Some(pid)) {
            Ok(pgid) => {
                if Some(pgid) == my_pgid {
                    log::warn!("pgid is our own, retrying");
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Some(pgid);
            }
            // If the pid doesn't exist then... we're done!
            Err(rustix::io::Errno::SRCH) => return None,
            // If we have an error we're done.
            Err(_) => {
                log::warn!("error getting pgid for kill");
                return None;
            }
        }
    }
    None
}

/// Builds the argv array for the process we should exec for the
/// configured command. This isn't as straightforward as it seems since
/// we deal with shell-wrapping, etc.
///
/// (The macOS `login(1)` branch of the Zig function is not ported, Linux
/// only; neither is the Windows `cmd.exe` branch.)
pub fn exec_command(command: &ConfigCommand) -> Vec<String> {
    match command {
        // We need to clone the command since there's no guarantee the config remains valid.
        ConfigCommand::Direct(args) => args.clone(),

        ConfigCommand::Shell(v) => {
            let mut args: Vec<String> = Vec::with_capacity(4);

            // We run our shell wrapped in `/bin/sh` so that we don't have
            // to parse the command line ourselves if it has arguments.
            // Additionally, some environments (NixOS, I found) use /bin/sh
            // to setup some environment variables that are important to
            // have set.
            args.push("/bin/sh".into());
            if ghostty_os::flatpak::is_flatpak() {
                args.push("-l".into());
            }
            args.push("-c".into());
            args.push(v.clone());
            args
        }
    }
}

impl Drop for ThreadData {
    fn drop(&mut self) {
        // Join the reader if thread_exit wasn't called (shouldn't happen).
        if let Some(pipe) = self.read_thread_pipe.take() {
            let _ = rustix::io::write(&pipe, b"x");
        }
        if let Some(t) = self.read_thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_command_shell_command() {
        // execCommand: shell command, empty passwd
        let result = exec_command(&ConfigCommand::Shell("foo bar baz".into()));
        assert_eq!(result, ["/bin/sh", "-c", "foo bar baz"]);
    }

    #[test]
    fn exec_command_direct_command() {
        // execCommand: direct command, error passwd
        let result = exec_command(&ConfigCommand::Direct(vec!["foo".into(), "bar baz".into()]));
        assert_eq!(result, ["foo", "bar baz"]);
    }

    #[test]
    fn exec_command_direct_command_config_freed() {
        let result = {
            let command = ConfigCommand::Direct(vec!["foo".into(), "bar baz".into()]);
            let r = exec_command(&command);
            drop(command);
            r
        };
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "foo");
        assert_eq!(result[1], "bar baz");
    }

    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code(Exit::Exited(3)), 3);
        assert_eq!(exit_code(Exit::Signal(9)), 9);
    }
}
