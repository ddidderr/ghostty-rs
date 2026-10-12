//! Main queue helpers: `DispatchQueue.main.async { ... }` and
//! `.asyncAfter(deadline:)` for closures that capture main-thread-only
//! (non-`Send`) state such as `Rc`/`Retained` AppKit objects.

use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime};

/// Wrapper to move a closure to the main queue. It is only ever run there,
/// i.e. on the main thread, where the main-thread-only captures are valid.
struct MainOnly<F>(F);

// SAFETY: the wrapped closure is only invoked on the main queue. Callers of
// `main_async*` are on the main thread (AppKit objects are created there) or
// pass only `Send` data; non-`Send` state is never touched off the main thread.
#[allow(unsafe_code)]
unsafe impl<F> Send for MainOnly<F> {}

/// `DispatchQueue.main.async { f() }`.
pub fn main_async(f: impl FnOnce() + 'static) {
    let wrapped = MainOnly(f);
    DispatchQueue::main().exec_async(move || {
        let wrapped = wrapped;
        (wrapped.0)();
    });
}

/// `DispatchQueue.main.asyncAfter(deadline: .now() + delay) { f() }`.
pub fn main_async_after(delay: Duration, f: impl FnOnce() + 'static) {
    let wrapped = MainOnly(f);
    let when = DispatchTime::try_from(delay).unwrap_or(DispatchTime::FOREVER);
    let _ = DispatchQueue::main().after(when, move || {
        let wrapped = wrapped;
        (wrapped.0)();
    });
}
