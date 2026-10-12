//! Exec implements the logic for starting and stopping a subprocess with a
//! ConPTY pseudo console as well as spinning up the necessary read thread to
//! read from the pty and forward it to the Termio instance
//! (`termio/Exec.zig`, Windows branches). Windows variant of [`crate::exec`].
//!
//! # Event loop mapping
//!
//! libxev completions are replaced by state in [`ThreadData`] that the IO
//! thread's `WaitForMultipleObjects` loop (see [`crate::thread`]) consults:
//!
//! * process exit: the process handle (signaled when the process exits),
//! * pty writes: a dedicated writer thread draining a queue of byte chunks
//!   with blocking overlapped `WriteFile` calls (the input pipe is a named
//!   pipe opened for overlapped I/O, as in Zig),
//! * the termios timer does not exist on Windows (Zig does not run it
//!   either).

// Win32 FFI takes out-parameters as `*mut`; implicit `&mut` -> pointer coercions
// are the idiomatic spelling at these call sites.
#![allow(clippy::borrow_as_ptr)]

use std::{
    io,
    os::windows::io::{AsRawHandle, OwnedHandle},
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use ghostty_apprt::surface::{ChildExited, Message as SurfaceMessage};
use ghostty_config::{
    command::Command as ConfigCommand,
    repeatable_string_map::RepeatableStringMap,
    types_b::{ShellIntegration, ShellIntegrationFeatures},
};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_os::osbytes::EncodedOs;
use ghostty_renderer::size::{GridSize, ScreenSize};
use ghostty_terminal::terminal::Terminal;
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE,
        ERROR_IO_PENDING,
        ERROR_NO_DATA,
        ERROR_OPERATION_ABORTED,
        ERROR_PIPE_NOT_CONNECTED,
        FALSE,
        GetLastError,
        HANDLE,
        TRUE,
    },
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::PeekNamedPipe,
        Threading::CreateEventW,
    },
};

/// `build_config.version_string`.
pub use crate::stream_handler::VERSION_STRING;
use crate::{
    command::{self, Command, Exit},
    pty::{self, Pty, Winsize},
    shell_integration::{self, EnvMap, Shell},
    sinks::SurfaceMailbox,
    state::SharedState,
    termio::Output,
    windows_args::windows_shell_args,
};

/// Errors from starting and running the exec backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Pty(#[from] pty::Error),
    #[error(transparent)]
    Start(#[from] command::StartError),
    #[error(transparent)]
    ShellIntegration(#[from] shell_integration::Error),
    /// A configured path, argument or environment value is not well-formed
    /// UTF-8 (Zig's process launch conversions fail on such input).
    #[error(transparent)]
    InvalidUtf8(#[from] ghostty_os::osbytes::InvalidUtf8),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("failed to get WINDIR, cannot run shell command")]
    SystemError,
    #[error("process not started")]
    ProcessNotStarted,
    #[error("process has no pid")]
    ProcessNoPid,
}

/// Configuration for the exec backend (`Exec.Config`).
///
/// The POSIX-only fields (`rt_pre_exec_info`, `rt_post_fork_info`,
/// `create_scope`, `restore_rlimit`) do not exist on Windows.
pub struct Config {
    pub command: Option<ConfigCommand>,
    pub env: EnvMap,
    pub env_override: RepeatableStringMap,
    pub shell_integration: ShellIntegration,
    pub shell_integration_features: ShellIntegrationFeatures,
    pub cursor_blink: Option<bool>,
    pub working_directory: Option<EncodedOs>,
    pub resources_dir: Option<EncodedOs>,
    pub term: EncodedOs,
}

impl Config {
    /// A config with Zig-struct-style defaults for everything optional.
    pub fn new(env: EnvMap, term: EncodedOs) -> Config {
        Config {
            command: None,
            env,
            env_override: RepeatableStringMap::default(),
            shell_integration: ShellIntegration::Detect,
            shell_integration_features: ShellIntegrationFeatures::default(),
            cursor_blink: None,
            working_directory: None,
            resources_dir: None,
            term,
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
    pub fn cwd(&self) -> Option<&EncodedOs> {
        self.subprocess.cwd.as_ref()
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
    /// This is called before any termio begins.
    pub fn init_terminal(&mut self, term: &mut Terminal) {
        // If we have an initial pwd requested by the subprocess, then we
        // set that on the terminal now. This allows rapidly initializing
        // new surfaces to use the proper pwd.
        if let Some(cwd) = &self.subprocess.cwd
            && let Err(err) = term.set_pwd(&cwd.to_bytes())
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

    /// Start the subprocess and the read/write threads. `output` is where
    /// the read thread delivers pty output.
    pub fn thread_enter(
        &mut self,
        output: Arc<Output>,
        surface_mailbox: Arc<dyn SurfaceMailbox>,
        _renderer_state: SharedState,
    ) -> Result<ThreadData, Error> {
        // Start our subprocess
        let (read_handle, write_handle) = self.subprocess.start()?;

        // Watcher to detect subprocess exit: a duplicate of the process
        // handle that the IO thread waits on.
        let process = self
            .subprocess
            .command
            .as_ref()
            .and_then(Command::process_handle)
            .ok_or(Error::ProcessNotStarted)?
            .try_clone_to_owned()?;

        // Track our process start time for abnormal exits
        let start = Instant::now();

        // Create our pipe that we'll use to kill our read thread.
        // pipe.0 is the read end, pipe.1 is the write end.
        let (quit_read, quit_write) = ghostty_os::pipe::pipe()?;

        // Handles we use to interrupt the blocking I/O of the threads.
        let read_cancel = read_handle.try_clone()?;
        let write_cancel = write_handle.try_clone()?;

        // Start our read thread
        let crash_dims = ghostty_crash::current_dims();
        let read_thread =
            std::thread::Builder::new()
                .name("io-reader".into())
                .spawn(move || {
                    let _crash =
                        ghostty_crash::ThreadGuard::new(ghostty_crash::ThreadType::Io, crash_dims);
                    read_thread_main(&read_handle, &output, &quit_read);
                })?;

        // Start our write thread
        let (write_tx, write_rx) = crossbeam_channel::unbounded();
        let write_thread = std::thread::Builder::new()
            .name("io-writer".into())
            .spawn(move || write_thread_main(&write_handle, &write_rx))?;

        Ok(ThreadData {
            start,
            exited: false,
            process: Some(process),
            write_tx: Some(write_tx),
            write_thread: Some(write_thread),
            write_cancel,
            read_thread: Some(read_thread),
            read_thread_pipe: Some(quit_write),
            read_cancel,
            surface_mailbox,
        })
    }

    pub fn thread_exit(&mut self, td: &mut ThreadData) {
        if td.exited {
            self.subprocess.external_exit();
        }
        self.subprocess.stop();
        td.stop_threads();
    }

    /// The termios timer isn't supported on Windows (as in Zig), so focus
    /// changes need no handling here.
    #[allow(clippy::unused_self)]
    pub fn focus_gained(&mut self, _td: &mut ThreadData, _focused: bool) {}

    pub fn resize(&mut self, grid_size: GridSize, screen_size: ScreenSize) -> Result<(), Error> {
        self.subprocess.resize(grid_size, screen_size)
    }

    #[allow(clippy::unused_self)]
    pub fn queue_write(&mut self, td: &mut ThreadData, data: &[u8], linefeed: bool) {
        td.queue_write(data, linefeed);
    }
}

/// The thread local data for the exec implementation.
pub struct ThreadData {
    /// Process start time and boolean of whether its already exited.
    start: Instant,
    pub exited: bool,

    /// Duplicate of the process handle; signaled when the process exits.
    process: Option<OwnedHandle>,

    /// Pending writes to the pty, drained by the writer thread.
    write_tx: Option<Sender<Vec<u8>>>,
    write_thread: Option<JoinHandle<()>>,
    write_cancel: OwnedHandle,

    /// Reader thread state
    read_thread: Option<JoinHandle<()>>,
    read_thread_pipe: Option<OwnedHandle>,
    read_cancel: OwnedHandle,

    surface_mailbox: Arc<dyn SurfaceMailbox>,
}

impl ThreadData {
    /// A handle that becomes signaled when the child exited.
    pub fn process_handle(&self) -> Option<std::os::windows::io::BorrowedHandle<'_>> {
        use std::os::windows::io::AsHandle;
        self.process.as_ref().map(AsHandle::as_handle)
    }

    /// Queue a write directly to the pty. If the process exited no more
    /// writes are sent. With `linefeed` every `\r` is replaced by `\r\n`.
    pub fn queue_write(&mut self, data: &[u8], linefeed: bool) {
        // If our process is exited then we don't send any more writes.
        if self.exited || data.is_empty() {
            return;
        }
        let Some(tx) = &self.write_tx else {
            return;
        };

        let chunk = if linefeed {
            let mut buf = Vec::with_capacity(data.len() + 8);
            for &ch in data {
                buf.push(ch);
                if ch == b'\r' {
                    buf.push(b'\n');
                }
            }
            buf
        } else {
            data.to_vec()
        };
        let _ = tx.send(chunk);
    }

    fn process_exit_common(&mut self, exit_code: u32) {
        self.exited = true;
        self.process = None;
        // Nothing can be written anymore.
        self.write_tx = None;

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

    /// Called when `process_handle()` is signaled: report the exit.
    #[allow(unsafe_code)]
    pub fn on_process_ready(&mut self) {
        let Some(process) = &self.process else {
            return;
        };
        let mut code: u32 = 0;
        // SAFETY: valid process handle and out pointer.
        let ok = unsafe {
            windows_sys::Win32::System::Threading::GetExitCodeProcess(
                process.as_raw_handle() as HANDLE,
                &mut code,
            )
        };
        if ok == 0 {
            log::warn!(
                "GetExitCodeProcess failed on exit notification err={}",
                io::Error::last_os_error()
            );
            code = 0;
        }
        self.process_exit_common(code);
    }

    /// The next time a timer needs to run (none on Windows).
    #[allow(clippy::unused_self)]
    pub fn next_deadline(&self) -> Option<Instant> {
        None
    }

    /// Run due timers (none on Windows).
    #[allow(clippy::unused_self)]
    pub fn on_timer(&mut self, _now: Instant) {}

    /// Quit the read and write threads (the tail of `threadExit`).
    #[allow(unsafe_code)]
    fn stop_threads(&mut self) {
        // Stop the writer: closing the queue ends its loop and cancelling
        // wakes it if it is blocked in a write.
        self.write_tx = None;

        // Quit our read thread after exiting the subprocess so that
        // we don't get stuck waiting for data to stop flowing if it is
        // a particularly noisy process.
        if let Some(pipe) = self.read_thread_pipe.take() {
            let mut written = 0u32;
            // SAFETY: valid pipe handle; the buffer is valid for 1 byte.
            let ok = unsafe {
                WriteFile(
                    pipe.as_raw_handle() as HANDLE,
                    b"x".as_ptr(),
                    1,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                // A broken pipe means that our read thread is closed
                // already, which is completely fine.
                if err != ERROR_BROKEN_PIPE && err != ERROR_NO_DATA {
                    log::warn!("error writing to read thread quit pipe err={err}");
                }
            }
        }

        // Interrupt the blocking read/write so the threads can see the quit
        // message. A cancel issued while a thread is between I/O calls is
        // lost, so repeat until the threads are really gone.
        for (thread, handle) in [
            (self.read_thread.take(), &self.read_cancel),
            (self.write_thread.take(), &self.write_cancel),
        ] {
            let Some(thread) = thread else { continue };
            while !thread.is_finished() {
                // SAFETY: valid handle; a null OVERLAPPED cancels all of this
                // process' pending I/O on it.
                if unsafe { CancelIoEx(handle.as_raw_handle() as HANDLE, std::ptr::null()) } == 0 {
                    let err = unsafe { GetLastError() };
                    // ERROR_NOT_FOUND: nothing pending right now.
                    if err != windows_sys::Win32::Foundation::ERROR_NOT_FOUND {
                        log::warn!("error interrupting io thread err={err}");
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if thread.join().is_err() {
                log::warn!("io thread panicked");
            }
        }
    }
}

impl Drop for ThreadData {
    fn drop(&mut self) {
        // Join the threads if thread_exit wasn't called (shouldn't happen).
        self.stop_threads();
    }
}

/// Convert the way a child exited to the `u32` exit code reported to the
/// surface.
pub fn exit_code(exit: Exit) -> u32 {
    match exit {
        Exit::Exited(c) => c,
    }
}

/// The read thread sits in a loop doing the following pseudo code:
///
///   while (true) { blocking_read(); exit_if_eof(); process(); }
///
/// (`ReadThread.threadMainWindows`.) The blocking `ReadFile` is interrupted
/// with `CancelIoEx` from [`Exec::thread_exit`], after which the quit pipe is
/// checked.
#[allow(unsafe_code)]
pub fn read_thread_main(fd: &OwnedHandle, output: &Output, quit: &OwnedHandle) {
    let fd_raw = fd.as_raw_handle() as HANDLE;
    let quit_raw = quit.as_raw_handle() as HANDLE;
    let mut buf = [0u8; 1024];
    loop {
        loop {
            let mut n: u32 = 0;
            // SAFETY: valid handle; the buffer is valid for `buf.len()` bytes.
            let ok = unsafe {
                ReadFile(
                    fd_raw,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut n,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // SAFETY: no preconditions.
                let err = unsafe { GetLastError() };
                match err {
                    // Check for a quit signal
                    ERROR_OPERATION_ABORTED => break,

                    // The pseudo console closed our pipe: we're done.
                    ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED => {
                        log::info!("io reader exiting");
                        return;
                    }

                    _ => {
                        log::error!("io reader error err={err}");
                        return;
                    }
                }
            }

            output.process_output(&buf[..n as usize]);
        }

        let mut quit_bytes: u32 = 0;
        // SAFETY: valid pipe handle and out pointer.
        if unsafe {
            PeekNamedPipe(
                quit_raw,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut quit_bytes,
                std::ptr::null_mut(),
            )
        } == 0
        {
            // SAFETY: no preconditions.
            log::error!("quit pipe reader error err={}", unsafe { GetLastError() });
            return;
        }

        if quit_bytes > 0 {
            log::info!("read thread got quit signal");
            return;
        }
    }
}

/// Drains the write queue with blocking overlapped writes to the input pipe.
#[allow(unsafe_code)]
fn write_thread_main(handle: &OwnedHandle, rx: &Receiver<Vec<u8>>) {
    let raw = handle.as_raw_handle() as HANDLE;

    // Manual-reset event for the OVERLAPPED structure.
    // SAFETY: default security attributes, unnamed.
    let event = unsafe { CreateEventW(std::ptr::null(), TRUE, FALSE, std::ptr::null()) };
    if event.is_null() {
        log::error!(
            "io writer failed to create event err={}",
            io::Error::last_os_error()
        );
        return;
    }
    // SAFETY: just created and owned by us.
    let event = unsafe { std::os::windows::io::FromRawHandle::from_raw_handle(event.cast()) };
    let event: OwnedHandle = event;

    while let Ok(chunk) = rx.recv() {
        let mut offset = 0;
        while offset < chunk.len() {
            // SAFETY: OVERLAPPED is plain data.
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = event.as_raw_handle() as HANDLE;
            let rest = &chunk[offset..];
            // SAFETY: valid handle; the buffer and `ov` outlive the I/O,
            // which we wait for below before leaving this scope.
            let ok = unsafe {
                WriteFile(
                    raw,
                    rest.as_ptr(),
                    rest.len() as u32,
                    std::ptr::null_mut(),
                    &mut ov,
                )
            };
            // SAFETY: no preconditions.
            if ok == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                log::error!("write error: {}", io::Error::last_os_error());
                break;
            }

            let mut written: u32 = 0;
            // SAFETY: waits for the operation started above to finish.
            if unsafe { GetOverlappedResult(raw, &ov, &mut written, TRUE) } == 0 {
                // SAFETY: no preconditions.
                let err = unsafe { GetLastError() };
                if err == ERROR_OPERATION_ABORTED {
                    // We're shutting down.
                    return;
                }
                log::error!("write error: {}", io::Error::from_raw_os_error(err as i32));
                break;
            }
            offset += written as usize;
        }
    }
}

//-------------------------------------------------------------------------
// Subprocess

/// Insert the config `env` overrides, decoding the encoded key/value text
/// to OS strings after strict UTF-8 validation, as in Zig.
///
/// Invalid UTF-8 fails (Zig's conversions for the environment block are
/// fallible) instead of being substituted with U+FFFD.
fn apply_env_overrides(
    env: &mut EnvMap,
    overrides: &RepeatableStringMap,
) -> Result<(), ghostty_os::osbytes::InvalidUtf8> {
    for (k, v) in overrides.iter() {
        env.insert(k.try_to_utf8_os()?, v.try_to_utf8_os()?);
    }
    Ok(())
}

struct Subprocess {
    cwd: Option<EncodedOs>,
    env: Option<EnvMap>,
    args: Vec<String>,
    grid_size: GridSize,
    screen_size: ScreenSize,
    pty: Option<Pty>,
    command: Option<Command>,
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
            log::info!("found Ghostty resources dir: {}", dir.to_text_lossy());
            // The resources dir is an encoded OS string: decode it for the
            // child's environment.
            env.insert("GHOSTTY_RESOURCES_DIR", dir.try_to_utf8_os()?);
        }

        // Set our TERM var. This is a bit complicated because we want to use
        // the ghostty TERM value but we want to only do that if we have
        // ghostty in the TERMINFO database.
        if let Some(base) = &cfg.resources_dir {
            // The configured TERM is an encoded OS string: export its bytes.
            env.insert("TERM", cfg.term.try_to_utf8_os()?);
            env.insert("COLORTERM", "truecolor");

            // Assume that the resources directory is adjacent to the terminfo
            // database
            base.try_to_utf8_os()?; // invalid UTF-8 fails like Zig
            env.insert("TERMINFO", terminfo_dir(base)?);
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
        env.remove("VTE_VERSION");

        // Setup our shell integration, if we can.
        let shell_command: ConfigCommand = 'shell: {
            let default_shell_command = cfg
                .command
                .clone()
                .unwrap_or_else(|| ConfigCommand::Shell("cmd.exe".into()));

            // Always set up shell features (GHOSTTY_SHELL_FEATURES). These are
            // used by both automatic and manual shell integrations.
            shell_integration::setup_features(
                &mut env,
                cfg.shell_integration_features,
                cfg.cursor_blink.unwrap_or(true),
            );

            let force: Option<Shell> = match cfg.shell_integration {
                ShellIntegration::None => {
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
        apply_env_overrides(&mut env, &cfg.env_override)?;

        // Build our args list. If we fail to find the shell, we'd still like
        // to try to run something so Ghostty can launch.
        // %WINDIR% may hold lone surrogates: carry it in the encoded form the
        // argv strings use (validated as UTF-8 when the process is created).
        let windir = std::env::var_os("WINDIR").map(|v| EncodedOs::from_os(&v).into_encoded());
        let args = match exec_command(&shell_command, windir.as_deref()) {
            Ok(args) => args,
            Err(Error::SystemError) => {
                log::warn!("failed to get WINDIR, cannot run shell command");
                return Err(Error::SystemError);
            }
            Err(err) => return Err(err),
        };

        // We have to copy the cwd because there is no guarantee that
        // pointers in full_config remain valid.
        let cwd = cfg.working_directory;

        // Propagate the current working directory (CWD) to the shell.
        if let Some(pwd) = &cwd {
            env.insert("PWD", pwd.try_to_utf8_os()?);
        }

        Ok(Subprocess {
            env: Some(env),
            cwd,
            args,

            // Should be initialized with init_terminal call.
            grid_size: GridSize::default(),
            screen_size: ScreenSize {
                width: 1,
                height: 1,
            },
            pty: None,
            command: None,
        })
    }

    /// Start the subprocess. If the subprocess is already started this
    /// will panic. Returns `(read, write)` handles: duplicates of the output
    /// pipe (read end) and input pipe (write end).
    fn start(&mut self) -> Result<(OwnedHandle, OwnedHandle), Error> {
        assert!(self.pty.is_none() && self.command.is_none());

        // Create our pty
        let pty = Pty::open(Winsize {
            ws_row: clamp_u16(u32::from(self.grid_size.rows)),
            ws_col: clamp_u16(u32::from(self.grid_size.columns)),
            ws_xpixel: clamp_u16(self.screen_size.width),
            ws_ypixel: clamp_u16(self.screen_size.height),
        })?;
        let handles = (
            pty.out_pipe().try_clone_to_owned()?,
            pty.in_pipe().try_clone_to_owned()?,
        );
        let pseudo_console = pty.pseudo_console();
        self.pty = Some(pty);

        match self.start_process(pseudo_console) {
            Ok(()) => {
                // Successful start we can clear out some memory.
                self.env = None;
                Ok(handles)
            }
            Err(err) => {
                // Dropping the pty closes everything.
                self.pty = None;
                Err(err)
            }
        }
    }

    fn start_process(
        &mut self,
        pseudo_console: windows_sys::Win32::System::Console::HPCON,
    ) -> Result<(), Error> {
        log::debug!(
            "starting command command={}",
            self.args
                .iter()
                .map(|a| format!("`{a}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );

        // If we can't access the cwd, then don't set any cwd and inherit.
        // This is important because our cwd can be set by the shell (OSC 7)
        // and we don't want to break new windows.
        let cwd: Option<EncodedOs> = match &self.cwd {
            None => None,
            Some(proposed) => match proposed.to_path().and_then(std::fs::metadata) {
                Ok(_) => Some(proposed.clone()),
                Err(err) => {
                    log::warn!("cannot access cwd, ignoring: {err}");
                    None
                }
            },
        };

        // Build our subcommand
        let mut cmd = Command::default();
        cmd.path = self.args.first().cloned().ok_or(Error::ProcessNotStarted)?;
        cmd.args.clone_from(&self.args);
        cmd.env.clone_from(&self.env);
        cmd.cwd = cwd;
        cmd.pseudo_console = Some(pseudo_console);
        cmd.start()?;
        log::info!("started subcommand path={} pid={:?}", self.args[0], cmd.pid);

        self.command = Some(cmd);
        Ok(())
    }

    /// Called to notify that we exited externally so we can unset our
    /// running state.
    fn external_exit(&mut self) {
        self.command = None;
    }

    /// Stop the subprocess. This is safe to call anytime. This does not
    /// close the pty.
    fn stop(&mut self) {
        if let Some(cmd) = self.command.take()
            && let Err(err) = kill_command(&cmd)
        {
            log::error!("error terminating command, may hang: {err}");
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
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            log::warn!("failed to get ghostty exe path err={err}");
            return;
        }
    };
    add_ghostty_path_for(env, &exe);
}

/// [`add_ghostty_path`] for a given executable (testable with non-UTF-8
/// directories).
fn add_ghostty_path_for(env: &mut EnvMap, exe: &std::path::Path) {
    // Keep the exact OS string (it may hold lone UTF-16 surrogates).
    let Some(exe_dir) = exe.parent().map(|p| p.as_os_str().to_owned()) else {
        return;
    };
    log::debug!("appending ghostty bin to path dir={}", exe_dir.display());

    // We always set this so that if the shell overwrites the path
    // scripts still have a way to find the Ghostty binary when
    // running in Ghostty.
    env.insert("GHOSTTY_BIN_DIR", exe_dir.clone());

    // Append if we have a path. We want to append so that ghostty is
    // the last priority in the path. If we don't have a path set
    // then we just set it to the directory of the binary.
    if let Some(path) = env.get("PATH").map(std::ffi::OsStr::to_owned) {
        // Verify that our path doesn't already contain this entry
        if std::env::split_paths(&path)
            .any(|entry| !entry.as_os_str().is_empty() && entry.as_os_str() == exe_dir.as_os_str())
        {
            return;
        }
        env.insert("PATH", ghostty_os::env::append_env_os(&path, &exe_dir));
    } else {
        env.insert("PATH", exe_dir);
    }
}

/// The `TERMINFO` value for a resources dir: `<parent of base>\\terminfo` as
/// an OS string (the resources dir is an encoded OS string).
fn terminfo_dir(base: &EncodedOs) -> std::io::Result<std::ffi::OsString> {
    let base = base.to_path()?;
    let mut out = base
        .parent()
        .map_or_else(std::ffi::OsString::new, |p| p.as_os_str().to_owned());
    out.push("\\terminfo");
    Ok(out)
}

/// Terminate the underlying subprocess and wait for it to exit
/// (`killCommand`: `TerminateProcess(pid, 0)` then wait).
fn kill_command(command: &Command) -> Result<(), Error> {
    if command.pid.is_some() {
        command.terminate()?;
        command.wait(false)?;
    }
    Ok(())
}

/// Builds the argv array for the process we should exec for the
/// configured command (`execCommand`). `windir` is `%WINDIR%`: shell
/// commands run as `%WINDIR%\System32\cmd.exe /C <command>`.
pub fn exec_command(command: &ConfigCommand, windir: Option<&str>) -> Result<Vec<String>, Error> {
    match command {
        // We need to clone the command since there's no guarantee the config remains valid.
        ConfigCommand::Direct(args) => Ok(args.iter().map(|v| v.as_encoded().to_owned()).collect()),

        ConfigCommand::Shell(v) => {
            let windir = windir.ok_or(Error::SystemError)?;
            Ok(windows_shell_args(windir, v.as_encoded()))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_overrides_keep_literal_fdd0_and_reject_surrogates() {
        use ghostty_config::cli_args::ConfigField;
        let mut overrides = RepeatableStringMap::default();
        overrides
            .parse_cli(Some("K\u{fdd0}é=value\u{fdd0}é"))
            .unwrap();
        let mut env = EnvMap::new();
        super::apply_env_overrides(&mut env, &overrides).unwrap();
        assert_eq!(env.get("K\u{fdd0}é").unwrap(), "value\u{fdd0}é");
        for raw in [
            b"A=\xff".as_slice(),
            b"A=\xed\xa0\x80",
            b"\xed\xa0\x80=value",
        ] {
            let mut overrides = RepeatableStringMap::default();
            overrides.parse_bytes(Some(raw)).unwrap();
            assert!(super::apply_env_overrides(&mut env, &overrides).is_err());
        }
    }

    #[test]
    fn terminfo_dir_is_sibling_of_resources_and_lossless() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        // C:\Users\<lone surrogate>\share\ghostty -> ...\share\terminfo
        let mut wide: Vec<u16> = "C:\\Users\\".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend("\\share\\ghostty".encode_utf16());
        let base = EncodedOs::from_wide(&wide);
        let got = super::terminfo_dir(&base).unwrap();
        let mut want: Vec<u16> = "C:\\Users\\".encode_utf16().collect();
        want.push(0xD800);
        want.extend("\\share\\terminfo".encode_utf16());
        assert_eq!(got.encode_wide().collect::<Vec<_>>(), want);
        assert_eq!(std::ffi::OsString::from_wide(&want), got);
    }

    use super::*;

    #[test]
    fn exec_command_shell_command() {
        let result = exec_command(
            &ConfigCommand::Shell("foo bar baz".into()),
            Some("C:\\Windows"),
        )
        .unwrap();
        assert_eq!(
            result,
            vec!["C:\\Windows\\System32\\cmd.exe", "/C", "foo bar baz"]
        );
    }

    #[test]
    fn exec_command_requires_windir() {
        assert!(matches!(
            exec_command(&ConfigCommand::Shell("x".into()), None),
            Err(Error::SystemError)
        ));
    }

    #[test]
    fn exec_command_direct_command() {
        let result = exec_command(
            &ConfigCommand::Direct(vec!["foo".into(), "bar baz".into()]),
            None,
        )
        .unwrap();
        assert_eq!(result, vec!["foo", "bar baz"]);
    }

    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code(Exit::Exited(5)), 5);
    }
}
