//! Linux PTY creation and management. This is just a thin layer on top
//! of Linux syscalls. Port of `pty.zig` (POSIX variant only; the Windows
//! and null variants are out of scope).

use std::os::fd::{AsRawFd, OwnedFd, RawFd};

use rustix::{fs, process, pty, termios};

/// Redeclare the winsize struct so we can just use a Rust struct. The
/// defaults on this are some reasonable screen size but you should probably
/// not use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl Default for Winsize {
    fn default() -> Self {
        Self {
            ws_row: 100,
            ws_col: 80,
            ws_xpixel: 800,
            ws_ypixel: 600,
        }
    }
}

impl From<Winsize> for termios::Winsize {
    fn from(w: Winsize) -> Self {
        Self {
            ws_row: w.ws_row,
            ws_col: w.ws_col,
            ws_xpixel: w.ws_xpixel,
            ws_ypixel: w.ws_ypixel,
        }
    }
}

impl From<termios::Winsize> for Winsize {
    fn from(w: termios::Winsize) -> Self {
        Self {
            ws_row: w.ws_row,
            ws_col: w.ws_col,
            ws_xpixel: w.ws_xpixel,
            ws_ypixel: w.ws_ypixel,
        }
    }
}

/// The modes of a pty. Not all of these modes are supported on
/// all platforms but all platforms share the same mode struct.
///
/// The default values of fields in this struct are set to the
/// most typical values for a pty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// ICANON on POSIX
    pub canonical: bool,

    /// ECHO on POSIX
    pub echo: bool,
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            canonical: true,
            echo: true,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("openpty failed")]
    OpenptyFailed,
    #[error("get mode failed")]
    GetModeFailed,
    #[error("ioctl failed")]
    IoctlFailed,
    #[error("operation not supported")]
    OperationNotSupported,
    #[error("setsid failed")]
    ProcessGroupFailed,
    #[error("setting the controlling terminal failed")]
    SetControllingTerminalFailed,
}

/// A pty master/slave pair.
///
/// Deviation from Zig: both fds are `OwnedFd` and are closed on drop (the Zig
/// struct never closes the slave automatically). To close the slave early
/// (as Exec does after the fork) use [`Pty::close_slave`] or destructure.
#[derive(Debug)]
pub struct Pty {
    pub master: OwnedFd,
    /// `None` once closed with [`Pty::close_slave`].
    pub slave: Option<OwnedFd>,
}

impl Pty {
    /// Open a new PTY with the given initial size.
    pub fn open(size: Winsize) -> Result<Pty, Error> {
        // Equivalent of glibc openpty(): the master is CLOEXEC (Ghostty sets
        // this explicitly: only the slave fd should be inherited by the
        // child), the slave is not.
        let master = pty::openpt(
            pty::OpenptFlags::RDWR | pty::OpenptFlags::NOCTTY | pty::OpenptFlags::CLOEXEC,
        )
        .map_err(|_| Error::OpenptyFailed)?;
        pty::grantpt(&master).map_err(|_| Error::OpenptyFailed)?;
        pty::unlockpt(&master).map_err(|_| Error::OpenptyFailed)?;
        let name = pty::ptsname(&master, Vec::new()).map_err(|_| Error::OpenptyFailed)?;
        let slave = fs::open(
            name.as_c_str(),
            fs::OFlags::RDWR | fs::OFlags::NOCTTY,
            fs::Mode::empty(),
        )
        .map_err(|_| Error::OpenptyFailed)?;

        termios::tcsetwinsize(&master, size.into()).map_err(|_| Error::OpenptyFailed)?;

        // Enable UTF-8 mode. I think this is on by default on Linux but it
        // is NOT on by default on macOS so we ensure that it is always set.
        let mut attrs = termios::tcgetattr(&master).map_err(|_| Error::OpenptyFailed)?;
        attrs.input_modes.insert(termios::InputModes::IUTF8);
        termios::tcsetattr(&master, termios::OptionalActions::Now, &attrs)
            .map_err(|_| Error::OpenptyFailed)?;

        Ok(Pty {
            master,
            slave: Some(slave),
        })
    }

    /// Close the slave side (the parent does this after forking the child).
    pub fn close_slave(&mut self) {
        self.slave = None;
    }

    /// Get the modes of the pty (read from the master).
    pub fn get_mode(&self) -> Result<Mode, Error> {
        get_mode_fd(&self.master)
    }

    /// Return the size of the pty.
    pub fn get_size(&self) -> Result<Winsize, Error> {
        termios::tcgetwinsize(&self.master)
            .map(Into::into)
            .map_err(|_| Error::IoctlFailed)
    }

    /// Set the size of the pty.
    pub fn set_size(&mut self, size: Winsize) -> Result<(), Error> {
        termios::tcsetwinsize(&self.master, size.into()).map_err(|_| Error::IoctlFailed)
    }

    /// Raw fds for use in the child's pre-exec hook (see [`child_pre_exec`]).
    pub fn raw_fds(&self) -> (RawFd, Option<RawFd>) {
        (
            self.master.as_raw_fd(),
            self.slave.as_ref().map(AsRawFd::as_raw_fd),
        )
    }

    /// This should be called prior to exec in the forked child process
    /// in order to setup the tty properly. See [`child_pre_exec`].
    pub fn child_pre_exec(&self) -> Result<(), Error> {
        let (master, slave) = self.raw_fds();
        child_pre_exec(master, slave.unwrap_or(-1))
    }
}

/// Read the termios mode from a pty master fd (what Exec's termios timer
/// does by rebuilding a `Pty` around the master in Zig).
pub fn get_mode_fd(master: impl std::os::fd::AsFd) -> Result<Mode, Error> {
    let attrs = termios::tcgetattr(master).map_err(|_| Error::GetModeFailed)?;
    Ok(Mode {
        canonical: attrs.local_modes.contains(termios::LocalModes::ICANON),
        echo: attrs.local_modes.contains(termios::LocalModes::ECHO),
    })
}

/// Signals reset to their default disposition in the child.
const RESET_SIGNALS: [nix::sys::signal::Signal; 13] = {
    use nix::sys::signal::Signal as S;
    [
        S::SIGABRT,
        S::SIGALRM,
        S::SIGBUS,
        S::SIGCHLD,
        S::SIGFPE,
        S::SIGHUP,
        S::SIGILL,
        S::SIGINT,
        S::SIGPIPE,
        S::SIGSEGV,
        S::SIGTRAP,
        S::SIGTERM,
        S::SIGQUIT,
    ]
};

/// Child-side setup of the tty: reset signals, `setsid`, `TIOCSCTTY` on the
/// slave and close the master/slave fds. Operates on raw fd numbers so it can
/// be used from a post-fork hook (`Command::os_pre_exec`) without touching
/// any `OwnedFd` destructors. Everything in here is async-signal-safe (no
/// allocation, no locks).
///
/// A negative `slave` skips the controlling-terminal step and the close.
pub fn child_pre_exec(master: RawFd, slave: RawFd) -> Result<(), Error> {
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet};

    // Reset our signals
    let sa = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    for sig in RESET_SIGNALS {
        reset_signal(sig, &sa);
    }

    // Create a new process group
    process::setsid().map_err(|_| Error::ProcessGroupFailed)?;

    if slave >= 0 {
        // Set controlling terminal
        set_controlling_terminal(slave)?;
    }

    // Can close master/slave pair now
    close_raw(slave);
    close_raw(master);

    Ok(())
}

#[allow(unsafe_code)]
fn reset_signal(sig: nix::sys::signal::Signal, sa: &nix::sys::signal::SigAction) {
    // SAFETY: sigaction(2) is async-signal-safe and SIG_DFL installs no
    // Rust handler code.
    let _ = unsafe { nix::sys::signal::sigaction(sig, sa) };
}

#[allow(unsafe_code)]
fn set_controlling_terminal(slave: RawFd) -> Result<(), Error> {
    use std::os::fd::BorrowedFd;
    // SAFETY: the fd is owned by the caller and stays open for the duration of
    // this call; we only borrow it for one ioctl.
    let fd = unsafe { BorrowedFd::borrow_raw(slave) };
    process::ioctl_tiocsctty(fd).map_err(|err| {
        log::error!("error setting controlling terminal errno={err:?}");
        Error::SetControllingTerminalFailed
    })
}

#[allow(unsafe_code)]
fn close_raw(fd: RawFd) {
    if fd < 0 {
        return;
    }
    // SAFETY: closing a raw fd number we were told to close; in the child
    // the `OwnedFd`s that nominally own these are never dropped (the child
    // execs or `_exit`s), so there is no double close.
    unsafe { rustix::io::close(fd) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_size() {
        let mut ws = Winsize {
            ws_row: 50,
            ws_col: 80,
            ws_xpixel: 1,
            ws_ypixel: 1,
        };

        let mut pty = Pty::open(ws).unwrap();

        // Initialize size should match what we gave it
        assert_eq!(ws, pty.get_size().unwrap());

        // Can set and read new sizes
        ws.ws_row *= 2;
        pty.set_size(ws).unwrap();
        assert_eq!(ws, pty.get_size().unwrap());
    }

    #[test]
    fn default_mode_and_utf8() {
        let pty = Pty::open(Winsize::default()).unwrap();
        let mode = pty.get_mode().unwrap();
        assert_eq!(mode, Mode::default());
        let attrs = termios::tcgetattr(&pty.master).unwrap();
        assert!(attrs.input_modes.contains(termios::InputModes::IUTF8));
    }

    #[test]
    fn master_is_cloexec_slave_is_not() {
        let pty = Pty::open(Winsize::default()).unwrap();
        let m = rustix::io::fcntl_getfd(&pty.master).unwrap();
        assert!(m.contains(rustix::io::FdFlags::CLOEXEC));
        let s = rustix::io::fcntl_getfd(pty.slave.as_ref().unwrap()).unwrap();
        assert!(!s.contains(rustix::io::FdFlags::CLOEXEC));
    }
}
