//! Failure injection for testing error handling paths (port of
//! `tripwire.zig`).
//!
//! Improper cleanup on error paths is one of the highest sources of bugs.
//! Many of those points are hard to exercise in unit tests and rare to
//! encounter in production. This module injects errors at named points
//! during unit tests so that every error path can be exercised.
//!
//! # Usage
//!
//! Declare a module per fallible function with [`tripwire_module!`]. The
//! point enum is hand-curated; the error type is the function's own error.
//! Sprinkle `tw::check(Point::X)?` before every operation that you want to be
//! able to fail, then in a unit test configure expected failures with
//! [`error_always`](TripwireState::error_always) /
//! [`error_after`](TripwireState::error_after) and finish with
//! `tw::end(ResetMode::Reset)` to verify the expectations were met.
//!
//! ```ignore
//! tripwire_module!(resize_tw, ResizePoint, TryReserveError);
//!
//! fn resize() -> Result<(), E> {
//!     resize_tw::check(ResizePoint::DynamicAlloc)?;
//!     // ...
//! }
//!
//! #[test]
//! fn resize_fails() {
//!     resize_tw::error_always(ResizePoint::DynamicAlloc, e);
//!     assert!(resize().is_err());
//!     resize_tw::end(ResetMode::Reset).unwrap();
//! }
//! ```
//!
//! Outside of unit tests (`cfg(test)` of the crate that expands the macro)
//! `check` is a no-op that the optimizer removes.
//!
//! State is per module *and per thread*. Rust runs tests on separate threads
//! in parallel, so this gives each test the same isolation Zig gets from
//! running tests sequentially.

use std::fmt::Debug;

/// Whether to reset expectations in [`TripwireState::end`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Reset,
    Retain,
}

/// Returned from `end` if a configured point was never tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("untripped tripwire")]
pub struct UntrippedError;

#[derive(Debug, Clone)]
struct Tripwire<E> {
    /// Error to return when tripped.
    err: E,
    /// The number of times this tripwire has been reached. This is NOT the
    /// number of times it has tripped, since we may have `min`.
    reached: usize,
    /// The minimum number of times this must be reached before tripping.
    /// After this point it trips every time. This is a "before" check, so if
    /// this is 1 it trips the second time it is reached.
    min: usize,
    /// True if this has been tripped at least once.
    tripped: bool,
}

/// The configured tripwires of one module.
#[derive(Debug, Clone)]
pub struct TripwireState<P, E> {
    wires: Vec<(P, Tripwire<E>)>,
}

impl<P, E> Default for TripwireState<P, E> {
    fn default() -> Self {
        Self { wires: Vec::new() }
    }
}

impl<P: Copy + PartialEq + Debug, E: Clone> TripwireState<P, E> {
    /// Check for a failure at the given failure point. Place directly before
    /// the operation that may fail.
    pub fn check(&mut self, point: P) -> Result<(), E> {
        let Some((_, w)) = self.wires.iter_mut().find(|(p, _)| *p == point) else {
            return Ok(());
        };
        w.reached += 1;
        if w.reached <= w.min {
            return Ok(());
        }
        w.tripped = true;
        Err(w.err.clone())
    }

    /// Mark a failure point to always trip with the given error.
    pub fn error_always(&mut self, point: P, err: E) {
        self.error_after(point, err, 0);
    }

    /// Mark a failure point to trip with the given error after it has been
    /// reached at least `min` times. Zero is equivalent to `error_always`.
    pub fn error_after(&mut self, point: P, err: E, min: usize) {
        let wire = Tripwire {
            err,
            reached: 0,
            min,
            tripped: false,
        };
        if let Some(slot) = self.wires.iter_mut().find(|(p, _)| *p == point) {
            slot.1 = wire;
        } else {
            self.wires.push((point, wire));
        }
    }

    /// Ends the tripwire session. Errors if there were untripped
    /// expectations. Expectations are reset (per `mode`) even on error.
    pub fn end(&mut self, mode: ResetMode) -> Result<(), UntrippedError> {
        let mut untripped = false;
        for (p, w) in &self.wires {
            if !w.tripped {
                log::warn!("untripped point={p:?}");
                untripped = true;
            }
        }
        if mode == ResetMode::Reset {
            self.reset();
        }
        if untripped {
            Err(UntrippedError)
        } else {
            Ok(())
        }
    }

    /// Unset all the tripwires. You should usually call `end` instead.
    pub fn reset(&mut self) {
        self.wires.clear();
    }
}

/// Declare a tripwire module `$name` with failure points `$point` (a `Copy +
/// PartialEq + Debug` enum) and error type `$err` (`Clone`). Generates
/// `$name::{check, error_always, error_after, end, reset}`.
#[macro_export]
macro_rules! tripwire_module {
    ($vis:vis $name:ident, $point:ty, $err:ty) => {
        #[allow(dead_code)]
        $vis mod $name {
            use super::*;
            use $crate::tripwire::{ResetMode, TripwireState, UntrippedError};

            ::std::thread_local! {
                static STATE: ::std::cell::RefCell<TripwireState<$point, $err>> =
                    ::std::cell::RefCell::new(TripwireState::default());
            }

            /// Check for a failure at `point`. A no-op outside of tests.
            #[inline(always)]
            pub fn check(point: $point) -> ::std::result::Result<(), $err> {
                #[cfg(test)]
                {
                    STATE.with(|s| s.borrow_mut().check(point))
                }
                #[cfg(not(test))]
                {
                    let _ = point;
                    Ok(())
                }
            }

            pub fn error_always(point: $point, err: $err) {
                STATE.with(|s| s.borrow_mut().error_always(point, err));
            }

            pub fn error_after(point: $point, err: $err, min: usize) {
                STATE.with(|s| s.borrow_mut().error_after(point, err, min));
            }

            pub fn end(mode: ResetMode) -> ::std::result::Result<(), UntrippedError> {
                STATE.with(|s| s.borrow_mut().end(mode))
            }

            pub fn reset() {
                STATE.with(|s| s.borrow_mut().reset());
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Io {
        Read,
        Write,
    }
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum E {
        Oom,
    }
    crate::tripwire_module!(io, Io, E);

    #[test]
    fn tripwire_basic() {
        // Reset should work
        io::end(ResetMode::Reset).unwrap();
        // By default, it's pass-through
        io::check(Io::Read).unwrap();
        // Always trip
        io::error_always(Io::Read, E::Oom);
        assert_eq!(Err(E::Oom), io::check(Io::Read));
        // Happens again
        assert_eq!(Err(E::Oom), io::check(Io::Read));
        // Other points are unaffected
        io::check(Io::Write).unwrap();
        io::end(ResetMode::Reset).unwrap();
    }

    #[test]
    fn tripwire_error_after() {
        // Trip after 2 calls (on the 3rd call)
        io::error_after(Io::Read, E::Oom, 2);
        io::check(Io::Read).unwrap();
        io::check(Io::Read).unwrap();
        assert_eq!(Err(E::Oom), io::check(Io::Read));
        assert_eq!(Err(E::Oom), io::check(Io::Read));
        io::end(ResetMode::Reset).unwrap();
    }

    #[test]
    fn tripwire_error_after_untripped_if_min_not_reached() {
        io::error_after(Io::Read, E::Oom, 2);
        io::check(Io::Read).unwrap();
        assert_eq!(Err(UntrippedError), io::end(ResetMode::Reset));
        // Expectations are reset even when end() fails.
        io::check(Io::Read).unwrap();
        io::end(ResetMode::Reset).unwrap();
    }

    #[test]
    fn tripwire_untripped_never_reached_and_retain() {
        io::error_always(Io::Write, E::Oom);
        assert_eq!(Err(UntrippedError), io::end(ResetMode::Retain));
        // Retained: still armed.
        assert_eq!(Err(E::Oom), io::check(Io::Write));
        io::end(ResetMode::Reset).unwrap();
    }

    #[test]
    fn tripwire_state_is_per_thread() {
        io::error_always(Io::Read, E::Oom);
        std::thread::spawn(|| io::check(Io::Read).unwrap())
            .join()
            .unwrap();
        assert_eq!(Err(E::Oom), io::check(Io::Read));
        io::end(ResetMode::Reset).unwrap();
    }
}
