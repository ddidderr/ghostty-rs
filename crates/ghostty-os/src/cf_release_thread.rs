//! The CFRelease thread. Port of `os/cf_release_thread.zig`.
//!
//! Pools of `CFTypeRef`s are sent to this thread to be released, so that
//! their release callback logic doesn't block the execution of a high
//! throughput thread like the renderer thread.
//!
//! The libxev loop of the Zig version (wakeup + stop async handles around a
//! blocking mailbox) is replaced by a bounded channel (capacity 64, like
//! `BlockingQueue(Message, 64)`) plus a stop channel; a `select!` loop
//! drains the mailbox. The release function is injectable so the thread
//! logic is testable on every platform; on Darwin the default is
//! `CFRelease`.

use std::{ffi::c_void, ptr::NonNull, thread::JoinHandle};

use crossbeam_channel::{Receiver, Sender, bounded, select};

/// Mailbox capacity (Zig: `BlockingQueue(Message, 64)`).
pub const MAILBOX_CAPACITY: usize = 64;

/// A raw `CFTypeRef` owned by the sender (+1 retain to be balanced by one
/// release on this thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfRef(pub NonNull<c_void>);

// SAFETY: a CFTypeRef is a thread-safe, reference-counted handle; ownership
// of one retain count is transferred with the message.
#[allow(unsafe_code)]
unsafe impl Send for CfRef {}

#[derive(Debug)]
pub enum Message {
    /// Release a list of CFTypeRefs.
    Release(Vec<CfRef>),
}

/// Function used to release one ref.
pub type ReleaseFn = fn(CfRef);

/// Darwin `CFRelease`.
#[cfg(target_vendor = "apple")]
pub fn cf_release(r: CfRef) {
    use objc2_core_foundation::{CFRetained, CFType};

    // SAFETY: the sender transferred one retain count to us, and `r` is a
    // valid non-null CFTypeRef; dropping the CFRetained releases it.
    #[allow(unsafe_code)]
    unsafe {
        drop(CFRetained::from_raw(r.0.cast::<CFType>()));
    }
}

/// Handle used to talk to a running [`Thread`].
#[derive(Clone)]
pub struct Handle {
    mailbox: Sender<Message>,
    stop: Sender<()>,
}

impl Handle {
    /// Push a message, blocking if the mailbox is full (`.forever`).
    /// Returns false if the thread is gone.
    pub fn send(&self, msg: Message) -> bool {
        self.mailbox.send(msg).is_ok()
    }

    /// Ask the thread to stop on the next loop iteration.
    pub fn stop(&self) {
        let _ = self.stop.try_send(());
    }
}

/// The CFRelease thread (not yet started).
pub struct Thread {
    mailbox_rx: Receiver<Message>,
    stop_rx: Receiver<()>,
    handle: Handle,
    release: ReleaseFn,
}

pub struct Running {
    pub handle: Handle,
    join: JoinHandle<()>,
}

impl Running {
    /// Stop the thread and wait for it to exit.
    pub fn stop_and_join(self) {
        self.handle.stop();
        let _ = self.join.join();
    }
}

impl Thread {
    /// Initialize the thread with the default (`CFRelease`) release function.
    #[cfg(target_vendor = "apple")]
    pub fn init() -> Thread {
        Thread::with_release(cf_release)
    }

    /// Initialize the thread with a custom release function.
    pub fn with_release(release: ReleaseFn) -> Thread {
        let (tx, mailbox_rx) = bounded(MAILBOX_CAPACITY);
        let (stop_tx, stop_rx) = bounded(1);
        Thread {
            mailbox_rx,
            stop_rx,
            handle: Handle {
                mailbox: tx,
                stop: stop_tx,
            },
            release,
        }
    }

    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Spawn the thread (named "cf_release" like the Zig version).
    pub fn spawn(self) -> std::io::Result<Running> {
        let handle = self.handle.clone();
        let join = std::thread::Builder::new()
            .name("cf_release".into())
            .spawn(move || self.thread_main())?;
        Ok(Running { handle, join })
    }

    /// The main entrypoint for the thread.
    pub fn thread_main(self) {
        // Right now, on Darwin, `std::thread::Builder::name` already names
        // the thread through pthread_setname_np for the current thread, but
        // keep the explicit call for parity with Zig.
        #[cfg(target_vendor = "apple")]
        crate::macos::pthread_setname_np(c"cf_release");

        log::debug!("starting cf release thread");
        loop {
            select! {
                recv(self.mailbox_rx) -> msg => match msg {
                    Ok(msg) => self.handle_message(msg),
                    // All senders gone: nothing more can arrive.
                    Err(_) => break,
                },
                recv(self.stop_rx) -> _ => break,
            }
        }

        // Messages published before the stop request are still released so
        // we never leak refs.
        while let Ok(msg) = self.mailbox_rx.try_recv() {
            self.handle_message(msg);
        }
        log::debug!("cf release thread exited");
    }

    fn handle_message(&self, msg: Message) {
        match msg {
            Message::Release(refs) => {
                for r in refs {
                    (self.release)(r);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static RELEASED: AtomicUsize = AtomicUsize::new(0);

    fn count_release(_: CfRef) {
        RELEASED.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn releases_all_refs_before_exit() {
        let t = Thread::with_release(count_release);
        let running = t.spawn().unwrap();
        for _ in 0..3 {
            let refs = (1..=5usize)
                .map(|i| CfRef(NonNull::new(i as *mut c_void).unwrap()))
                .collect();
            assert!(running.handle.send(Message::Release(refs)));
        }
        running.stop_and_join();
        assert_eq!(RELEASED.load(Ordering::SeqCst), 15);
    }
}
