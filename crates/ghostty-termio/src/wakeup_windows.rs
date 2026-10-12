//! A tiny cross-thread wakeup primitive (replaces `xev.Async`), Windows
//! variant of [`crate::wakeup`].
//!
//! Backed by an auto-reset event object, so it can be included in a
//! `WaitForMultipleObjects` set next to a process handle.
//! [`Wakeup::notify`] may be called from any thread, any number of times
//! (notifications coalesce); the waiter calls [`Wakeup::drain`] after the
//! wait reports the event signaled.

// Win32 FFI takes out-parameters as `*mut`; implicit `&mut` -> pointer coercions
// are the idiomatic spelling at these call sites.
#![allow(clippy::borrow_as_ptr)]

use std::{
    io,
    os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    ptr,
    sync::Arc,
};

use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, SetEvent, WaitForSingleObject},
};

/// See the module docs. Cloning shares the same underlying event.
#[derive(Clone)]
pub struct Wakeup(Arc<OwnedHandle>, Option<Arc<dyn Fn() + Send + Sync>>);

impl std::fmt::Debug for Wakeup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Wakeup").field(&self.0).finish()
    }
}

#[allow(unsafe_code)]
fn new_event() -> io::Result<OwnedHandle> {
    // SAFETY: default security, auto-reset, initially non-signaled, unnamed.
    let h = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
    if h.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateEventW succeeded; we own the handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(h.cast()) })
}

impl Wakeup {
    pub fn new() -> io::Result<Wakeup> {
        Ok(Wakeup(Arc::new(new_event()?), None))
    }

    /// A wakeup that calls `hook` instead of signalling the event. Used to
    /// bridge into waiters that are not handle-based (the renderer thread's
    /// channel). `as_handle`/`drain` are meaningless for these.
    pub fn with_hook(hook: Arc<dyn Fn() + Send + Sync>) -> io::Result<Wakeup> {
        Ok(Wakeup(Arc::new(new_event()?), Some(hook)))
    }

    /// Wake up the waiter. Multiple notifications coalesce.
    #[allow(unsafe_code)]
    pub fn notify(&self) -> io::Result<()> {
        if let Some(hook) = &self.1 {
            hook();
            return Ok(());
        }
        // SAFETY: valid event handle.
        if unsafe { SetEvent(self.0.as_raw_handle() as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Reset the notification state. Returns true if a notification was
    /// pending.
    #[allow(unsafe_code)]
    pub fn drain(&self) -> bool {
        // SAFETY: valid event handle. A zero timeout polls; success on an
        // auto-reset event also resets it.
        unsafe { WaitForSingleObject(self.0.as_raw_handle() as HANDLE, 0) == WAIT_OBJECT_0 }
    }

    /// The handle to wait on (signaled when notified).
    pub fn as_handle(&self) -> BorrowedHandle<'_> {
        self.0.as_handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signaled(w: &Wakeup) -> bool {
        // Polling consumes the (auto-reset) event, so re-arm if it fired.
        let fired = w.drain();
        if fired {
            w.notify().unwrap();
        }
        fired
    }

    #[test]
    fn notify_coalesces_and_drains() {
        let w = Wakeup::new().unwrap();
        assert!(!signaled(&w));
        w.notify().unwrap();
        w.notify().unwrap();
        assert!(signaled(&w));
        assert!(w.drain());
        assert!(!signaled(&w));
        assert!(!w.drain());
    }

    #[test]
    fn notify_from_other_thread() {
        let w = Wakeup::new().unwrap();
        let w2 = w.clone();
        std::thread::spawn(move || w2.notify().unwrap())
            .join()
            .unwrap();
        assert!(signaled(&w));
    }
}
