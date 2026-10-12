//! `config/Wasm.zig`: the wasm-compatible config API.

use std::io::Cursor;

use ghostty_config::{
    cli_args::{self, LineIterator},
    config::Config,
    file_formatter::FileFormatter,
};

use crate::{ensure_init, host_bytes};

/// Create a new configuration filled with the initial default values.
#[unsafe(no_mangle)]
pub extern "C" fn config_new() -> *mut Config {
    ensure_init();
    ghostty_config::config::set_app_runtime(ghostty_config::config::AppRuntime::None);
    Box::into_raw(Box::new(Config::new_default()))
}

#[unsafe(no_mangle)]
pub extern "C" fn config_free(ptr: *mut Config) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `config_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// Load the configuration from a string in the same format as the file-based
/// syntax for the desktop version of the terminal.
#[unsafe(no_mangle)]
pub extern "C" fn config_load_string(config: *mut Config, s: *const u8, len: usize) {
    // SAFETY: live pointer from `config_new`; `s` is valid for `len` bytes.
    let (config, s) = unsafe { (&mut *config, host_bytes(s, len)) };
    let mut iter = LineIterator::new(Cursor::new(s));
    if let Err(err) = cli_args::parse(config, &mut iter) {
        log::error!("error loading config err={err}");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn config_finalize(config: *mut Config) {
    // SAFETY: live pointer from `config_new`.
    if let Err(err) = unsafe { &mut *config }.finalize() {
        log::error!("error finalizing config err={err}");
    }
}

/// Extra (not in Zig): writes the config in file format (like
/// `+show-config`, only changed values) to `out` (capacity `cap`) and returns
/// the full length; the output is truncated if it does not fit.
#[unsafe(no_mangle)]
pub extern "C" fn config_format(config: *const Config, out: *mut u8, cap: usize) -> usize {
    // SAFETY: live pointer from `config_new`; `out` is valid for `cap` bytes.
    let (config, out) = unsafe { (&*config, std::slice::from_raw_parts_mut(out, cap)) };
    let mut fmt = FileFormatter::new(config);
    fmt.changed = true;
    let s = fmt.to_string();
    let n = s.len().min(cap);
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    s.len()
}

/// Extra (not in Zig): the number of diagnostics (parse errors) recorded.
#[unsafe(no_mangle)]
pub extern "C" fn config_diagnostics_count(config: *const Config) -> usize {
    // SAFETY: live pointer from `config_new`.
    unsafe { &*config }.diagnostics.items().len()
}
