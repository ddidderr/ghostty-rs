//! The location to where write-related messages are sent
//! (`termio/mailbox.zig`).
//!
//! Zig has a single `spsc` variant: a fixed-capacity blocking queue plus an
//! `xev.Async` wakeup. Here the queue is a bounded `crossbeam-channel` (so a
//! full queue can hand the rejected message back to the sender, which the
//! `BlockingQueue` cannot) and the wakeup is an eventfd ([`Wakeup`]).

use std::sync::{Mutex, MutexGuard, PoisonError};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::message::Message;
use crate::wakeup::Wakeup;

/// Capacity of the queue. Hardcoded to a value that empirically has made
/// sense for Ghostty usage (`BlockingQueue(termio.Message, 64)`).
pub const CAPACITY: usize = 64;

/// The termio mailbox. Cloning is cheap and shares the same queue; all
/// clones are producers, but only the IO thread should call [`Mailbox::pop`]
/// / [`Mailbox::drain`] (single consumer by convention, like Zig).
#[derive(Clone, Debug)]
pub struct Mailbox {
    tx: Sender<Message>,
    rx: Receiver<Message>,
    wakeup: Wakeup,
}

impl Mailbox {
    /// Init the SPSC mailbox (`initSPSC`).
    pub fn new() -> std::io::Result<Mailbox> {
        let (tx, rx) = bounded(CAPACITY);
        Ok(Mailbox {
            tx,
            rx,
            wakeup: Wakeup::new()?,
        })
    }

    /// The wakeup the IO thread waits on.
    pub fn wakeup(&self) -> &Wakeup {
        &self.wakeup
    }

    /// Sends the given message without notifying there are messages.
    ///
    /// If the queue is full this wakes up the IO thread so it can make room
    /// and then blocks until the message is queued. Do NOT call this while
    /// holding the terminal lock if the queue may be full (the IO thread may
    /// need the lock to make progress); use [`Mailbox::send_locked`] or
    /// [`Mailbox::try_send`] in that case.
    pub fn send(&self, msg: Message) {
        match self.tx.try_send(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(msg)) => {
                if let Err(err) = self.wakeup.notify() {
                    log::warn!("failed to wake up writer, data will be dropped err={err}");
                    return;
                }
                // Disconnected can't happen: we own a receiver too.
                let _ = self.tx.send(msg);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Like [`Mailbox::send`] for callers that hold a lock (the Zig
    /// `mutex` argument): if the queue is full the guard is released while
    /// blocking and re-acquired afterwards.
    pub fn send_locked<'a, T>(
        &self,
        msg: Message,
        mutex: &'a Mutex<T>,
        guard: MutexGuard<'a, T>,
    ) -> MutexGuard<'a, T> {
        match self.tx.try_send(msg) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => guard,
            Err(TrySendError::Full(msg)) => {
                if let Err(err) = self.wakeup.notify() {
                    log::warn!("failed to wake up writer, data will be dropped err={err}");
                    return guard;
                }
                // Unlock the renderer state so the writer thread can acquire
                // it, then queue our message before continuing.
                drop(guard);
                let _ = self.tx.send(msg);
                mutex.lock().unwrap_or_else(PoisonError::into_inner)
            }
        }
    }

    /// Queue without ever blocking. Returns the message back if the queue is
    /// full.
    pub fn try_send(&self, msg: Message) -> Result<(), Message> {
        match self.tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(m) | TrySendError::Disconnected(m)) => Err(m),
        }
    }

    /// Notify that there are new messages.
    pub fn notify(&self) {
        if let Err(err) = self.wakeup.notify() {
            log::warn!("failed to notify writer, data will be dropped err={err}");
        }
    }

    /// Pop one message without blocking (IO thread).
    pub fn pop(&self) -> Option<Message> {
        self.rx.try_recv().ok()
    }

    /// Pop everything currently queued (IO thread).
    pub fn drain(&self) -> impl Iterator<Item = Message> + '_ {
        std::iter::from_fn(|| self.pop())
    }

    /// Number of queued messages.
    pub fn len(&self) -> usize {
        self.rx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn send_pop_order() {
        let mb = Mailbox::new().unwrap();
        mb.send(Message::Focused(true));
        mb.send(Message::LinefeedMode(false));
        assert_eq!(mb.len(), 2);
        assert!(matches!(mb.pop(), Some(Message::Focused(true))));
        assert!(matches!(mb.pop(), Some(Message::LinefeedMode(false))));
        assert!(mb.pop().is_none());
    }

    #[test]
    fn try_send_returns_message_when_full() {
        let mb = Mailbox::new().unwrap();
        for _ in 0..CAPACITY {
            mb.try_send(Message::Crash).unwrap();
        }
        let back = mb.try_send(Message::Focused(true)).unwrap_err();
        assert!(matches!(back, Message::Focused(true)));
    }

    #[test]
    fn blocking_send_wakes_consumer_and_completes() {
        let mb = Mailbox::new().unwrap();
        for _ in 0..CAPACITY {
            mb.send(Message::Crash);
        }
        let consumer = mb.clone();
        let t = std::thread::spawn(move || {
            // Wait for the wakeup the full-queue path posts, then drain.
            let mut fds = [rustix::event::PollFd::from_borrowed_fd(
                consumer.wakeup().as_fd(),
                rustix::event::PollFlags::IN,
            )];
            rustix::event::poll(&mut fds, None).unwrap();
            consumer.wakeup().drain();
            consumer.drain().count()
        });
        mb.send(Message::Focused(false)); // blocks until the consumer drains
        let n = t.join().unwrap();
        assert!(n >= CAPACITY);
    }

    #[test]
    fn send_locked_releases_lock_when_full() {
        let mb = Mailbox::new().unwrap();
        for _ in 0..CAPACITY {
            mb.send(Message::Crash);
        }
        let m = Arc::new(Mutex::new(0u32));
        let consumer = mb.clone();
        let m2 = Arc::clone(&m);
        // The consumer needs the lock to make progress (like the IO thread).
        let t = std::thread::spawn(move || {
            let mut fds = [rustix::event::PollFd::from_borrowed_fd(
                consumer.wakeup().as_fd(),
                rustix::event::PollFlags::IN,
            )];
            rustix::event::poll(&mut fds, None).unwrap();
            let mut g = m2.lock().unwrap();
            *g += 1;
            consumer.drain().count()
        });
        let g = m.lock().unwrap();
        let g = mb.send_locked(Message::Focused(true), &m, g);
        assert!(*g <= 1);
        drop(g);
        t.join().unwrap();
    }
}
