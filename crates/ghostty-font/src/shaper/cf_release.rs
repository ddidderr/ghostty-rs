//! Dedicated thread for releasing CoreFoundation objects.
//!
//! Pools of CF objects (strings, attributed strings, typesetters, lines,
//! fonts) are sent here so that their (excessively slow) release callback
//! logic doesn't block the shaping/renderer thread. Equivalent of
//! `os/cf_release_thread.zig` for the shaper's needs: a mailbox of owned
//! pools and a thread that drops them.
//!
//! TODO(integration): this is a self-contained version living in the font
//! crate; it can be replaced by `ghostty_os::cf_release_thread` once the os
//! crate's port lands and the font crate depends on it.
#![allow(unsafe_code)]

use std::{
    sync::mpsc::{Sender, channel},
    thread::JoinHandle,
};

use objc2_core_foundation::{CFRetained, CFType};

use crate::coretext_util::CfBox;

/// An owned CF object queued for release.
pub(crate) type Pooled = CfBox<CFType>;

/// Convert any retained CF object into a pool entry.
pub(crate) fn pooled<T: objc2_core_foundation::Type>(v: CFRetained<T>) -> Pooled {
    // SAFETY: every CF object is a CFType; the retain count is carried over
    // by the cast and released exactly once when the `Pooled` drops.
    CfBox::new(unsafe { CFRetained::cast_unchecked::<CFType>(v) })
}

pub(crate) struct ReleaseThread {
    tx: Option<Sender<Vec<Pooled>>>,
    handle: Option<JoinHandle<()>>,
}

impl ReleaseThread {
    pub(crate) fn new() -> Option<ReleaseThread> {
        let (tx, rx) = channel::<Vec<Pooled>>();
        let handle = std::thread::Builder::new()
            .name("cf_release".into())
            .spawn(move || {
                // Dropping each pool releases every object in it.
                while let Ok(pool) = rx.recv() {
                    drop(pool);
                }
                log::debug!("cf release thread exited");
            })
            .inspect_err(|e| log::warn!("error spawning cf release thread: {e}"))
            .ok()?;
        Some(ReleaseThread {
            tx: Some(tx),
            handle: Some(handle),
        })
    }

    /// Send a pool for release. If the thread is gone the pool is released
    /// inline.
    pub(crate) fn release(&self, pool: Vec<Pooled>) {
        if pool.is_empty() {
            return;
        }
        if let Some(tx) = &self.tx
            && let Err(e) = tx.send(pool)
        {
            // Receiver gone: release here.
            drop(e.0);
        }
    }
}

impl Drop for ReleaseThread {
    fn drop(&mut self) {
        // Closing the channel stops the thread after it drains the mailbox.
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
