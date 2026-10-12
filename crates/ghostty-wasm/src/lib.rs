//! The Ghostty WASM module. Port of `ZIG/src/main_wasm.zig`.
//!
//! The module exports a C ABI. Like Zig's `main_wasm.zig` it pulls in
//! `os/wasm` (logging, JS interop), `font` (atlas, deferred face, canvas face,
//! shaper), `terminal` and the config/app wasm APIs:
//!
//! - module `atlas`, `font/Atlas.zig` `Wasm` (`atlas_*`)
//! - module `face`, `font/face/web_canvas.zig` `Wasm` (`face_*`) and
//!   `font/DeferredFace.zig` `Wasm` (`deferred_face_*`)
//! - module `shaper`, `font/shaper/web_canvas.zig` `Wasm` (`shaper_*`)
//! - module `config`, `config/Wasm.zig` (`config_*`)
//! - module `app`, `App.zig` `Wasm` (commented out upstream, nothing exported)
//!
//! The host provides the `ghostty_js` imports (see
//! `ghostty_os::wasm::js`) and `env.log`. Strings are passed as
//! pointer+length pairs into wasm memory allocated by the host with
//! [`wasm_alloc`] / released with [`wasm_free`].
//!
//! The crate is empty on non-wasm targets so the native workspace builds.
#![cfg(target_arch = "wasm32")]
// The C ABI needs raw pointers; every unsafe block has a SAFETY comment.
#![allow(unsafe_code)]
// Exports are `extern "C"` taking raw pointers by design.
#![allow(clippy::not_unsafe_ptr_arg_deref, clippy::missing_safety_doc)]

pub mod app;
pub mod atlas;
pub mod config;
pub mod face;
pub mod shaper;

use std::sync::Once;

static INIT: Once = Once::new();

/// Installs the logger and panic hook (Zig: `std_options.logFn`). Called by
/// every constructor export so the host does not need a separate start call;
/// also exported for hosts that want logging before the first constructor.
pub(crate) fn ensure_init() {
    INIT.call_once(|| {
        ghostty_os::wasm::log::init();
        std::panic::set_hook(Box::new(|info| {
            ghostty_os::wasm::log::host_log(&format!("error(panic): {info}"));
        }));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn ghostty_wasm_init() {
    ensure_init();
}

/// Allocates `len` bytes in wasm memory for the host to write into (strings,
/// bitmaps). Release with [`wasm_free`]. Returns null on `len == 0`.
#[unsafe(no_mangle)]
pub extern "C" fn wasm_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    let mut v = vec![0u8; len].into_boxed_slice();
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Releases memory from [`wasm_alloc`].
#[unsafe(no_mangle)]
pub extern "C" fn wasm_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    // SAFETY: `ptr`/`len` came from `wasm_alloc` (boxed slice of `len` bytes).
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// Borrows a host string (`ptr`, `len`) as bytes.
///
/// # Safety
/// `ptr` must be valid for `len` bytes (or `len == 0`).
pub(crate) unsafe fn host_bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        return &[];
    }
    // SAFETY: guaranteed by the caller.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}
