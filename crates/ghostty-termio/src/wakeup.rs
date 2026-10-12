//! A tiny cross-thread wakeup primitive (replaces `xev.Async`).
//!
//! Backed by a non-blocking `eventfd` (a non-blocking pipe on Darwin, which
//! has no eventfd; libxev uses kqueue there), so it can be included in a
//! `poll(2)` set next to a pty fd or a pidfd. [`Wakeup::notify`] may be called from any
//! thread, any number of times; the waiter calls [`Wakeup::drain`] after
//! `poll` reports the fd readable.

use std::{
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::Arc,
};

/// The fds backing a wakeup: a single eventfd (read == write) on Linux, the
/// two ends of a pipe on Darwin.
struct Fds {
    read: OwnedFd,
    write: Option<OwnedFd>,
}

impl Fds {
    #[cfg(not(target_vendor = "apple"))]
    fn new() -> io::Result<Fds> {
        use rustix::event::{EventfdFlags, eventfd};
        Ok(Fds {
            read: eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?,
            write: None,
        })
    }

    #[cfg(target_vendor = "apple")]
    fn new() -> io::Result<Fds> {
        use rustix::{
            fs::{OFlags, fcntl_getfl, fcntl_setfl},
            io::{FdFlags, fcntl_setfd},
        };
        let (read, write) = rustix::pipe::pipe()?;
        for fd in [&read, &write] {
            fcntl_setfd(fd, FdFlags::CLOEXEC)?;
            fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?;
        }
        Ok(Fds {
            read,
            write: Some(write),
        })
    }

    fn write_fd(&self) -> &OwnedFd {
        self.write.as_ref().unwrap_or(&self.read)
    }
}

/// See the module docs. Cloning shares the same underlying fds.
#[derive(Clone)]
pub struct Wakeup(Arc<Fds>, Option<Arc<dyn Fn() + Send + Sync>>);

impl std::fmt::Debug for Wakeup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Wakeup").field(&self.0.read).finish()
    }
}

impl Wakeup {
    pub fn new() -> io::Result<Wakeup> {
        Ok(Wakeup(Arc::new(Fds::new()?), None))
    }

    /// A wakeup that calls `hook` instead of signalling an eventfd. Used to
    /// bridge into waiters that are not `poll(2)`-based (the renderer
    /// thread's channel). `as_fd`/`drain` are meaningless for these.
    pub fn with_hook(hook: Arc<dyn Fn() + Send + Sync>) -> io::Result<Wakeup> {
        Ok(Wakeup(Arc::new(Fds::new()?), Some(hook)))
    }

    /// Wake up the waiter. Multiple notifications coalesce.
    pub fn notify(&self) -> io::Result<()> {
        if let Some(hook) = &self.1 {
            hook();
            return Ok(());
        }
        loop {
            match rustix::io::write(self.0.write_fd(), &1u64.to_ne_bytes()) {
                Ok(_) => return Ok(()),
                Err(rustix::io::Errno::INTR) => {}
                // Counter would overflow: the waiter is already guaranteed
                // to wake up.
                Err(rustix::io::Errno::AGAIN) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Reset the notification state. Returns true if a notification was
    /// pending.
    pub fn drain(&self) -> bool {
        let mut buf = [0u8; 64];
        // An eventfd resets with one read; a pipe is read until empty.
        let mut any = false;
        loop {
            match rustix::io::read(&self.0.read, &mut buf) {
                Ok(_) if self.0.write.is_none() => return true,
                Ok(0) => return any,
                Ok(_) => any = true,
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return any,
            }
        }
    }

    /// The fd to poll for `POLLIN`.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.read.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use rustix::event::{PollFd, PollFlags, poll};

    use super::*;

    fn readable(w: &Wakeup) -> bool {
        let mut fds = [PollFd::from_borrowed_fd(w.as_fd(), PollFlags::IN)];
        poll(
            &mut fds,
            Some(&rustix::time::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .unwrap()
            > 0
    }

    #[test]
    fn notify_coalesces_and_drains() {
        let w = Wakeup::new().unwrap();
        assert!(!readable(&w));
        w.notify().unwrap();
        w.notify().unwrap();
        assert!(readable(&w));
        assert!(w.drain());
        assert!(!readable(&w));
        assert!(!w.drain());
    }

    #[test]
    fn notify_from_other_thread() {
        let w = Wakeup::new().unwrap();
        let w2 = w.clone();
        std::thread::spawn(move || w2.notify().unwrap())
            .join()
            .unwrap();
        assert!(readable(&w));
    }
}
