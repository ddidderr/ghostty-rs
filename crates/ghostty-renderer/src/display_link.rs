//! The CVDisplayLink used to drive the rendering loop in sync with the
//! display (`pkg/macos/video/display_link.zig`, used by `generic.zig`).
//!
//! This is only functional on macOS. On every other platform
//! [`DisplayLink`] is an uninhabited type, so a `None` is all the generic
//! renderer ever has (Zig: `DisplayLink = void`).

use std::sync::Arc;

/// The error of display link operations (`DisplayLink.Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid operation")]
    InvalidOperation,
}

/// Called on every vsync from the display link's thread.
pub type Callback = Arc<dyn Fn() + Send + Sync>;

#[cfg(target_os = "macos")]
pub use self::macos::DisplayLink;
#[cfg(not(target_os = "macos"))]
pub use self::unsupported::DisplayLink;

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use std::convert::Infallible;

    use super::{Callback, Error};

    /// There is no display link on this platform.
    pub struct DisplayLink(Infallible);

    impl DisplayLink {
        pub fn create_with_active_cg_displays() -> Option<DisplayLink> {
            None
        }

        pub fn start(&self) -> Result<(), Error> {
            match self.0 {}
        }

        pub fn stop(&self) -> Result<(), Error> {
            match self.0 {}
        }

        pub fn is_running(&self) -> bool {
            match self.0 {}
        }

        pub fn set_current_cg_display(&self, _display_id: u32) -> Result<(), Error> {
            match self.0 {}
        }

        pub fn set_output_callback(&self, _callback: Callback) -> Result<(), Error> {
            match self.0 {}
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code, deprecated)]
mod macos {
    use std::{
        ffi::c_void,
        ptr::NonNull,
        sync::{Mutex, PoisonError},
    };

    use objc2_core_foundation::CFRetained;
    use objc2_core_video::{CVDisplayLink, CVOptionFlags, CVReturn, CVTimeStamp, kCVReturnSuccess};

    use super::{Callback, Error};

    /// A CVDisplayLink.
    pub struct DisplayLink {
        link: CFRetained<CVDisplayLink>,

        /// The callback handed to CoreVideo as `user_info` (a leaked
        /// `Box<Callback>`), freed when the link is dropped or the callback is
        /// replaced.
        callback: Mutex<Option<NonNull<Callback>>>,
    }

    // SAFETY: CVDisplayLink is a thread-safe CoreVideo object; the callback
    // pointer is only freed with the lock held, after the link is stopped.
    unsafe impl Send for DisplayLink {}
    // SAFETY: see above.
    unsafe impl Sync for DisplayLink {}

    fn check(r: CVReturn) -> Result<(), Error> {
        if r == kCVReturnSuccess {
            Ok(())
        } else {
            Err(Error::InvalidOperation)
        }
    }

    impl DisplayLink {
        /// `createWithActiveCGDisplays`.
        pub fn create_with_active_cg_displays() -> Option<DisplayLink> {
            let mut out: *mut CVDisplayLink = std::ptr::null_mut();
            // SAFETY: `out` is a valid out pointer.
            let r =
                unsafe { CVDisplayLink::create_with_active_cg_displays(NonNull::from(&mut out)) };
            if r != kCVReturnSuccess {
                return None;
            }
            let ptr = NonNull::new(out)?;
            // SAFETY: the function returns a +1 retained link.
            let link = unsafe { CFRetained::from_raw(ptr) };
            Some(DisplayLink {
                link,
                callback: Mutex::new(None),
            })
        }

        pub fn start(&self) -> Result<(), Error> {
            check(self.link.start())
        }

        pub fn stop(&self) -> Result<(), Error> {
            check(self.link.stop())
        }

        pub fn is_running(&self) -> bool {
            self.link.is_running()
        }

        pub fn set_current_cg_display(&self, display_id: u32) -> Result<(), Error> {
            check(self.link.set_current_cg_display(display_id))
        }

        /// Set the callback called on every vsync.
        ///
        /// Note: this purposely throws away a ton of arguments we don't need.
        pub fn set_output_callback(&self, callback: Callback) -> Result<(), Error> {
            let boxed = NonNull::from(Box::leak(Box::new(callback)));
            // SAFETY: `trampoline` matches the callback signature and
            // `user_info` stays valid until the link is dropped.
            let r = unsafe {
                self.link
                    .set_output_callback(Some(trampoline), boxed.as_ptr().cast::<c_void>())
            };
            if let Err(e) = check(r) {
                // SAFETY: we just leaked this box and CoreVideo rejected it.
                drop(unsafe { Box::from_raw(boxed.as_ptr()) });
                return Err(e);
            }

            let old = self
                .callback
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .replace(boxed);
            if let Some(old) = old {
                // Make sure the old callback can't be running anymore.
                let _ = self.link.stop();
                // SAFETY: the pointer came from `Box::leak` above and the
                // link no longer references it (replaced + stopped).
                drop(unsafe { Box::from_raw(old.as_ptr()) });
            }
            Ok(())
        }
    }

    impl Drop for DisplayLink {
        fn drop(&mut self) {
            let _ = self.link.stop();
            if let Some(cb) = self
                .callback
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                // SAFETY: the link is stopped and released right after.
                drop(unsafe { Box::from_raw(cb.as_ptr()) });
            }
        }
    }

    unsafe extern "C-unwind" fn trampoline(
        _display_link: NonNull<CVDisplayLink>,
        _in_now: NonNull<CVTimeStamp>,
        _in_output_time: NonNull<CVTimeStamp>,
        _flags_in: CVOptionFlags,
        _flags_out: NonNull<CVOptionFlags>,
        user_info: *mut c_void,
    ) -> CVReturn {
        // SAFETY: `user_info` is the leaked `Box<Callback>` set in
        // `set_output_callback`, alive until the link is stopped.
        if let Some(cb) = unsafe { user_info.cast::<Callback>().as_ref() } {
            cb();
        }
        kCVReturnSuccess
    }
}
