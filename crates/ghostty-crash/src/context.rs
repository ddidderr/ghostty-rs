//! Thread-local crash context (port of `ThreadState` in `crash/sentry.zig`).
//!
//! The native Sentry SDK has one global scope and no thread-local scopes, so
//! Ghostty keeps thread-specific data in a thread-local that the crash
//! callback reads. We do the same: thread entry functions install a
//! [`ThreadGuard`], and the panic hook / signal handler read it.
//!
//! Zig stores a `*Surface` and reads its size when the crash happens. The
//! Rust surface is not shareable across threads, so it instead publishes its
//! dimensions into a shared [`SurfaceDims`] (plain atomics, readable from a
//! signal handler) that the threads hold an `Arc` of.
#![allow(unsafe_code)]

use std::{
    cell::Cell,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

/// The type of thread (Zig: `ThreadState.Type`). Used to tag the crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThreadType {
    Main = 1,
    Renderer = 2,
    Io = 3,
}

impl ThreadType {
    /// The Zig `@tagName`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadType::Main => "main",
            ThreadType::Renderer => "renderer",
            ThreadType::Io => "io",
        }
    }

    fn from_u8(v: u8) -> Option<ThreadType> {
        match v {
            1 => Some(ThreadType::Main),
            2 => Some(ThreadType::Renderer),
            3 => Some(ThreadType::Io),
            _ => None,
        }
    }
}

/// The surface dimensions recorded in a crash (`Dimensions` context).
/// Updated by the owner of the surface whenever the size changes.
#[derive(Debug, Default)]
pub struct SurfaceDims {
    screen_width: AtomicU32,
    screen_height: AtomicU32,
    grid_columns: AtomicU32,
    grid_rows: AtomicU32,
    cell_width: AtomicU32,
    cell_height: AtomicU32,
}

/// A plain snapshot of [`SurfaceDims`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DimsSnapshot {
    pub screen_width: u32,
    pub screen_height: u32,
    pub grid_columns: u32,
    pub grid_rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
}

impl SurfaceDims {
    #[must_use]
    pub fn new() -> Arc<SurfaceDims> {
        Arc::new(SurfaceDims::default())
    }

    pub fn set(&self, d: DimsSnapshot) {
        self.screen_width.store(d.screen_width, Ordering::Relaxed);
        self.screen_height.store(d.screen_height, Ordering::Relaxed);
        self.grid_columns.store(d.grid_columns, Ordering::Relaxed);
        self.grid_rows.store(d.grid_rows, Ordering::Relaxed);
        self.cell_width.store(d.cell_width, Ordering::Relaxed);
        self.cell_height.store(d.cell_height, Ordering::Relaxed);
    }

    #[must_use]
    pub fn get(&self) -> DimsSnapshot {
        DimsSnapshot {
            screen_width: self.screen_width.load(Ordering::Relaxed),
            screen_height: self.screen_height.load(Ordering::Relaxed),
            grid_columns: self.grid_columns.load(Ordering::Relaxed),
            grid_rows: self.grid_rows.load(Ordering::Relaxed),
            cell_width: self.cell_width.load(Ordering::Relaxed),
            cell_height: self.cell_height.load(Ordering::Relaxed),
        }
    }
}

thread_local! {
    // Const-initialized with a type that has no destructor so reading it
    // from a signal handler is just a TLS load.
    static STATE: Cell<(u8, *const SurfaceDims)> = const { Cell::new((0, ptr::null())) };
}

/// Sets the thread state for the current thread until dropped, then restores
/// whatever was set before (Zig: `thread_state = ...; defer thread_state = null`).
#[must_use = "the crash context is cleared when the guard is dropped"]
pub struct ThreadGuard {
    prev: (u8, *const SurfaceDims),
    // Keeps the pointer in `STATE` valid.
    _dims: Option<Arc<SurfaceDims>>,
    // Thread-local state must be restored on the same thread.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ThreadGuard {
    pub fn new(kind: ThreadType, dims: Option<Arc<SurfaceDims>>) -> ThreadGuard {
        let p = dims.as_ref().map_or(ptr::null(), Arc::as_ptr);
        let prev = STATE.with(|s| s.replace((kind as u8, p)));
        ThreadGuard {
            prev,
            _dims: dims,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for ThreadGuard {
    fn drop(&mut self) {
        STATE.with(|s| s.set(self.prev));
    }
}

/// The surface dimensions of the current thread's crash context, to hand to
/// threads it spawns (e.g. the termio read thread).
#[must_use]
pub fn current_dims() -> Option<Arc<SurfaceDims>> {
    let (_, p) = STATE.with(Cell::get);
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` came from `Arc::as_ptr` of an Arc owned by a live
    // `ThreadGuard` on this thread (the guard restores `STATE` before it
    // releases the Arc), so the strong count is >= 1 here.
    unsafe {
        Arc::increment_strong_count(p);
        Some(Arc::from_raw(p))
    }
}

/// Async-signal-safe read of the current thread state.
pub(crate) fn current() -> Option<(ThreadType, Option<DimsSnapshot>)> {
    let (k, p) = STATE.try_with(Cell::get).ok()?;
    let kind = ThreadType::from_u8(k)?;
    let dims = if p.is_null() {
        None
    } else {
        // SAFETY: see `current_dims`; the guard keeps the Arc alive while
        // `STATE` points at it. Fields are atomics so a racing writer is
        // fine (values may be stale, which upstream also tolerates).
        Some(unsafe { (*p).get() })
    };
    Some((kind, dims))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn guard_sets_and_restores() {
        assert!(current().is_none());
        let dims = SurfaceDims::new();
        dims.set(DimsSnapshot {
            grid_columns: 80,
            ..Default::default()
        });
        {
            let _g = ThreadGuard::new(ThreadType::Renderer, Some(dims));
            let (k, d) = current().unwrap();
            assert_eq!(ThreadType::Renderer, k);
            assert_eq!(80, d.unwrap().grid_columns);
            assert!(current_dims().is_some());
            {
                let _g2 = ThreadGuard::new(ThreadType::Io, None);
                assert_eq!(ThreadType::Io, current().unwrap().0);
            }
            assert_eq!(ThreadType::Renderer, current().unwrap().0);
        }
        assert!(current().is_none());
        assert!(current_dims().is_none());
    }
}
