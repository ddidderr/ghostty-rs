//! Windows crash capture, mirroring Breakpad's in-process `ExceptionHandler`
//! (which sentry-native's breakpad backend uses upstream): an unhandled
//! exception filter hands the exception to a pre-started handler thread, which
//! writes a minidump of the process with `minidump-writer`
//! (`MiniDumpWriteDump`) and attaches it to the envelope (`event.minidump`).
//! Running the dump on a separate thread with its own stack keeps it working
//! after a stack overflow. After writing, the filter returns
//! `EXCEPTION_CONTINUE_SEARCH` so the normal termination (and Windows Error
//! Reporting) proceeds.
//!
//! Faults raised by `__fastfail` (Rust's `abort`) bypass exception filters
//! and are not captured; panics are covered by the panic hook.
#![allow(unsafe_code)]

use std::{
    fs::File,
    hash::{BuildHasher, Hasher, RandomState},
    sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, Ordering},
};

use minidump_writer::{crash_context::CrashContext, minidump_writer::MinidumpWriter};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FALSE, HANDLE, WAIT_OBJECT_0},
    System::{
        Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter},
        Threading::{
            CreateEventW,
            GetCurrentProcessId,
            GetCurrentThreadId,
            INFINITE,
            SetEvent,
            WaitForSingleObject,
        },
    },
};

use crate::{exception, sentry};

/// How long the faulting thread waits for the dump.
const TIMEOUT_MS: u32 = 30_000;

/// `EXCEPTION_CONTINUE_SEARCH`.
const CONTINUE_SEARCH: i32 = 0;

static BUSY: AtomicBool = AtomicBool::new(false);
static REQUEST: AtomicIsize = AtomicIsize::new(0);
static DONE: AtomicIsize = AtomicIsize::new(0);
static POINTERS: AtomicPtr<EXCEPTION_POINTERS> = AtomicPtr::new(std::ptr::null_mut());
static FAULT_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Fill `out` with random bytes (used for the event UUID). The standard
/// library's per-instance random hasher keys are good enough for an id.
pub(crate) fn random_bytes(out: &mut [u8; 16]) {
    for chunk in out.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_usize(chunk.len());
        chunk.copy_from_slice(&h.finish().to_ne_bytes()[..chunk.len()]);
    }
}

/// Start the dump thread and register the unhandled exception filter.
pub(crate) fn install_handlers() {
    // SAFETY: plain Win32 calls; the events live for the whole process.
    let (request, done) = unsafe {
        (
            CreateEventW(std::ptr::null(), FALSE, FALSE, std::ptr::null()),
            CreateEventW(std::ptr::null(), FALSE, FALSE, std::ptr::null()),
        )
    };
    if request.is_null() || done.is_null() {
        return;
    }
    REQUEST.store(request as isize, Ordering::Release);
    DONE.store(done as isize, Ordering::Release);
    let spawned = std::thread::Builder::new()
        .name("ghostty-exception-handler".into())
        .spawn(dump_thread);
    if spawned.is_err() {
        // SAFETY: the handles were just created and are unused.
        unsafe {
            CloseHandle(request);
            CloseHandle(done);
        }
        return;
    }
    // SAFETY: `filter` has the required signature and lives forever.
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
    if sentry::state().is_some() && !BUSY.swap(true, Ordering::AcqRel) {
        POINTERS.store(info.cast_mut(), Ordering::Release);
        // SAFETY: trivial Win32 calls with handles created at install.
        unsafe {
            FAULT_THREAD.store(GetCurrentThreadId(), Ordering::Release);
            SetEvent(REQUEST.load(Ordering::Acquire) as HANDLE);
            let r = WaitForSingleObject(DONE.load(Ordering::Acquire) as HANDLE, TIMEOUT_MS);
            let _ = r == WAIT_OBJECT_0;
        }
    }
    CONTINUE_SEARCH
}

fn dump_thread() {
    loop {
        // SAFETY: handle created at install.
        let r = unsafe { WaitForSingleObject(REQUEST.load(Ordering::Acquire) as HANDLE, INFINITE) };
        if r != WAIT_OBJECT_0 {
            return;
        }
        if let Some(st) = sentry::state() {
            let info = POINTERS.load(Ordering::Acquire);
            write_report(st, info, FAULT_THREAD.load(Ordering::Acquire));
        }
        // SAFETY: handle created at install.
        unsafe { SetEvent(DONE.load(Ordering::Acquire) as HANDLE) };
    }
}

fn write_report(st: &sentry::State, info: *mut EXCEPTION_POINTERS, tid: u32) {
    // SAFETY: the faulting thread is blocked in the filter, so the pointers
    // it passed stay valid.
    let code = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) }
        .map_or(0, |r| r.ExceptionCode) as u32;
    let (kind, desc) = exception::windows_exception(code);

    let dir = std::path::PathBuf::from(String::from_utf8_lossy(&st.dir).into_owned());
    let dump = (|| -> Result<Vec<u8>, String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("minidump-{}.tmp", std::process::id()));
        let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
        let cc = CrashContext {
            exception_pointers: info.cast_const().cast(),
            // SAFETY: trivial Win32 call.
            process_id: unsafe { GetCurrentProcessId() },
            thread_id: tid,
            exception_code: code as i32,
        };
        let r = MinidumpWriter::dump_crash_context(&cc, None, &mut file).map_err(|e| e.to_string());
        drop(file);
        let data = r.and_then(|()| std::fs::read(&tmp).map_err(|e| e.to_string()));
        let _ = std::fs::remove_file(&tmp);
        data
    })();
    let dump = match dump {
        Ok(d) => Some(d),
        Err(e) => {
            log::warn!("minidump failed err={e}");
            None
        }
    };

    let value = format!("{desc} (code=0x{code:08x})");
    match sentry::write_report_ext(
        st,
        kind,
        &value,
        "exceptionfilter",
        None,
        None,
        dump.as_deref(),
    ) {
        Ok(path) => eprintln!(
            "warning(sentry): crash report with minidump written to disk path={}",
            path.display()
        ),
        Err(e) => eprintln!("warning(sentry): failed to persist crash report err={e}"),
    }
}
