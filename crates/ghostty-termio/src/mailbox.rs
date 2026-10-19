//! The location to where write-related messages are sent
//! (`termio/mailbox.zig`).
//!
//! Zig has a single `spsc` variant: a fixed-capacity blocking queue plus an
//! `xev.Async` wakeup. Here the queue is a bounded `crossbeam-channel` (so a
//! full queue can hand the rejected message back to the sender, which the
//! `BlockingQueue` cannot) and the wakeup is an eventfd ([`Wakeup`]).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::{message::Message, wakeup::Wakeup};

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
    cancel: Arc<Mutex<Option<Sender<()>>>>,
    cancelled: Receiver<()>,
}

impl Mailbox {
    /// Init the SPSC mailbox (`initSPSC`).
    pub fn new() -> std::io::Result<Mailbox> {
        let (tx, rx) = bounded(CAPACITY);
        let (cancel, cancelled) = bounded(0);
        Ok(Mailbox {
            tx,
            rx,
            wakeup: Wakeup::new()?,
            cancel: Arc::new(Mutex::new(Some(cancel))),
            cancelled,
        })
    }

    /// Permanently reject sends and release every blocked producer, even though
    /// mailbox clones still own receivers. Queued messages remain drainable.
    /// A send selected concurrently with close may finish enqueueing; calls
    /// after close prefer cancellation even when the queue has room.
    pub fn close(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    fn try_send_inner(&self, msg: Message) -> Result<(), TrySendError<Message>> {
        crossbeam_channel::select_biased! {
            recv(self.cancelled) -> _ => Err(TrySendError::Disconnected(msg)),
            send(self.tx, msg) -> result => result.map_err(|e| TrySendError::Disconnected(e.0)),
            default => Err(TrySendError::Full(msg)),
        }
    }

    fn send_or_cancel(&self, msg: Message) {
        crossbeam_channel::select_biased! {
            recv(self.cancelled) -> _ => {},
            send(self.tx, msg) -> _ => {},
        }
    }

    /// The wakeup the IO thread waits on.
    pub fn wakeup(&self) -> &Wakeup {
        &self.wakeup
    }

    /// Sends the given message without notifying there are messages.
    ///
    /// If the queue is full this wakes up the IO thread so it can make room
    /// and then blocks until queued or closed. Do NOT call this while holding
    /// the terminal lock if the queue may be full (the IO thread may
    /// need the lock to make progress); use [`Mailbox::send_locked`] or
    /// [`Mailbox::try_send`] in that case.
    pub fn send(&self, msg: Message) {
        match self.try_send_inner(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(msg)) => {
                if let Err(err) = self.wakeup.notify() {
                    log::warn!("failed to wake up writer, data will be dropped err={err}");
                    return;
                }
                self.send_or_cancel(msg);
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
        match self.try_send_inner(msg) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => guard,
            Err(TrySendError::Full(msg)) => {
                if let Err(err) = self.wakeup.notify() {
                    log::warn!("failed to wake up writer, data will be dropped err={err}");
                    return guard;
                }
                // Unlock the renderer state so the writer thread can acquire
                // it, then queue our message before continuing.
                drop(guard);
                self.send_or_cancel(msg);
                mutex.lock().unwrap_or_else(PoisonError::into_inner)
            }
        }
    }

    /// Queue without ever blocking. Returns the message back if the queue is
    /// full or closed.
    pub fn try_send(&self, msg: Message) -> Result<(), Message> {
        match self.try_send_inner(msg) {
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
    use std::sync::Arc;

    use super::*;

    #[test]
    fn close_releases_blocked_sends_including_locked_send() {
        use std::time::Duration;
        let mb = Mailbox::new().unwrap();
        for _ in 0..CAPACITY {
            mb.send(Message::Crash);
        }
        let lock = Arc::new(Mutex::new(0));
        let (done_tx, done_rx) = crossbeam_channel::unbounded();
        let workers: Vec<_> = [false, true]
            .into_iter()
            .map(|locked| {
                let mb = mb.clone();
                let lock = Arc::clone(&lock);
                let done = done_tx.clone();
                std::thread::spawn(move || {
                    if locked {
                        let guard = lock.lock().unwrap();
                        let mut guard = mb.send_locked(Message::Focused(true), &lock, guard);
                        *guard += 1;
                    } else {
                        mb.send(Message::Focused(false));
                    }
                    done.send(()).unwrap();
                })
            })
            .collect();
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        mb.clone().close();
        mb.close();
        for _ in &workers {
            done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(*lock.lock().unwrap(), 1); // send_locked reacquired the lock
        assert_eq!(mb.len(), CAPACITY); // no consumer was needed
        assert_eq!(mb.drain().count(), CAPACITY);
        assert!(matches!(
            mb.try_send(Message::Focused(true)),
            Err(Message::Focused(true))
        ));
        mb.send(Message::Crash);
        let guard = lock.lock().unwrap();
        drop(mb.send_locked(Message::Crash, &lock, guard));
        assert!(mb.is_empty());
    }

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

    /// Block until the wakeup is signaled.
    #[cfg(unix)]
    fn wait_readable(w: &Wakeup) {
        let mut fds = [rustix::event::PollFd::from_borrowed_fd(
            w.as_fd(),
            rustix::event::PollFlags::IN,
        )];
        rustix::event::poll(&mut fds, None).unwrap();
    }

    /// Block until the wakeup is signaled (this consumes the auto-reset
    /// event; the callers' `drain` after it is then a harmless no-op).
    #[cfg(windows)]
    #[allow(unsafe_code)]
    fn wait_readable(w: &Wakeup) {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: valid event handle owned by the wakeup.
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(
                w.as_handle().as_raw_handle().cast(),
                windows_sys::Win32::System::Threading::INFINITE,
            )
        };
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
            wait_readable(consumer.wakeup());
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
            wait_readable(consumer.wakeup());
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
