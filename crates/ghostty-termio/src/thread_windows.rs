//! The Windows event loop of the termio thread (see [`super`]). libxev's IOCP
//! loop is replaced by a single `WaitForMultipleObjects` over: the mailbox
//! wakeup event, the stop event, the child's process handle and a timeout
//! for the deadlines (selection scroll, resize coalescing, synchronized
//! output reset). Writes to the pty are done by the exec backend's writer
//! thread, so there is no write readiness to wait for.

use std::{os::windows::io::AsRawHandle, time::Instant};

use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{INFINITE, WaitForMultipleObjects, WaitForSingleObject},
};

use super::Thread;
use crate::termio::{Termio, ThreadData};

/// The maximum number of milliseconds for one wait (must stay below
/// `INFINITE`).
const MAX_WAIT_MS: u128 = 0xFFFF_FFF0;

/// Milliseconds until `deadline`, rounded up so we never wake early and spin.
fn timeout_ms(deadline: Option<Instant>, now: Instant) -> u32 {
    match deadline {
        None => INFINITE,
        Some(d) => {
            let dur = d.saturating_duration_since(now);
            let ms = dur.as_millis() + u128::from(dur.subsec_nanos() % 1_000_000 != 0);
            ms.min(MAX_WAIT_MS) as u32
        }
    }
}

/// True if the (manual- or auto-reset) object is signaled right now.
#[allow(unsafe_code)]
fn is_signaled(h: HANDLE) -> bool {
    // SAFETY: valid handle owned by the caller for the duration of the call.
    unsafe { WaitForSingleObject(h, 0) == WAIT_OBJECT_0 }
}

impl Thread {
    /// After an abnormal failure: discard messages until the stop signal.
    #[allow(unsafe_code)]
    pub(super) fn drain_until_stop(&mut self, io: &Termio) {
        let mailbox = io.mailbox().clone();
        loop {
            let handles = [
                mailbox.wakeup().as_handle().as_raw_handle() as HANDLE,
                self.stop.as_handle().as_raw_handle() as HANDLE,
            ];
            // SAFETY: `handles` is a valid array of valid handles.
            let rc = unsafe {
                WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE)
            };
            if rc == WAIT_FAILED {
                log::error!(
                    "failed to wait for draining err={}",
                    std::io::Error::last_os_error()
                );
                return;
            }
            // The auto-reset events were consumed by the wait (the one that
            // fired) or by `drain`; check both explicitly like the poll loop.
            let stop = rc == WAIT_OBJECT_0 + 1 || self.stop.drain();
            let msgs = rc == WAIT_OBJECT_0 || mailbox.wakeup().drain();
            if msgs {
                while mailbox.pop().is_some() {}
            }
            if stop {
                return;
            }
        }
    }

    /// The event loop. Returns when stopped.
    #[allow(unsafe_code)]
    pub(super) fn run(&mut self, io: &mut Termio, td: &mut ThreadData) {
        let mailbox = io.mailbox().clone();

        loop {
            let timeout = timeout_ms(self.next_deadline(td), Instant::now());

            // Build the wait set: fixed entries first, optional ones after.
            let mut handles: Vec<HANDLE> = Vec::with_capacity(3);
            handles.push(mailbox.wakeup().as_handle().as_raw_handle() as HANDLE);
            handles.push(self.stop.as_handle().as_raw_handle() as HANDLE);
            let proc_idx = td.backend.as_exec().process_handle().map(|h| {
                handles.push(h.as_raw_handle() as HANDLE);
                handles.len() - 1
            });

            // SAFETY: `handles` is a valid array of valid handles that stay
            // alive (owned by `mailbox`, `self` and `td`) during the wait.
            let rc = unsafe {
                WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, timeout)
            };
            if rc == WAIT_FAILED {
                log::error!(
                    "io thread wait failed err={}",
                    std::io::Error::last_os_error()
                );
                return;
            }

            // WaitForMultipleObjects reports only the lowest signaled index
            // (and consumes it if it is an auto-reset event), so probe the
            // others without waiting. A timeout means nothing is signaled.
            let signaled = |i: usize| -> bool {
                let hit = (rc - WAIT_OBJECT_0) as usize == i;
                hit || (rc != WAIT_TIMEOUT && is_signaled(handles[i]))
            };

            // Stop requested.
            if signaled(1) {
                self.stop.drain();
                return;
            }

            // The mailbox has messages.
            if signaled(0) {
                // When we wake up, we check the mailbox. Mailbox producers
                // should wake up our thread after publishing.
                mailbox.wakeup().drain();
                self.drain_mailbox(io, td);
            }

            if proc_idx.is_some_and(signaled) {
                td.backend.as_exec_mut().on_process_ready();
            }

            self.run_timers(io, td, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn timeout_rounds_up_and_clamps() {
        let now = Instant::now();
        assert_eq!(timeout_ms(None, now), INFINITE);
        assert_eq!(timeout_ms(Some(now), now), 0);
        assert_eq!(timeout_ms(Some(now + Duration::from_millis(25)), now), 25);
        assert_eq!(timeout_ms(Some(now + Duration::from_micros(1500)), now), 2);
        assert!(timeout_ms(Some(now + Duration::from_secs(10_000_000)), now) < INFINITE);
    }
}
