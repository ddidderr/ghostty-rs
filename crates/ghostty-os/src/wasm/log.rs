//! Freestanding log sink. Port of `ZIG/src/os/wasm/log.zig`.
//!
//! Formats `"<level>(<scope>): <message>"` (or `"<level>: <message>"` for
//! the default scope) and hands it to the host through the `env.log` import,
//! exactly like Ghostty's `Freestanding.log`.
#![allow(unsafe_code)]

use ::log::{Level, LevelFilter, Log, Metadata, Record};

mod ffi {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        /// `extern "env" fn log(ptr: [*]const u8, len: usize) void`
        pub fn log(ptr: *const u8, len: usize);
    }
}

/// Zig `std.log.Level.asText()`.
fn level_text(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warning",
        Level::Info => "info",
        Level::Debug | Level::Trace => "debug",
    }
}

/// Formats a record like the Zig log function does.
pub fn format_record(level: Level, scope: &str, args: &std::fmt::Arguments<'_>) -> String {
    let prefix = if scope.is_empty() || scope == "default" {
        ": ".to_owned()
    } else {
        format!("({scope}): ")
    };
    format!("{}{prefix}{args}", level_text(level))
}

/// Sends a string to the host.
pub fn host_log(s: &str) {
    // SAFETY: the pointer/length pair describes a live UTF-8 buffer and the
    // host only reads it for the duration of the call.
    unsafe { ffi::log(s.as_ptr(), s.len()) }
}

struct Logger;

impl Log for Logger {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        host_log(&format_record(
            record.level(),
            record.target(),
            record.args(),
        ));
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

/// Installs the logger with errors only in every build mode.
pub fn init() {
    if ::log::set_logger(&LOGGER).is_ok() {
        ::log::set_max_level(LevelFilter::Error);
    }
}
