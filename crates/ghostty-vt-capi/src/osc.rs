//! OSC parser C API (ZIG/src/terminal/c/osc.zig, `include/ghostty/vt/osc.h`).
#![allow(unsafe_code)]

use std::{
    ffi::{c_int, c_void},
    ptr::NonNull,
};

use ghostty_vt::osc::{Command, Parser};

use crate::{
    alloc::{Alloc, Allocator, CBlock, CByteStorage},
    result::GhosttyResult,
};

/// C: `GhosttyOscParser` (opaque).
pub struct OscParser {
    alloc: Alloc,
    parser: Parser,
    /// Backing store for the last command returned by `ghostty_osc_end`.
    command: OscCommand,
}

/// C: `GhosttyOscCommand` (opaque).
///
/// Zig hands out a pointer to the parser's own command; the Rust `Command`
/// is returned by value, so it is parked here (with a NUL-terminated copy of
/// string payloads, as Zig's sentinel slices are) and stays valid until the
/// next `ghostty_osc_end`/`ghostty_osc_reset`/`ghostty_osc_free`.
pub struct OscCommand {
    /// `GhosttyOscCommandType` of the last command.
    key: c_int,
    /// NUL-terminated window title (only for `ChangeWindowTitle`), allocated
    /// with the caller's allocator.
    title_z: Option<CBlock>,
}

/// C: `GhosttyOscCommandData`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum OscCommandData {
    Invalid = 0,
    ChangeWindowTitleStr = 1,
}

impl OscCommandData {
    fn from_c(v: c_int) -> Option<Self> {
        match v {
            0 => Some(Self::Invalid),
            1 => Some(Self::ChangeWindowTitleStr),
            _ => None,
        }
    }
}

/// # Safety
/// `allocator` must be null or valid; `result` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_new(
    allocator: *const Allocator,
    result: *mut *mut OscParser,
) -> GhosttyResult {
    // SAFETY: forwarded caller contract.
    let alloc = unsafe { Alloc::from_c(allocator) };
    // C-built parsers have an allocator (Zig: `.init(alloc)`), so commands
    // that need unbounded buffers (OSC 52, ...) are accepted.
    // The parser's payload buffer (Zig's inline fixed buffer / allocating
    // writer) lives in memory from the same allocator; if either allocation
    // fails the caller gets OUT_OF_MEMORY.
    let Some(storage) = CByteStorage::new(alloc, Parser::MAX_BUF) else {
        return GhosttyResult::OutOfMemory;
    };
    let Some(p) = alloc.create(OscParser {
        alloc,
        parser: Parser::with_storage(Box::new(storage)),
        command: OscCommand {
            key: 0,
            title_z: None,
        },
    }) else {
        return GhosttyResult::OutOfMemory;
    };
    // SAFETY: caller guarantees `result` is valid for writes.
    unsafe { *result = p.as_ptr() };
    GhosttyResult::Success
}

/// # Safety
/// `parser` must be null or a live parser from `ghostty_osc_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_free(parser: *mut OscParser) {
    let Some(p) = NonNull::new(parser) else {
        return;
    };
    // SAFETY: `p` came from `Alloc::create` in `ghostty_osc_new`.
    unsafe {
        let alloc = p.as_ref().alloc;
        alloc.destroy(p);
    }
}

/// # Safety
/// `parser` must be null or a live parser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_reset(parser: *mut OscParser) {
    // SAFETY: caller guarantees a live parser (or null).
    if let Some(p) = unsafe { parser.as_mut() } {
        p.parser.reset();
    }
}

/// # Safety
/// `parser` must be null or a live parser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_next(parser: *mut OscParser, byte: u8) {
    // SAFETY: caller guarantees a live parser (or null).
    if let Some(p) = unsafe { parser.as_mut() } {
        p.parser.next(byte);
    }
}

/// # Safety
/// `parser` must be null or a live parser. The returned command is owned by
/// the parser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_end(
    parser: *mut OscParser,
    terminator: u8,
) -> *mut OscCommand {
    // SAFETY: caller guarantees a live parser (or null).
    let Some(p) = (unsafe { parser.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let cmd = p.parser.end(Some(terminator)).unwrap_or(Command::Invalid);
    p.command.title_z = None;
    p.command.key = cmd.key() as c_int;
    if let Command::ChangeWindowTitle(t) = &cmd {
        match CBlock::from_bytes_z(p.alloc, t) {
            Some(b) => p.command.title_z = Some(b),
            // The title cannot be materialized with the caller's allocator:
            // report an invalid command rather than a dangling title.
            None => p.command.key = Command::Invalid.key() as c_int,
        }
    }
    &raw mut p.command
}

/// Returns the `GhosttyOscCommandType` (an `int` enum).
///
/// # Safety
/// `command` must be null or a command returned by `ghostty_osc_end`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_command_type(command: *mut OscCommand) -> c_int {
    // SAFETY: caller guarantees validity (or null).
    match unsafe { command.as_ref() } {
        Some(c) => c.key,
        None => 0, // invalid
    }
}

/// # Safety
/// `command` must be null or valid; `out` must point to storage of the type
/// documented for `data` (for the title: a `const char *`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_osc_command_data(
    command: *mut OscCommand,
    data: c_int,
    out: *mut c_void,
) -> bool {
    let Some(data) = OscCommandData::from_c(data) else {
        log::warn!("commandData invalid data value={data}");
        return false;
    };
    // SAFETY: caller guarantees validity (or null).
    let Some(command) = (unsafe { command.as_ref() }) else {
        return false;
    };
    match data {
        OscCommandData::Invalid => false,
        OscCommandData::ChangeWindowTitleStr => match &command.title_z {
            Some(title) if !out.is_null() => {
                // SAFETY: caller guarantees `out` is a `const char **`.
                unsafe {
                    out.cast::<*const u8>().write_unaligned(title.as_ptr());
                }
                true
            }
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use super::*;

    fn new() -> *mut OscParser {
        let mut p = std::ptr::null_mut();
        // SAFETY: valid out pointer, default allocator.
        assert_eq!(
            unsafe { ghostty_osc_new(std::ptr::null(), &mut p) },
            GhosttyResult::Success
        );
        p
    }

    #[test]
    fn alloc() {
        let p = new();
        // SAFETY: `p` is live.
        unsafe { ghostty_osc_free(p) };
    }

    #[test]
    fn command_type_null() {
        // SAFETY: null is allowed.
        assert_eq!(unsafe { ghostty_osc_command_type(std::ptr::null_mut()) }, 0);
    }

    #[test]
    fn change_window_title() {
        let p = new();
        // SAFETY: `p` is live; `title` is a valid out slot.
        unsafe {
            ghostty_osc_next(p, b'0');
            ghostty_osc_next(p, b';');
            ghostty_osc_next(p, b'a');
            let cmd = ghostty_osc_end(p, 0);
            assert_eq!(ghostty_osc_command_type(cmd), 1);
            let mut title: *const u8 = std::ptr::null();
            assert!(ghostty_osc_command_data(cmd, 1, (&raw mut title).cast()));
            assert_eq!(CStr::from_ptr(title.cast()).to_str().unwrap(), "a");
            assert!(!ghostty_osc_command_data(cmd, 0, std::ptr::null_mut()));
            assert!(!ghostty_osc_command_data(cmd, 99, std::ptr::null_mut()));
            ghostty_osc_free(p);
        }
    }

    #[test]
    fn invalid_command_is_type_zero() {
        let p = new();
        // SAFETY: `p` is live.
        unsafe {
            ghostty_osc_next(p, b'x');
            let cmd = ghostty_osc_end(p, 0);
            assert_eq!(ghostty_osc_command_type(cmd), 0);
            ghostty_osc_free(p);
        }
    }
}
