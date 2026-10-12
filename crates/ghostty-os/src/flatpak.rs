/// GLib's g_variant_new_string substitutes the entire invalid value (Zig's {ss}).
fn glib_env_string(value: &std::ffi::OsStr) -> String {
    let bytes = crate::osbytes::os_bytes(value);
    let bytes = bytes.split(|b| *b == 0).next().unwrap_or_default();
    std::str::from_utf8(bytes)
        .unwrap_or("[Invalid UTF-8]")
        .to_owned()
}

use std::{
    collections::HashMap,
    os::fd::{AsFd, OwnedFd},
    sync::{Arc, Condvar, Mutex},
};

use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator},
    message::Type as MessageType,
    zvariant::Fd,
};

const BUS_NAME: &str = "org.freedesktop.Flatpak";
const OBJECT_PATH: &str = "/org/freedesktop/Flatpak/Development";
const INTERFACE: &str = "org.freedesktop.Flatpak.Development";

/// Returns true if we're running in a Flatpak environment.
pub fn is_flatpak() -> bool {
    std::path::Path::new(
        FLATPAK_INFO_OVERRIDE
            .get()
            .map_or(std::path::Path::new(FLATPAK_INFO), |p| p.as_path()),
    )
    .exists()
}

const FLATPAK_INFO: &str = "/.flatpak-info";

static FLATPAK_INFO_OVERRIDE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Injection seam for integration tests: make [`is_flatpak`] look at `path`
/// instead of `/.flatpak-info`. Can be set once per process; returns false if
/// it was already set. Not used by the application.
pub fn set_flatpak_info_path(path: impl Into<std::path::PathBuf>) -> bool {
    FLATPAK_INFO_OVERRIDE.set(path.into()).is_ok()
}

/// Errors that are possible from us.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("flatpak host command must be started")]
    FlatpakMustBeStarted,
    #[error("flatpak spawn failed")]
    FlatpakSpawnFail,
    #[error("flatpak setup failed")]
    FlatpakSetupFail,
    #[error("flatpak rpc failed")]
    FlatpakRPCFail,
}

/// State the process is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Initial state
    Init,
    /// Error starting. The error message is only available via logs.
    Err,
    /// Process started with the given pid on the host.
    Started { pid: u32 },
    /// Process exited
    Exited { pid: u32, status: u8 },
}

/// State shared with the dedicated thread, protected by the lock and
/// condition variable.
struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    fn update(&self, state: State) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = state;
        self.cv.notify_all();
    }
}

/// A struct to help execute commands on the host via the
/// org.freedesktop.Flatpak.Development DBus module. This uses `zbus`
/// (blocking API) rather than GIO/GLib.
///
/// This always spawns its own thread to wait for the exit signal. This makes
/// it easy for the command to behave synchronously similar to
/// `std::process::Child`.
///
/// There are lots of chances for low-hanging improvements here (automatic
/// pipes, /dev/null, etc.) but this was purpose built for a specific need so
/// it doesn't have all of those.
///
/// The DBus path is exercised against a mock `org.freedesktop.Flatpak`
/// service in `tests/flatpak_mock.rs` (run under `dbus-run-session`).
pub struct FlatpakHostCommand {
    /// Argv are the arguments to call on the host with argv[0] being
    /// the command to execute.
    pub argv: Vec<String>,

    /// The cwd for the new process. If this is not set then it will use
    /// the current cwd of the calling process.
    pub cwd: Option<crate::osbytes::EncodedOs>,

    /// Environment variables for the child process. If this is None, this
    /// does not send any environment variables.
    pub env: Option<crate::env::EnvMap>,

    /// File descriptors to send to the child process. It is up to the
    /// caller to create the file descriptors and set them up. They are
    /// duplicated when spawning so the caller keeps ownership.
    pub stdin: OwnedFd,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,

    /// Connection kept for sending signals.
    shared: Arc<Shared>,
}

impl FlatpakHostCommand {
    pub fn new(argv: Vec<String>, stdin: OwnedFd, stdout: OwnedFd, stderr: OwnedFd) -> Self {
        Self {
            argv,
            cwd: None,
            env: None,
            stdin,
            stdout,
            stderr,
            shared: Arc::new(Shared {
                state: Mutex::new(State::Init),
                cv: Condvar::new(),
            }),
        }
    }

    /// Spawn the command. This will start the host command. On return,
    /// the pid will be available. This must only be called with the
    /// state in "init".
    pub fn spawn(&mut self) -> Result<u32, Error> {
        let request = Request {
            argv: self.argv.clone(),
            cwd: self.cwd.clone(),
            // The Development.HostCommand DBus method takes `a{ss}`, which
            // can only carry UTF-8, so non-UTF-8 bytes are replaced here.
            env: self
                .env
                .iter()
                .flat_map(crate::env::EnvMap::iter)
                .map(|(k, v)| (glib_env_string(k), glib_env_string(v)))
                .collect(),
            fds: [
                self.stdin
                    .try_clone()
                    .map_err(|_| Error::FlatpakSetupFail)?,
                self.stdout
                    .try_clone()
                    .map_err(|_| Error::FlatpakSetupFail)?,
                self.stderr
                    .try_clone()
                    .map_err(|_| Error::FlatpakSetupFail)?,
            ],
        };
        let shared = Arc::clone(&self.shared);

        // We don't track this thread, it will terminate on its own on
        // command exit.
        std::thread::Builder::new()
            .name("flatpak-host-command".into())
            .spawn(move || thread_main(&shared, request))
            .map_err(|_| Error::FlatpakSpawnFail)?;

        // Wait for the process to start or error.
        let mut guard = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *guard == State::Init {
            guard = self
                .shared
                .cv
                .wait(guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        match *guard {
            State::Init => unreachable!(),
            State::Err => Err(Error::FlatpakSpawnFail),
            State::Started { pid } | State::Exited { pid, .. } => Ok(pid),
        }
    }

    /// Wait for the process to end and return the exit status. This
    /// can only be called ONCE. Once this returns, the state is reset.
    pub fn wait(&mut self) -> Result<u8, Error> {
        let mut guard = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match *guard {
                State::Init => return Err(Error::FlatpakMustBeStarted),
                State::Err => return Err(Error::FlatpakSpawnFail),
                State::Started { .. } => {}
                State::Exited { status, .. } => {
                    *guard = State::Init;
                    self.shared.cv.notify_all();
                    return Ok(status);
                }
            }

            guard = self
                .shared
                .cv
                .wait(guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Wait for the process to end asynchronously. The callback is invoked
    /// from a helper thread with the result. This can only be called ONCE.
    ///
    /// This replaces `waitXev` (libxev completion) of the Zig version.
    pub fn wait_callback(
        mut self,
        callback: impl FnOnce(Result<u8, Error>) + Send + 'static,
    ) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("flatpak-host-wait".into())
            .spawn(move || callback(self.wait()))?;
        Ok(())
    }

    /// Send a signal to the started command. This does nothing if the
    /// command is not in the started state.
    pub fn signal(&self, sig: u8, pg: bool) -> Result<(), Error> {
        self.signal_handle().signal(sig, pg)
    }

    /// A cloneable handle that can signal this command even after the
    /// command itself has been moved into an exit watcher
    /// (`wait_callback`). Zig's `FlatpakHostCommand` is a pointer shared
    /// between the exit watcher and `Exec.stop`; this is the equivalent.
    pub fn signal_handle(&self) -> FlatpakSignalHandle {
        FlatpakSignalHandle {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Handle used to signal a host command independent of its owner.
#[derive(Clone)]
pub struct FlatpakSignalHandle {
    shared: Arc<Shared>,
}

impl FlatpakSignalHandle {
    fn state(&self) -> State {
        *self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Send a signal to the started command. This does nothing if the
    /// command is not in the started state.
    pub fn signal(&self, sig: u8, pg: bool) -> Result<(), Error> {
        let State::Started { pid } = self.state() else {
            return Ok(());
        };

        // Get our bus connection.
        let bus = Connection::session().map_err(|e| {
            log::warn!("signal error getting bus: {e}");
            Error::FlatpakSetupFail
        })?;

        if let Err(e) = bus.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "HostCommandSignal",
            &(pid, u32::from(sig), pg),
        ) {
            log::warn!("signal send error: {e}");
        }
        Ok(())
    }
}

/// Decode a reversible config string to the original bytes plus NUL, the
/// D-Bus `ay` form of a path/argument.
fn nul_terminated_bytes(s: &crate::osbytes::EncodedOs) -> Vec<u8> {
    let mut v = s.to_bytes();
    v.push(0);
    v
}

struct Request {
    argv: Vec<String>,
    cwd: Option<crate::osbytes::EncodedOs>,
    env: HashMap<String, String>,
    fds: [OwnedFd; 3],
}

fn thread_main(shared: &Shared, request: Request) {
    // Get our bus connection. This has to remain active until we exit
    // the thread otherwise our signals won't be called.
    let bus = match Connection::session() {
        Ok(b) => b,
        Err(e) => {
            log::warn!("spawn error getting bus: {e}");
            shared.update(State::Err);
            return;
        }
    };

    // Spawn the command first. This will setup all our IO.
    match start(shared, &bus, request) {
        Ok(iter_and_pid) => {
            let (iter, pid) = iter_and_pid;
            wait_for_exit(shared, iter, pid);
        }
        Err(err) => {
            log::warn!("error starting host command: {err}");
            shared.update(State::Err);
        }
    }
}

/// Start the command. This will start the host command and set the
/// pid on success. This will not wait for completion.
fn start(
    shared: &Shared,
    bus: &Connection,
    request: Request,
) -> Result<(MessageIterator, u32), Error> {
    // Build our file descriptor map: 0 -> stdin, 1 -> stdout, 2 -> stderr.
    let mut fds: HashMap<u32, Fd<'_>> = HashMap::new();
    for (i, fd) in request.fds.iter().enumerate() {
        fds.insert(i as u32, Fd::from(fd.as_fd()));
    }

    // Build our args (arrays of NUL-terminated byte strings).
    let args: Vec<Vec<u8>> = request
        .argv
        .iter()
        .map(|a| {
            let mut v = crate::osbytes::config_str_to_bytes(a);
            v.push(0);
            v
        })
        .collect();

    // Get the cwd in case we don't have ours set.
    // D-Bus `ay`: the original bytes (request.cwd carries the reversible
    // config-string encoding).
    let cwd = if let Some(c) = request.cwd {
        nul_terminated_bytes(&c)
    } else {
        let mut v = crate::osbytes::os_bytes(
            std::env::current_dir()
                .map_err(|_| Error::FlatpakSetupFail)?
                .as_os_str(),
        );
        v.push(0);
        v
    };

    // Subscribe to exit notifications (before the call so we can't miss it).
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(BUS_NAME)
        .and_then(|b| b.interface(INTERFACE))
        .and_then(|b| b.member("HostCommandExited"))
        .and_then(|b| b.path(OBJECT_PATH))
        .map(zbus::match_rule::Builder::build)
        .map_err(|e| {
            log::warn!("error building match rule: {e}");
            Error::FlatpakSetupFail
        })?;
    let iter = MessageIterator::for_match_rule(rule, bus, None).map_err(|e| {
        log::warn!("error subscribing to HostCommandExited: {e}");
        Error::FlatpakSetupFail
    })?;

    // Go!
    let reply = bus
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "HostCommand",
            &(cwd, args, fds, request.env, 0u32),
        )
        .map_err(|e| {
            log::warn!("Flatpak.HostCommand failed: {e}");
            Error::FlatpakRPCFail
        })?;

    let pid: u32 = reply.body().deserialize().map_err(|e| {
        log::warn!("Flatpak.HostCommand bad reply: {e}");
        Error::FlatpakRPCFail
    })?;
    log::debug!("HostCommand started pid={pid}");

    shared.update(State::Started { pid });
    Ok((iter, pid))
}

/// Block on the exit signal for our pid and publish the exit status.
fn wait_for_exit(shared: &Shared, iter: MessageIterator, pid: u32) {
    for msg in iter {
        let Ok(msg) = msg else { continue };
        let Ok((exit_pid, exit_status_raw)) = msg.body().deserialize::<(u32, u32)>() else {
            continue;
        };
        if exit_pid != pid {
            continue;
        }

        // posix.W.EXITSTATUS
        let status = ((exit_status_raw >> 8) & 0xff) as u8;
        shared.update(State::Exited { pid, status });
        log::debug!("HostCommand exited pid={pid} status={status}");
        return;
    }

    // The connection closed without an exit signal.
    log::warn!("host command signal stream ended without exit pid={pid}");
    shared.update(State::Err);
}

#[cfg(test)]
mod tests {
    #[test]
    fn dbus_bytes_are_decoded() {
        let enc = crate::osbytes::EncodedOs::from_bytes(b"/tmp/\xff\xfe");
        assert_ne!(enc.as_encoded().as_bytes(), b"/tmp/\xff\xfe");
        assert_eq!(super::nul_terminated_bytes(&enc), b"/tmp/\xff\xfe\0");
    }

    use super::*;

    #[test]
    fn is_flatpak_matches_flatpak_info() {
        assert_eq!(
            is_flatpak(),
            std::path::Path::new("/.flatpak-info").exists()
        );
    }

    #[test]
    fn wait_before_spawn_errors() {
        let (r, w) = crate::pipe::pipe().unwrap();
        let w2 = w.try_clone().unwrap();
        let mut cmd = FlatpakHostCommand::new(vec!["true".into()], r, w, w2);
        assert_eq!(cmd.wait(), Err(Error::FlatpakMustBeStarted));
        // Signalling an unstarted command is a no-op.
        assert_eq!(cmd.signal(15, false), Ok(()));
    }

    #[test]
    fn signal_handle_survives_wait_callback() {
        let (r, w) = crate::pipe::pipe().unwrap();
        let w2 = w.try_clone().unwrap();
        let cmd = FlatpakHostCommand::new(vec!["true".into()], r, w, w2);
        let handle = cmd.signal_handle();
        let (tx, rx) = std::sync::mpsc::channel();
        cmd.wait_callback(move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        // The watcher owns the command; the handle still works (no-op
        // since never started) and the watcher reports the state error.
        assert_eq!(handle.signal(1, true), Ok(()));
        assert_eq!(rx.recv().unwrap(), Err(Error::FlatpakMustBeStarted));
    }

    #[test]
    fn exit_status_extraction() {
        // Mirrors posix.W.EXITSTATUS on a raw wait status.
        assert_eq!(((3u32 << 8) >> 8) & 0xff, 3);
    }
}

#[cfg(test)]
mod byte_string_tests {
    #[test]
    fn glib_env_replaces_the_whole_invalid_value() {
        use crate::osbytes::EncodedOs;
        assert_eq!(
            super::glib_env_string(&EncodedOs::from_bytes(b"before\xffafter").to_os()),
            "[Invalid UTF-8]"
        );
        let literal = "x\u{fdd0}é";
        assert_eq!(
            super::glib_env_string(std::ffi::OsStr::new(literal)),
            literal
        );
    }
}
