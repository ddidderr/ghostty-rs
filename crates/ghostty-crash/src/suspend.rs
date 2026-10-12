//! Suspending every other thread of the process while a crash report is
//! written, and *always* resuming them afterwards.
//!
//! The macOS exception handler thread suspends all other threads so the dump
//! is consistent. Breakpad resumes them before handing the exception back to
//! the kernel; without that, the faulting thread (which the kernel parked
//! until the handler replies) stays suspended forever and the process hangs
//! instead of dying through BSD signal delivery. The platform specific
//! thread operations are behind [`Threads`] so the bookkeeping is testable
//! everywhere.

// Only the macOS handler uses this outside of tests.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// The thread operations needed by [`SuspendGuard`].
pub(crate) trait Threads {
    type Id: Copy + PartialEq;

    /// All threads of the process.
    fn list(&self) -> Vec<Self::Id>;
    fn suspend(&self, id: Self::Id);
    fn resume(&self, id: Self::Id);
}

/// Resumes the threads it suspended when dropped (also on panic).
pub(crate) struct SuspendGuard<'a, T: Threads> {
    ops: &'a T,
    suspended: Vec<T::Id>,
}

impl<'a, T: Threads> SuspendGuard<'a, T> {
    /// Suspend every thread except `keep` (the calling handler thread).
    pub(crate) fn suspend_all_except(ops: &'a T, keep: T::Id) -> Self {
        let suspended: Vec<T::Id> = ops.list().into_iter().filter(|t| *t != keep).collect();
        for t in &suspended {
            ops.suspend(*t);
        }
        SuspendGuard { ops, suspended }
    }

    /// The number of threads this guard suspended.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.suspended.len()
    }
}

impl<T: Threads> Drop for SuspendGuard<'_, T> {
    fn drop(&mut self) {
        for t in self.suspended.drain(..) {
            self.ops.resume(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct Mock {
        // (id, suspend count)
        threads: RefCell<Vec<(u32, i32)>>,
    }

    impl Threads for Mock {
        type Id = u32;
        fn list(&self) -> Vec<u32> {
            self.threads.borrow().iter().map(|t| t.0).collect()
        }
        fn suspend(&self, id: u32) {
            for t in self.threads.borrow_mut().iter_mut().filter(|t| t.0 == id) {
                t.1 += 1;
            }
        }
        fn resume(&self, id: u32) {
            for t in self.threads.borrow_mut().iter_mut().filter(|t| t.0 == id) {
                t.1 -= 1;
            }
        }
    }

    #[test]
    fn suspends_all_but_handler_and_resumes_all_on_drop() {
        // Thread 1 is the faulting thread, 9 the handler.
        let m = Mock {
            threads: RefCell::new(vec![(1, 0), (2, 0), (9, 0)]),
        };
        {
            let g = SuspendGuard::suspend_all_except(&m, 9);
            assert_eq!(g.len(), 2);
            assert_eq!(*m.threads.borrow(), [(1, 1), (2, 1), (9, 0)]);
        }
        // Every thread, including the faulting one, runs again.
        assert_eq!(*m.threads.borrow(), [(1, 0), (2, 0), (9, 0)]);
    }

    #[test]
    fn resumes_even_if_the_dump_panics() {
        let m = Mock {
            threads: RefCell::new(vec![(1, 0), (9, 0)]),
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = SuspendGuard::suspend_all_except(&m, 9);
            panic!("dump failed");
        }));
        assert!(r.is_err());
        assert_eq!(*m.threads.borrow(), [(1, 0), (9, 0)]);
    }
}
