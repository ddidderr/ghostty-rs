//! Darwin Instruments signposts (port of `pkg/macos/os/{log,signpost}.zig`,
//! the subset used by `benchmark/Benchmark.zig`). macOS/iOS only.
//!
//! The `os_signpost_*` C macros expand to calls of
//! `_os_signpost_emit_with_name_impl`, which is what is declared here.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};

/// `OS_SIGNPOST_INTERVAL_BEGIN` / `OS_SIGNPOST_INTERVAL_END`.
const INTERVAL_BEGIN: u8 = 1;
const INTERVAL_END: u8 = 2;

/// `os_log_t` handle category that shows up as the "Points of Interest" track
/// in Instruments.
pub const CATEGORY_POINTS_OF_INTEREST: &CStr = c"PointsOfInterest";

unsafe extern "C" {
    /// The Mach-O header of the image this code is linked into, provided by
    /// the linker. Unlike Zig (ziglang/zig#24370) Rust can reference it.
    static __dso_handle: c_void;

    fn os_log_create(subsystem: *const c_char, category: *const c_char) -> *mut c_void;
    fn os_release(object: *mut c_void);
    fn os_signpost_id_make_with_pointer(log: *mut c_void, ptr: *const c_void) -> u64;
    fn _os_signpost_emit_with_name_impl(
        dso: *const c_void,
        log: *mut c_void,
        typ: u8,
        spid: u64,
        name: *const c_char,
        format: *const c_char,
        buf: *mut u8,
        size: u32,
    );
}

/// An `os_log_t` plus the signpost id of one interval. The interval ends (and
/// the log is released) on drop.
pub struct Interval {
    log: *mut c_void,
    id: u64,
    name: &'static CStr,
}

impl Interval {
    /// Begin an interval named `name` in `subsystem`/"PointsOfInterest",
    /// correlated by the address `key` (Zig: `Id.forPointer(log, self.ptr)`).
    #[must_use]
    pub fn begin(subsystem: &CStr, name: &'static CStr, key: usize) -> Option<Interval> {
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let log =
            unsafe { os_log_create(subsystem.as_ptr(), CATEGORY_POINTS_OF_INTEREST.as_ptr()) };
        if log.is_null() {
            return None;
        }
        // SAFETY: `log` is a valid os_log_t; `key` is only used as an opaque key.
        let id = unsafe { os_signpost_id_make_with_pointer(log, key as *const c_void) };
        let i = Interval { log, id, name };
        i.emit(INTERVAL_BEGIN);
        Some(i)
    }

    fn emit(&self, typ: u8) {
        // Two zero bytes: summary flags and argument count (no arguments).
        let mut buf = [0u8; 2];
        // SAFETY: `log` is valid for the lifetime of `self`, `name` is a
        // NUL-terminated static string, `format` is "" and `buf` is the
        // 2-byte empty argument buffer the C macro would build.
        unsafe {
            _os_signpost_emit_with_name_impl(
                &raw const __dso_handle,
                self.log,
                typ,
                self.id,
                self.name.as_ptr(),
                c"".as_ptr(),
                buf.as_mut_ptr(),
                2,
            );
        }
    }
}

impl Drop for Interval {
    fn drop(&mut self) {
        self.emit(INTERVAL_END);
        // SAFETY: `log` came from `os_log_create` (+1 retained) and is not
        // used afterwards.
        unsafe { os_release(self.log) };
    }
}
