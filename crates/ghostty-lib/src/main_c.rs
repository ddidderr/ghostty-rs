//! `ghostty_init`, `ghostty_cli_try_action`, `ghostty_info`,
//! `ghostty_translate` and `ghostty_string_free` (ZIG/src/main_c.zig).
#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    ffi::{CStr, CString, OsString, c_char, c_int},
    sync::{Mutex, OnceLock},
};

use ghostty_cli::build_info::{self, BuildInfo};
use ghostty_core::global::{self, InitOptions};
use ghostty_os::osbytes;

use crate::types::{build_mode, ghostty_info_s, ghostty_string_s};

/// The CLI arguments of the process, as given to `ghostty_init` (Zig:
/// `std.os.argv`).
static ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// Build the `ghostty_string_s` for an owned string (Zig:
/// `String.fromSlice` of an allocated `[:0]const u8`). Free with
/// `ghostty_string_free`.
pub(crate) fn string_from_owned(s: &str) -> ghostty_string_s {
    let mut bytes = Vec::with_capacity(s.len() + 1);
    bytes.extend_from_slice(s.as_bytes());
    bytes.push(0);
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len() - 1;
    let ptr = Box::into_raw(boxed).cast::<c_char>();
    ghostty_string_s {
        ptr,
        len,
        sentinel: true,
    }
}

/// The empty string (`String.empty`).
pub(crate) const EMPTY_STRING: ghostty_string_s = ghostty_string_s {
    ptr: std::ptr::null(),
    len: 0,
    sentinel: false,
};

/// Free text allocated with [`alloc_c_text`]: `text_len` bytes plus the
/// terminating NUL.
///
/// # Safety
/// `ptr` must come from [`alloc_c_text`] (or `string_from_owned`) with the
/// same `len` and not have been freed.
pub(crate) unsafe fn free_c_text(ptr: *const c_char, len: usize, sentinel: bool) {
    if ptr.is_null() {
        return;
    }
    let total = len + usize::from(sentinel);
    // SAFETY: per the function contract the allocation is a boxed slice of
    // exactly `total` bytes.
    drop(unsafe {
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr.cast_mut().cast::<u8>(),
            total,
        ))
    });
}

/// Allocate a NUL terminated copy of `bytes`; returns the pointer (free with
/// [`free_c_text`] with `sentinel = true`).
pub(crate) fn alloc_c_text(bytes: &[u8]) -> *const c_char {
    let mut v = Vec::with_capacity(bytes.len() + 1);
    v.extend_from_slice(bytes);
    v.push(0);
    Box::into_raw(v.into_boxed_slice()).cast::<c_char>()
}

/// Initialize ghostty global state.
///
/// # Safety
/// `argv` must be null or point to `argc` valid NUL terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_init(argc: usize, argv: *mut *mut c_char) -> c_int {
    // Raw argument bytes (Zig: `std.os.argv = argv[0..argc]`).
    let mut raw: Vec<OsString> = Vec::with_capacity(argc);
    if !argv.is_null() {
        for i in 0..argc {
            // SAFETY: the caller guarantees `argv[0..argc]` are valid.
            let p = unsafe { *argv.add(i) };
            if p.is_null() {
                break;
            }
            // SAFETY: each argv entry is a NUL terminated string.
            let c = unsafe { CStr::from_ptr(p) };
            raw.push(osbytes::os_from_bytes(c.to_bytes().to_vec()));
        }
    }

    // Later config CLI loading (`ghostty_config_load_cli_args`) must see
    // these arguments rather than the process arguments.
    ghostty_os::args::set_argv(raw.clone());
    let args: Vec<String> = raw
        .iter()
        .map(|a| osbytes::os_to_config_string(a))
        .collect();

    // We first try to parse any action that we may be executing.
    let action = match ghostty_cli::ghostty::detect(&args) {
        Ok(a) => a,
        Err(err) => {
            log::error!("failed to initialize ghostty error={err:?}");
            return 1;
        }
    };
    let _ = ARGS.set(args);

    // `+version` and friends report the library build.
    build_info::set(lib_build_info());

    // Setup our signal handlers before logging (Zig: `initSignals`). A C or
    // Swift host does not get the Rust runtime's SIGPIPE ignore.
    init_signals();

    global::init_lib(InitOptions {
        action: action.is_some(),
        resources_dir: None,
        crash: Some(lib_crash_options()),
    });

    0
}

/// Ignore SIGPIPE: it is common with how termio shuts down read threads
/// (Zig: `GlobalState.initSignals`; not on Windows).
fn init_signals() {
    #[cfg(unix)]
    // SAFETY: installing SIG_IGN for SIGPIPE has no memory safety impact.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// Crash metadata for the library artifact: no app runtime, CoreText +
/// Metal on Apple targets (so the GTK / fontconfig / OpenGL defaults are
/// not reported for the macOS app).
fn lib_crash_options() -> ghostty_crash::Options {
    let info = build_info::get();
    let tag = |s: &str| s.trim_start_matches('.').to_owned();
    let mut o = global::default_crash_options();
    o.release.clone_from(&info.version_string);
    o.app_runtime = tag(&lib_build_info().app_runtime);
    if cfg!(target_vendor = "apple") {
        "coretext".clone_into(&mut o.font_backend);
        "metal".clone_into(&mut o.renderer);
    }
    o
}

/// The build info of the library (Zig: `build_config` for the lib artifact:
/// no app runtime, CoreText + Metal on Apple targets).
fn lib_build_info() -> BuildInfo {
    let mut info = BuildInfo {
        app_runtime: ".none".to_owned(),
        gtk_build: None,
        gtk_runtime: None,
        adw_build: None,
        adw_runtime: None,
        x11: false,
        wayland: false,
        ..BuildInfo::default()
    };
    if cfg!(target_vendor = "apple") {
        ".coretext".clone_into(&mut info.font_backend);
        "renderer.generic.Renderer(renderer.Metal)".clone_into(&mut info.renderer);
    }
    info
}

/// Runs an action if it is specified. If there is no action this returns.
/// If there is an action then this doesn't return.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_cli_try_action() {
    let Some(args) = ARGS.get() else {
        return;
    };
    // No action: nothing to do.
    if !matches!(ghostty_cli::ghostty::detect(args), Ok(Some(_))) {
        return;
    }
    log::info!("executing CLI action");
    let code = ghostty_cli::run_from_args(args).unwrap_or(0);
    std::process::exit(i32::from(code));
}

/// Return metadata about Ghostty, such as version, build mode, etc.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_info() -> ghostty_info_s {
    static VERSION: OnceLock<CString> = OnceLock::new();
    let info = build_info::get();
    let version = VERSION.get_or_init(|| CString::new(info.version_string).unwrap_or_default());
    let mode = match info.build_mode.trim_start_matches('.') {
        "Debug" => build_mode::DEBUG,
        "ReleaseSafe" => build_mode::RELEASE_SAFE,
        "ReleaseFast" => build_mode::RELEASE_FAST,
        "ReleaseSmall" => build_mode::RELEASE_SMALL,
        _ if cfg!(debug_assertions) => build_mode::DEBUG,
        _ => build_mode::RELEASE_SAFE,
    };
    ghostty_info_s {
        build_mode: mode,
        version: version.as_ptr(),
        version_len: version.as_bytes().len(),
    }
}

/// Translate a string maintained by libghostty into the current application
/// language. This will return the same string (same pointer) if no
/// translation is found, so the pointer must be stable through the function
/// call.
///
/// # Safety
/// `msgid` must be a valid NUL terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_translate(msgid: *const c_char) -> *const c_char {
    static CACHE: OnceLock<Mutex<HashMap<String, CString>>> = OnceLock::new();
    if msgid.is_null() {
        return msgid;
    }

    // SAFETY: the caller guarantees `msgid` is NUL terminated.
    let id = unsafe { CStr::from_ptr(msgid) };
    let Ok(id_str) = id.to_str() else {
        return msgid;
    };
    let translated = ghostty_os::i18n::tr(id_str);
    if translated == id_str {
        return msgid;
    }

    // Translations are interned so the returned pointer stays valid.
    let Ok(mut cache) = CACHE.get_or_init(Mutex::default).lock() else {
        return msgid;
    };
    if let Some(c) = cache.get(&translated) {
        return c.as_ptr();
    }
    let Ok(c) = CString::new(translated.clone()) else {
        return msgid;
    };
    let ptr = c.as_ptr();
    cache.insert(translated, c);
    ptr
}

/// Free a string allocated by Ghostty.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_string_free(str: ghostty_string_s) {
    // SAFETY: strings handed out by this library are allocated by
    // `string_from_owned` / `alloc_c_text` with `len` (+1 if sentinel) bytes.
    unsafe { free_c_text(str.ptr, str.len, str.sentinel) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghostty_string_s_empty_string() {
        let empty = EMPTY_STRING;
        ghostty_string_free(empty);
        assert_eq!(empty.len, 0);
        assert!(!empty.sentinel);
    }

    #[test]
    fn ghostty_string_s_c_string() {
        let s = string_from_owned("hello");
        assert_eq!(s.len, 5);
        assert!(s.sentinel);
        // SAFETY: just allocated with a NUL terminator.
        let c = unsafe { CStr::from_ptr(s.ptr) };
        assert_eq!(c.to_str().unwrap(), "hello");
        ghostty_string_free(s);
    }

    #[test]
    fn info_version() {
        let info = ghostty_info();
        assert!(info.version_len > 0);
        // SAFETY: the version pointer is a static NUL terminated string.
        let v = unsafe { CStr::from_ptr(info.version) };
        assert_eq!(v.to_bytes().len(), info.version_len);
    }

    #[test]
    fn translate_returns_same_pointer_without_translation() {
        let id = c"this message has no translation";
        // SAFETY: valid NUL terminated string.
        let out = unsafe { ghostty_translate(id.as_ptr()) };
        assert_eq!(out, id.as_ptr());
    }
}
