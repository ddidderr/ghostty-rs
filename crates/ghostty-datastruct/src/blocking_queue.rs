//! Blocking queue implementation aimed primarily for message passing
//! between threads (`blocking_queue.zig`).

use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

/// The type we use for queue size types.
pub type Size = u32;

/// Specifies the timeout for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    /// Fail instantly (non-blocking).
    Instant,

    /// Run forever or until interrupted
    Forever,

    /// Nanoseconds
    Ns(u64),
}

#[derive(Debug)]
struct Inner<T> {
    data: VecDeque<T>,
    not_full_waiters: usize,
}

/// A blocking queue for type T with a fixed `CAPACITY`.
///
/// This is tailor made for ghostty usage so it isn't meant to be maximally
/// generic. Traits of this queue that are specific to our usage:
///
///   - Fixed size. We expect our queue to quickly drain and also not be
///     too large so we prefer a fixed size queue for now.
///   - No blocking pop. We use an external event loop mechanism such as
///     eventfd to notify our waiter that there is no data available so
///     we don't need to implement a blocking pop.
///   - Drain function. Most queues usually pop one at a time. We have
///     a mechanism for draining since on every IO loop our TTY drains
///     the full queue so we can get rid of the overhead of a ton of
///     locks and bounds checking and do a one-time drain.
///
/// One key usage pattern is that our blocking queues are single producer
/// single consumer (SPSC). This should let us do some interesting optimizations
/// in the future. At the time of writing this, the blocking queue implementation
/// is purposely naive to build something quickly, but we should benchmark
/// and make this more optimized as necessary.
#[derive(Debug)]
pub struct BlockingQueue<T, const CAPACITY: usize> {
    /// The big mutex that must be held to read/write.
    inner: Mutex<Inner<T>>,

    /// A CV for being notified when the queue is no longer full. This is
    /// used for writing. Note we DON'T have a CV for waiting on the
    /// queue not being EMPTY because we use external notifiers for that.
    cond_not_full: Condvar,
}

impl<T, const CAPACITY: usize> Default for BlockingQueue<T, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const CAPACITY: usize> BlockingQueue<T, CAPACITY> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                data: VecDeque::with_capacity(CAPACITY),
                not_full_waiters: 0,
            }),
            cond_not_full: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Push a value to the queue. This returns the total size of the
    /// queue (unread items) after the push. A return value of zero
    /// means that the push failed.
    pub fn push(&self, value: T, timeout: Timeout) -> Size {
        let mut g = self.lock();

        if g.data.len() >= CAPACITY {
            match timeout {
                // If we're not waiting, then we failed to write.
                Timeout::Instant => return 0,

                Timeout::Forever => {
                    g.not_full_waiters += 1;
                    g = self
                        .cond_not_full
                        .wait_while(g, |i| i.data.len() >= CAPACITY)
                        .unwrap_or_else(PoisonError::into_inner);
                    g.not_full_waiters -= 1;
                }

                Timeout::Ns(ns) => {
                    g.not_full_waiters += 1;
                    let (guard, _timed_out) = self
                        .cond_not_full
                        .wait_timeout_while(g, Duration::from_nanos(ns), |i| {
                            i.data.len() >= CAPACITY
                        })
                        .unwrap_or_else(PoisonError::into_inner);
                    g = guard;
                    g.not_full_waiters -= 1;
                }
            }

            // If we're still full, then we failed to write. This can
            // happen in situations where we are interrupted.
            if g.data.len() >= CAPACITY {
                return 0;
            }
        }

        // Add our data and update our accounting
        g.data.push_back(value);
        g.data.len() as Size
    }

    /// Pop a value from the queue without blocking.
    pub fn pop(&self) -> Option<T> {
        let mut g = self.lock();

        // If we're empty we have nothing
        let v = g.data.pop_front()?;

        // If we have producers waiting on a full queue, notify.
        if g.not_full_waiters > 0 {
            self.cond_not_full.notify_one();
        }

        Some(v)
    }

    /// Pop all values from the queue. This will hold the big mutex
    /// until the returned iterator is dropped. This is used if
    /// you know you're going to "pop" and utilize all the values
    /// quickly to avoid many locks, bounds checks, and cv signals.
    pub fn drain(&self) -> DrainIterator<'_, T, CAPACITY> {
        DrainIterator {
            guard: self.lock(),
            queue: self,
        }
    }
}

/// Iterator returned by [`BlockingQueue::drain`]. Holds the queue lock until
/// dropped (the Zig `deinit`).
pub struct DrainIterator<'a, T, const CAPACITY: usize> {
    guard: MutexGuard<'a, Inner<T>>,
    queue: &'a BlockingQueue<T, CAPACITY>,
}

impl<T, const CAPACITY: usize> Iterator for DrainIterator<'_, T, CAPACITY> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.guard.data.pop_front()
    }
}

impl<T, const CAPACITY: usize> Drop for DrainIterator<'_, T, CAPACITY> {
    fn drop(&mut self) {
        // If we have producers waiting on a full queue, notify. The lock is
        // released after this when the guard is dropped.
        if self.guard.not_full_waiters > 0 {
            self.queue.cond_not_full.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    #[test]
    fn basic_push_and_pop() {
        let q = BlockingQueue::<u64, 4>::new();

        // Should have no values
        assert!(q.pop().is_none());

        // Push until we're full
        assert_eq!(q.push(1, Timeout::Instant), 1);
        assert_eq!(q.push(2, Timeout::Instant), 2);
        assert_eq!(q.push(3, Timeout::Instant), 3);
        assert_eq!(q.push(4, Timeout::Instant), 4);
        assert_eq!(q.push(5, Timeout::Instant), 0);

        // Pop!
        assert_eq!(q.pop().unwrap(), 1);
        assert_eq!(q.pop().unwrap(), 2);
        assert_eq!(q.pop().unwrap(), 3);
        assert_eq!(q.pop().unwrap(), 4);
        assert!(q.pop().is_none());

        // Drain does nothing
        let mut it = q.drain();
        assert!(it.next().is_none());
        drop(it);

        // Verify we can still push
        assert_eq!(q.push(1, Timeout::Instant), 1);
    }

    #[test]
    fn timed_push() {
        let q = BlockingQueue::<u64, 1>::new();

        // Push
        assert_eq!(q.push(1, Timeout::Instant), 1);
        assert_eq!(q.push(2, Timeout::Instant), 0);

        // Timed push should fail
        assert_eq!(q.push(2, Timeout::Ns(1000)), 0);
    }

    #[test]
    fn drain_returns_all_in_order() {
        let q = BlockingQueue::<u8, 8>::new();
        for i in 0..5 {
            q.push(i, Timeout::Instant);
        }
        let v: Vec<u8> = q.drain().collect();
        assert_eq!(v, [0, 1, 2, 3, 4]);
        assert!(q.pop().is_none());
    }

    #[test]
    fn blocking_push_wakes_on_pop() {
        let q = Arc::new(BlockingQueue::<u32, 1>::new());
        assert_eq!(q.push(1, Timeout::Instant), 1);

        let q2 = Arc::clone(&q);
        let t = thread::spawn(move || q2.push(2, Timeout::Forever));

        // Wait until the producer is blocked, then pop.
        loop {
            if q.lock().not_full_waiters > 0 {
                break;
            }
            thread::yield_now();
        }
        assert_eq!(q.pop(), Some(1));
        assert_eq!(t.join().unwrap(), 1);
        assert_eq!(q.pop(), Some(2));
    }

    #[test]
    fn blocking_push_wakes_on_drain() {
        let q = Arc::new(BlockingQueue::<u32, 1>::new());
        assert_eq!(q.push(1, Timeout::Instant), 1);

        let q2 = Arc::clone(&q);
        let t = thread::spawn(move || q2.push(2, Timeout::Ns(5_000_000_000)));
        loop {
            if q.lock().not_full_waiters > 0 {
                break;
            }
            thread::yield_now();
        }
        drop(q.drain().collect::<Vec<_>>());
        assert_eq!(t.join().unwrap(), 1);
    }
}
