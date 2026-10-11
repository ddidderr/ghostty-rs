//! A tiny cross-thread wakeup primitive (replaces `xev.Async`).
//!
//! Backed by a non-blocking `eventfd`, so it can be included in a `poll(2)`
//! set next to a pty fd or a pidfd. [`Wakeup::notify`] may be called from any
//! thread, any number of times; the waiter calls [`Wakeup::drain`] after
//! `poll` reports the fd readable.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

use rustix::event::{EventfdFlags, eventfd};

/// See the module docs. Cloning shares the same underlying eventfd.
#[derive(Clone, Debug)]
pub struct Wakeup(Arc<OwnedFd>);

impl Wakeup {
    pub fn new() -> io::Result<Wakeup> {
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        Ok(Wakeup(Arc::new(fd)))
    }

    /// Wake up the waiter. Multiple notifications coalesce.
    pub fn notify(&self) -> io::Result<()> {
        loop {
            match rustix::io::write(&*self.0, &1u64.to_ne_bytes()) {
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
        let mut buf = [0u8; 8];
        loop {
            match rustix::io::read(&*self.0, &mut buf) {
                Ok(_) => return true,
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return false,
            }
        }
    }

    /// The fd to poll for `POLLIN`.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::event::{PollFd, PollFlags, poll};

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
