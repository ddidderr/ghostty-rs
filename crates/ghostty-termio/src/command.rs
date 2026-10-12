//! Command launches sub-processes. This is an alternate implementation to
//! `std::process::Command` (mirroring why the Zig one exists) since we need
//! precise control over what happens between fork and exec (e.g. attaching
//! a shell to a pty, cgroup transitions, best-effort chdir).
//!
//! Port of `Command.zig` (POSIX/Linux only) plus the Linux cgroup pre-exec /
//! post-fork logic from `apprt/gtk/{pre_exec,post_fork}.zig`.
//!
//! Safety: the child side of [`Command::start`] runs between `fork` and `exec`
//! in a possibly multithreaded process. All allocation is done before the
//! fork; the child only performs raw syscalls plus the user supplied
//! `os_pre_exec` / `rt_pre_exec` hooks (which must therefore be
//! async-signal-safe, like their Zig counterparts are expected to be).

use std::{
    ffi::{CString, NulError, c_char},
    os::{fd::OwnedFd, unix::ffi::OsStrExt},
    panic::{AssertUnwindSafe, catch_unwind},
};

use ghostty_os::file::Rlimit;
use rustix::{
    io::Errno,
    process::{self, WaitOptions},
    stdio,
};

/// Function executed /in the child process/ after the fork, but before
/// exec'ing the command. If the function returns `Some(code)`, the child
/// process will be exited with that exit code.
///
/// Must be async-signal-safe (see module docs).
pub type PreExecFn = Box<dyn Fn() -> Option<u8> + Send + Sync>;

/// Allowable set of errors that can be returned by a post fork function. Any
/// errors will result in the failure to create the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("post fork hook failed")]
pub struct PostForkError;

/// Function executed /in the parent process/ after the fork. It receives the
/// command (the pid is already set).
pub type PostForkFn = Box<dyn Fn(&Command) -> Result<(), PostForkError> + Send + Sync>;

/// Errors from [`Command::start`] (all happen in the parent; the child never
/// returns from `start`, it either execs or exits like Zig's
/// `ExecFailedInChild` handling).
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("argument or environment contains an interior NUL byte")]
    InvalidArgument(#[from] NulError),
    #[error("fork failed: {0}")]
    Fork(Errno),
    #[error(transparent)]
    PostFork(#[from] PostForkError),
}

/// The various methods a process may exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Exited by normal exit call, value is exit status
    Exited(u8),

    /// Exited by a signal, value is the signal
    Signal(u32),

    /// Exited by a stop signal, value is signal
    Stopped(u32),

    /// Unknown exit reason, value is the status from waitpid
    Unknown(u32),
}

impl Exit {
    /// Decode a raw `waitpid` status (`W.IFEXITED` etc).
    pub fn init(status: u32) -> Exit {
        let low = status & 0x7f;
        if low == 0 {
            Exit::Exited(((status >> 8) & 0xff) as u8)
        } else if (((low + 1) as u8 as i8) >> 1) > 0 {
            Exit::Signal(low)
        } else if (status & 0xff) == 0x7f {
            Exit::Stopped((status >> 8) & 0xff)
        } else {
            Exit::Unknown(status)
        }
    }
}

pub use ghostty_config::types_b::{GtkSingleInstance, LinuxCgroup};

/// Configuration information needed by the apprt pre exec function
/// (`apprt/gtk/pre_exec.zig` `PreExecInfo`). Trivially copyable.
#[derive(Debug, Clone, Copy)]
pub struct RtPreExecInfo {
    pub gtk_single_instance: GtkSingleInstance,
    pub linux_cgroup: LinuxCgroup,
    pub linux_cgroup_hard_fail: bool,
}

impl Default for RtPreExecInfo {
    /// Config defaults (`linux-cgroup = single-instance`,
    /// `gtk-single-instance = detect`).
    fn default() -> Self {
        Self {
            gtk_single_instance: GtkSingleInstance::Detect,
            linux_cgroup: LinuxCgroup::SingleInstance,
            linux_cgroup_hard_fail: false,
        }
    }
}

impl RtPreExecInfo {
    /// `PreExecInfo.init(cfg)`.
    pub fn init(cfg: &ghostty_config::config::Config) -> Self {
        Self {
            gtk_single_instance: cfg.gtk_single_instance,
            linux_cgroup: cfg.linux_cgroup,
            linux_cgroup_hard_fail: cfg.linux_cgroup_hard_fail,
        }
    }
}

/// Configuration information needed by the apprt post fork function
/// (`apprt/gtk/post_fork.zig` `PostForkInfo`).
#[derive(Debug, Clone, Copy)]
pub struct RtPostForkInfo {
    pub gtk_single_instance: GtkSingleInstance,
    pub linux_cgroup: LinuxCgroup,
    pub linux_cgroup_hard_fail: bool,
    pub linux_cgroup_memory_limit: Option<u64>,
    pub linux_cgroup_processes_limit: Option<u64>,
}

impl Default for RtPostForkInfo {
    fn default() -> Self {
        Self {
            gtk_single_instance: GtkSingleInstance::Detect,
            linux_cgroup: LinuxCgroup::SingleInstance,
            linux_cgroup_hard_fail: false,
            linux_cgroup_memory_limit: None,
            linux_cgroup_processes_limit: None,
        }
    }
}

impl RtPostForkInfo {
    /// `PostForkInfo.init(cfg)`.
    pub fn init(cfg: &ghostty_config::config::Config) -> Self {
        Self {
            gtk_single_instance: cfg.gtk_single_instance,
            linux_cgroup: cfg.linux_cgroup,
            linux_cgroup_hard_fail: cfg.linux_cgroup_hard_fail,
            linux_cgroup_memory_limit: cfg.linux_cgroup_memory_limit,
            linux_cgroup_processes_limit: cfg.linux_cgroup_processes_limit,
        }
    }
}

/// A command to launch.
#[derive(Default)]
pub struct Command {
    /// Path to the command to run. This doesn't have to be an absolute path,
    /// because we use exec functions that search the PATH, if necessary.
    pub path: String,

    /// Command-line arguments. It is the responsibility of the caller to set
    /// args[0] to the command. If args is empty then args[0] will
    /// automatically be set to equal path.
    pub args: Vec<String>,

    /// Environment variables for the child process. If this is `None`,
    /// inherits the environment variables from this process. These are the
    /// exact environment variables to set; these are /not/ merged.
    pub env: Option<ghostty_os::EnvMap>,

    /// Working directory to change to in the child process. If not set, the
    /// working directory of the calling process is preserved. Failure to
    /// change directory is ignored (as in Ghostty).
    pub cwd: Option<String>,

    /// The file handle to set for stdin/out/err. If this isn't set, we do
    /// nothing explicitly so it is up to the behavior of the operating system.
    /// (The fds are duplicated onto 0/1/2 in the child; the caller keeps its
    /// own handles by passing `try_clone()`d fds.)
    pub stdin: Option<OwnedFd>,
    pub stdout: Option<OwnedFd>,
    pub stderr: Option<OwnedFd>,

    /// Rlimits to restore in the child (Ghostty raises RLIMIT_NOFILE at
    /// startup; replaces `global_state.rlimits.restore()`).
    /// TODO(integration): fill from the global state.
    pub restore_rlimit: Option<Rlimit>,

    /// Executed /in the child process/ after fork but before exec. Useful to
    /// setup some state in the child before the exec process takes over, such
    /// as signal handlers, setsid, setuid, etc.
    pub os_pre_exec: Option<PreExecFn>,

    /// Same as `os_pre_exec` but runs afterwards and is meant for the apprt
    /// (e.g. [`linux_cgroup_pre_exec`]).
    pub rt_pre_exec: Option<PreExecFn>,

    /// Executed /in the parent process/ after the fork.
    pub rt_post_fork: Option<PostForkFn>,

    /// Process ID is set after start is called.
    pub pid: Option<i32>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("path", &self.path)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

/// Everything that must be allocated before forking.
struct Prepared {
    path: CString,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl Command {
    pub fn new(path: impl Into<String>, args: &[&str]) -> Self {
        Self {
            path: path.into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn prepare(&self) -> Result<Prepared, NulError> {
        let path = CString::new(self.path.as_str())?;
        let argv = if self.args.is_empty() {
            vec![path.clone()]
        } else {
            self.args
                .iter()
                .map(|a| CString::new(a.as_str()))
                .collect::<Result<_, _>>()?
        };
        let envp = match &self.env {
            Some(map) => map
                .iter()
                .map(|(k, v)| {
                    let mut b = k.as_bytes().to_vec();
                    b.push(b'=');
                    b.extend_from_slice(v.as_bytes());
                    CString::new(b)
                })
                .collect::<Result<_, _>>()?,
            // Inherit: snapshot our environment before the fork (nothing may
            // allocate in the child).
            None => std::env::vars_os()
                .map(|(k, v)| {
                    let mut b = k.into_encoded_bytes();
                    b.push(b'=');
                    b.extend_from_slice(v.as_encoded_bytes());
                    CString::new(b)
                })
                .collect::<Result<_, _>>()?,
        };
        Ok(Prepared { path, argv, envp })
    }

    /// Start the subprocess. This returns immediately once the child is
    /// started.
    ///
    /// After this is successful, `self.pid` is available.
    pub fn start(&mut self) -> Result<(), StartError> {
        // IMPORTANT: do all allocation prior to the fork().
        let prep = self.prepare()?;
        let cwd = self.cwd.as_deref().and_then(|c| CString::new(c).ok());

        let mut argv_ptrs: Vec<*const c_char> = prep.argv.iter().map(|c| c.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());
        let mut envp_ptrs: Vec<*const c_char> = prep.envp.iter().map(|c| c.as_ptr()).collect();
        envp_ptrs.push(std::ptr::null());

        let pid = fork().map_err(StartError::Fork)?;
        if pid != 0 {
            // Parent, return immediately.
            self.pid = Some(pid);
            if let Some(f) = &self.rt_post_fork {
                f(self)?;
            }
            return Ok(());
        }

        // We are the child. This never returns.
        self.run_child(&prep, &argv_ptrs, &envp_ptrs, cwd.as_deref())
    }

    /// The child side of `start`: set up fds, chdir, rlimits, hooks, exec.
    fn run_child(
        &self,
        prep: &Prepared,
        argv: &[*const c_char],
        envp: &[*const c_char],
        cwd: Option<&std::ffi::CStr>,
    ) -> ! {
        // Setup our file descriptors for std streams. Any failure here is
        // equivalent to Zig's `ExecFailedInChild` -> the child exits 1.
        if let Some(fd) = &self.stdin
            && setup_fd(|f| stdio::dup2_stdin(f), fd).is_err()
        {
            exit_immediately(1);
        }
        if let Some(fd) = &self.stdout
            && setup_fd(|f| stdio::dup2_stdout(f), fd).is_err()
        {
            exit_immediately(1);
        }
        if let Some(fd) = &self.stderr
            && setup_fd(|f| stdio::dup2_stderr(f), fd).is_err()
        {
            exit_immediately(1);
        }

        // Setup our working directory. This can fail if we don't have
        // permission to go to this directory or if due to race conditions it
        // doesn't exist or any various other reasons. We don't want to crash
        // the entire process if this fails so we ignore it. We don't log
        // because that'll show up in the output.
        if let Some(cwd) = cwd {
            let _ = process::chdir(cwd);
        }

        // Restore any rlimits that were set by Ghostty. This might fail but
        // any failures are ignored (its best effort).
        if let Some(lim) = self.restore_rlimit {
            ghostty_os::restore_max_files(lim);
        }

        // If there are pre exec callbacks, call them now.
        for hook in [&self.os_pre_exec, &self.rt_pre_exec].into_iter().flatten() {
            match catch_unwind(AssertUnwindSafe(hook)) {
                Ok(Some(code)) => exit_immediately(code),
                Ok(None) => {}
                Err(_) => exit_immediately(1),
            }
        }

        // Finally, replace our process.
        // Note: we must use the "p"-variant of exec here because we do not
        // guarantee our command is looked up already in the path.
        let err = execvpe(prep, argv, envp);

        // If we are executing this code, the exec failed. We're in the child
        // process so there isn't much we can do. We try to output something
        // reasonable, then exit (Zig returns ExecFailedInChild which the
        // caller turns into exit(1)).
        if err == Errno::NOENT {
            write_stderr(
                b"Requested executable not found. Please verify the command is on\n\
                  the PATH and try again.\n",
            );
        } else {
            let mut buf = [0u8; 96];
            let mut n = 0;
            let prefix = b"exec syscall failed with unexpected error: errno=";
            buf[..prefix.len()].copy_from_slice(prefix);
            n += prefix.len();
            n += format_u32(err.raw_os_error() as u32, &mut buf[n..]);
            buf[n] = b'\n';
            n += 1;
            write_stderr(&buf[..n]);
        }
        exit_immediately(1)
    }

    /// Wait for the command to exit and return information about how it
    /// exited. With `block == false` this still loops until the child has a
    /// status to report (see the comment in the body).
    pub fn wait(&self, block: bool) -> Result<Exit, Errno> {
        let raw = self.pid.expect("wait called before start");
        // SAFETY of unwrap-free construction: pids are nonzero after fork.
        let pid = process::Pid::from_raw(raw).ok_or(Errno::SRCH)?;
        let opts = if block {
            WaitOptions::empty()
        } else {
            // We specify NOHANG because its not our fault if the process we
            // launch for the tty doesn't properly waitpid its children. We
            // don't want to hang the terminal over it. When NOHANG is
            // specified, waitpid returns "nothing" if the process doesn't
            // have a status to report. When that happens, it is as though the
            // wait call has not been performed, so we need to keep trying
            // until we get a status, otherwise we end up with zombie
            // processes.
            WaitOptions::NOHANG
        };
        loop {
            match process::waitpid(Some(pid), opts) {
                Ok(Some((_pid, status))) => return Ok(Exit::init(status.as_raw() as u32)),
                Ok(None) => std::thread::yield_now(),
                Err(Errno::INTR) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

fn setup_fd(dup: impl Fn(&OwnedFd) -> rustix::io::Result<()>, src: &OwnedFd) -> Result<(), Errno> {
    // dup2 (like Zig's dup3 with flags=0) leaves the target without
    // CLOEXEC: we do NOT want this file descriptor to be closed on exec.
    loop {
        match dup(src) {
            Err(Errno::INTR) => {}
            r => return r,
        }
    }
}

/// Exit the current (forked child) process immediately without running
/// destructors or atexit handlers (`_exit`). Use this from `os_pre_exec` /
/// `rt_pre_exec` hooks instead of `std::process::exit`.
#[allow(unsafe_code)]
pub fn exit_immediately(code: u8) -> ! {
    // SAFETY: _exit is async-signal-safe and has no preconditions.
    unsafe { libc::_exit(i32::from(code)) }
}

#[allow(unsafe_code)]
fn fork() -> Result<i32, Errno> {
    // SAFETY: the child branch (in `Command::start`) only performs
    // async-signal-safe operations on memory allocated before the fork and
    // then execs or `_exit`s; it never returns into Rust code that could
    // allocate or take locks.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        Err(Errno::from_raw_os_error(
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ))
    } else {
        Ok(pid)
    }
}

#[allow(unsafe_code)]
fn execvpe(prep: &Prepared, argv: &[*const c_char], envp: &[*const c_char]) -> Errno {
    debug_assert!(argv.last().is_some_and(|p| p.is_null()));
    debug_assert!(envp.last().is_some_and(|p| p.is_null()));
    // SAFETY: path is a valid NUL-terminated string; argv/envp are
    // NULL-terminated arrays of pointers into CStrings (owned by `prep`) that
    // outlive the call. execvpe only returns on failure.
    unsafe {
        libc::execvpe(prep.path.as_ptr(), argv.as_ptr(), envp.as_ptr());
    }
    Errno::from_raw_os_error(errno_now())
}

#[allow(unsafe_code)]
fn errno_now() -> i32 {
    // SAFETY: __errno_location returns a valid thread-local pointer.
    unsafe { *libc::__errno_location() }
}

fn write_stderr(msg: &[u8]) {
    // Best effort, no allocation.
    let _ = rustix::io::write(borrow_stderr(), msg);
}

#[allow(unsafe_code)]
fn borrow_stderr() -> rustix::fd::BorrowedFd<'static> {
    // SAFETY: STDERR_FILENO is valid for the life of the process (or the
    // write simply fails with EBADF).
    unsafe { rustix::fd::BorrowedFd::borrow_raw(libc::STDERR_FILENO) }
}

/// Format `v` in decimal into `out`, returning the number of bytes written.
fn format_u32(mut v: u32, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 10];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let n = tmp.len() - i;
    out[..n].copy_from_slice(&tmp[i..]);
    n
}

// ---------------------------------------------------------------------------
// Linux cgroup transitions (apprt/gtk/pre_exec.zig, post_fork.zig, cgroup.zig)
// ---------------------------------------------------------------------------

/// The name of the transient systemd scope for the given child pid
/// (`apprt/gtk/cgroup.zig` `fmtScope`).
pub fn fmt_scope(pid: u32) -> String {
    format!("app-ghostty-surface-transient-{pid}.scope")
}

use ghostty_os::cgroup::wait_for_scope as wait_for_cgroup;

/// If we are expecting to be moved to a transient systemd scope, wait to see
/// if that happens by checking for the correct name of the current cgroup.
/// Wait at most 250ms so that we don't overly delay the soft-fail scenario.
///
/// If we are configured to hard fail, log an error message and return an
/// exit code if we don't detect the move in time.
///
/// Intended as an `rt_pre_exec` hook (runs in the child; note that, like the
/// Zig code, it allocates and sleeps).
pub fn linux_cgroup_pre_exec(info: &RtPreExecInfo) -> Option<u8> {
    match info.linux_cgroup {
        LinuxCgroup::Always => {}
        LinuxCgroup::Never => return None,
        LinuxCgroup::SingleInstance => match info.gtk_single_instance {
            GtkSingleInstance::True => {}
            GtkSingleInstance::False => return None,
            GtkSingleInstance::Detect => {
                log::error!("gtk-single-instance is set to detect");
                return Some(127);
            }
        },
    }

    let pid = process::getpid().as_raw_nonzero().get() as u32;
    let expected = fmt_scope(pid);

    if !wait_for_cgroup(pid, &expected) && info.linux_cgroup_hard_fail {
        log::error!("transition to new transient systemd scope took too long");
        return Some(127);
    }
    None
}

/// If we are configured to do so, tell `systemd` to move the new child PID
/// into a transient `systemd` scope with the configured resource limits.
///
/// `create_scope(pid, memory_high, tasks_max)` performs the D-Bus call
/// (`cgroup.createScope`); it returns `None` if there is no D-Bus connection
/// and `Some(Err(msg))` on failure.
/// TODO(integration): wire to the apprt-gtk D-Bus connection.
pub fn linux_cgroup_post_fork(
    cmd: &Command,
    info: &RtPostForkInfo,
    create_scope: impl FnOnce(u32, Option<u64>, Option<u64>) -> Option<Result<(), String>>,
) -> Result<(), PostForkError> {
    match info.linux_cgroup {
        LinuxCgroup::Always => {}
        LinuxCgroup::Never => return Ok(()),
        LinuxCgroup::SingleInstance => match info.gtk_single_instance {
            GtkSingleInstance::True => {}
            GtkSingleInstance::False => return Ok(()),
            GtkSingleInstance::Detect => {
                log::error!("gtk-single-instance is set to detect which should be impossible!");
                return Err(PostForkError);
            }
        },
    }

    let Some(pid) = cmd.pid else {
        log::error!("PID of child not known!");
        return Err(PostForkError);
    };
    let pid = pid as u32;
    let expected = fmt_scope(pid);
    log::debug!("beginning transition to transient systemd scope {expected}");

    match create_scope(
        pid,
        info.linux_cgroup_memory_limit,
        info.linux_cgroup_processes_limit,
    ) {
        None => {
            if info.linux_cgroup_hard_fail {
                log::error!("dbus connection required for cgroup isolation, exiting");
                return Err(PostForkError);
            }
            return Ok(());
        }
        Some(Err(err)) => {
            if info.linux_cgroup_hard_fail {
                log::error!("unable to create transient systemd scope {expected}: {err}");
                return Err(PostForkError);
            }
            log::warn!("unable to create transient systemd scope {expected}: {err}");
            return Ok(());
        }
        Some(Ok(())) => {}
    }

    if wait_for_cgroup(pid, &expected) {
        log::debug!("transition to transient systemd scope {expected} complete");
    } else {
        if info.linux_cgroup_hard_fail {
            log::error!("transition to new transient systemd scope {expected} took too long");
            return Err(PostForkError);
        }
        log::warn!("transition to transient systemd scope {expected} took too long");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::{Read, Seek, SeekFrom},
        os::fd::AsRawFd,
        sync::Arc,
    };

    use ghostty_os::TempDir;

    use super::*;

    fn test_file(td: &TempDir, name: &str) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(td.path().join(name))
            .unwrap()
    }

    fn read_all(mut f: File) -> String {
        f.seek(SeekFrom::Start(0)).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        s
    }

    fn sh(script: &str) -> Command {
        Command::new("/bin/sh", &["/bin/sh", "-c", script])
    }

    #[test]
    fn create_null_delimited_env_map() {
        // The Zig test checks the null-delimited env block; here the
        // equivalent is `prepare()`.
        let mut cmd = Command::new("/bin/sh", &[]);
        cmd.env = Some(
            [
                ("HOME", "/home/ifreund"),
                ("WAYLAND_DISPLAY", "wayland-1"),
                ("DISPLAY", ":1"),
                ("DEBUGINFOD_URLS", " "),
                ("XCURSOR_SIZE", "24"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        );
        let prep = cmd.prepare().unwrap();
        assert_eq!(prep.envp.len(), 5);
        for target in [
            "HOME=/home/ifreund",
            "WAYLAND_DISPLAY=wayland-1",
            "DISPLAY=:1",
            "DEBUGINFOD_URLS= ",
            "XCURSOR_SIZE=24",
        ] {
            assert!(prep.envp.iter().any(|v| v.to_str().unwrap() == target));
        }
        // Empty args fall back to path as args[0].
        assert_eq!(prep.argv.len(), 1);
    }

    fn pre_exec_exit_test(cmd: &mut Command) {
        cmd.start().unwrap();
        assert!(cmd.pid.is_some());
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(42));
    }

    #[test]
    fn os_pre_exec_1() {
        let mut cmd = Command::new("/bin/sh", &["/bin/sh", "-v"]);
        // This runs in the child, so we can exit and it won't kill the test
        // runner.
        cmd.os_pre_exec = Some(Box::new(|| exit_immediately(42)));
        pre_exec_exit_test(&mut cmd);
    }

    #[test]
    fn os_pre_exec_2() {
        let mut cmd = Command::new("/bin/sh", &["/bin/sh", "-v"]);
        cmd.os_pre_exec = Some(Box::new(|| Some(42)));
        pre_exec_exit_test(&mut cmd);
    }

    #[test]
    fn rt_pre_exec_1() {
        let mut cmd = Command::new("/bin/sh", &["/bin/sh", "-v"]);
        cmd.rt_pre_exec = Some(Box::new(|| exit_immediately(42)));
        pre_exec_exit_test(&mut cmd);
    }

    #[test]
    fn rt_pre_exec_2() {
        let mut cmd = Command::new("/bin/sh", &["/bin/sh", "-v"]);
        cmd.rt_pre_exec = Some(Box::new(|| Some(42)));
        pre_exec_exit_test(&mut cmd);
    }

    #[test]
    fn rt_post_fork_1() {
        let mut cmd = sh("sleep 1");
        cmd.rt_post_fork = Some(Box::new(|_| Err(PostForkError)));
        assert!(matches!(
            cmd.start(),
            Err(StartError::PostFork(PostForkError))
        ));
        // Reap the child that was started anyway (like the Zig test leaks it).
        if let Some(pid) = cmd.pid.and_then(process::Pid::from_raw) {
            let _ = process::kill_process(pid, process::Signal::KILL);
        }
        let _ = cmd.wait(true);
    }

    #[test]
    fn redirect_stdout_to_file() {
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");

        let mut cmd = sh("echo hello");
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.start().unwrap();
        assert!(cmd.pid.is_some());
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(0));

        assert!(!read_all(stdout).is_empty());
    }

    #[test]
    fn custom_env_vars() {
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");

        let mut cmd = sh("echo $VALUE");
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.env = Some(ghostty_os::EnvMap::from_iter([("VALUE", "hello")]));
        cmd.start().unwrap();
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(0));

        assert_eq!("hello\n", read_all(stdout));
    }

    #[test]
    fn non_utf8_env_value_is_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");

        let mut cmd = sh("printf %s \"$VALUE\"");
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        let mut env = ghostty_os::EnvMap::new();
        env.insert(
            "VALUE",
            std::ffi::OsString::from_vec(vec![b'a', 0xff, b'b']),
        );
        cmd.env = Some(env);
        cmd.start().unwrap();
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(0));

        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::fs::File::open(td.path().join("stdout.txt")).unwrap(),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(bytes, vec![b'a', 0xff, b'b']);
    }

    #[test]
    fn custom_working_directory() {
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");

        let mut cmd = sh("pwd");
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.cwd = Some("/tmp".to_owned());
        cmd.start().unwrap();
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(0));

        assert_eq!("/tmp\n", read_all(stdout));
    }

    #[test]
    fn invalid_cwd_is_ignored() {
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");

        let mut cmd = sh("echo ok");
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.cwd = Some("/definitely/not/a/dir".to_owned());
        cmd.start().unwrap();
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(0));
        assert_eq!("ok\n", read_all(stdout));
    }

    // Validate an exec failure correctly terminates the child (in Zig:
    // error.ExecFailedInChild must be handled or a second copy of the process
    // keeps running; here the child can never return into the caller).
    #[test]
    fn posix_fork_handles_execve_failure() {
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");
        let stderr = test_file(&td, "stderr.txt");

        let mut cmd = Command::new("/not/a/binary", &["/not/a/binary", ""]);
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.stderr = Some(stderr.try_clone().unwrap().into());
        cmd.cwd = Some("/bin".to_owned());
        cmd.start().unwrap();
        assert!(cmd.pid.is_some());
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(1));
        assert!(read_all(stderr).starts_with("Requested executable not found."));
    }

    #[test]
    fn wait_nonblocking_eventually_reports() {
        let mut cmd = sh("exit 3");
        cmd.start().unwrap();
        assert_eq!(cmd.wait(false).unwrap(), Exit::Exited(3));
    }

    #[test]
    fn exit_signal_and_decode() {
        let mut cmd = sh("kill -TERM $$");
        cmd.start().unwrap();
        assert_eq!(cmd.wait(true).unwrap(), Exit::Signal(15));

        assert_eq!(Exit::init(0), Exit::Exited(0));
        assert_eq!(Exit::init(42 << 8), Exit::Exited(42));
        assert_eq!(Exit::init(9), Exit::Signal(9));
        assert_eq!(Exit::init((19 << 8) | 0x7f), Exit::Stopped(19));
    }

    #[test]
    fn post_fork_sees_pid() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let seen2 = Arc::clone(&seen);
        let mut cmd = sh("true");
        cmd.rt_post_fork = Some(Box::new(move |c| {
            *seen2.lock().unwrap() = c.pid;
            Ok(())
        }));
        cmd.start().unwrap();
        assert_eq!(*seen.lock().unwrap(), cmd.pid);
        cmd.wait(true).unwrap();
    }

    #[test]
    fn stdio_fds_are_not_cloexec_in_child_and_closed_others() {
        // The master end of a pty is CLOEXEC so the child must not see it.
        let pty = crate::pty::Pty::open(crate::pty::Winsize::default()).unwrap();
        let master = pty.master.as_raw_fd();
        let td = TempDir::init().unwrap();
        let stdout = test_file(&td, "stdout.txt");
        let mut cmd = sh(&format!(
            "[ -e /proc/self/fd/{master} ] && echo leaked || echo ok"
        ));
        cmd.stdout = Some(stdout.try_clone().unwrap().into());
        cmd.start().unwrap();
        cmd.wait(true).unwrap();
        assert_eq!("ok\n", read_all(stdout));
    }

    #[test]
    fn cgroup_pre_exec_modes() {
        let never = RtPreExecInfo {
            linux_cgroup: LinuxCgroup::Never,
            ..Default::default()
        };
        assert_eq!(linux_cgroup_pre_exec(&never), None);
        let detect = RtPreExecInfo::default();
        assert_eq!(linux_cgroup_pre_exec(&detect), Some(127));
        let single_false = RtPreExecInfo {
            gtk_single_instance: GtkSingleInstance::False,
            ..Default::default()
        };
        assert_eq!(linux_cgroup_pre_exec(&single_false), None);
        assert_eq!(fmt_scope(1234), "app-ghostty-surface-transient-1234.scope");
    }

    #[test]
    fn cgroup_post_fork_modes() {
        let mut cmd = sh("true");
        cmd.pid = Some(1);
        let never = RtPostForkInfo {
            linux_cgroup: LinuxCgroup::Never,
            ..Default::default()
        };
        assert!(linux_cgroup_post_fork(&cmd, &never, |_, _, _| unreachable!()).is_ok());
        let detect = RtPostForkInfo::default();
        assert!(linux_cgroup_post_fork(&cmd, &detect, |_, _, _| unreachable!()).is_err());
        // No dbus: soft fail ok, hard fail err.
        let always = RtPostForkInfo {
            linux_cgroup: LinuxCgroup::Always,
            ..Default::default()
        };
        assert!(linux_cgroup_post_fork(&cmd, &always, |_, _, _| None).is_ok());
        let hard = RtPostForkInfo {
            linux_cgroup_hard_fail: true,
            ..always
        };
        assert!(linux_cgroup_post_fork(&cmd, &hard, |_, _, _| None).is_err());
        assert!(linux_cgroup_post_fork(&cmd, &hard, |_, _, _| Some(Err("x".into()))).is_err());
    }
}
