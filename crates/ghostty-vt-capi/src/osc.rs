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
    parser: Parser<CByteStorage>,
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
    let Some(storage) = CByteStorage::new(alloc, ghostty_vt::osc::MAX_BUF) else {
        return GhosttyResult::OutOfMemory;
    };
    let Some(p) = alloc.create(OscParser {
        alloc,
        parser: Parser::with_storage(storage),
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
    // The command borrows its payload from the parser's own (caller
    // allocated) storage, so nothing is copied or allocated here except the
    // NUL-terminated title handed to C.
    p.command.title_z = None;
    let cmd = p.parser.end(Some(terminator)).unwrap_or(Command::Invalid);
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
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
        ffi::CStr,
    };

    use super::*;

    /// Counts Rust global-allocator calls made by the current thread, so the
    /// tests can prove that OSC parsing and `ghostty_osc_end` use only the
    /// caller's allocator.
    struct CountingGlobal;
    thread_local! {
        static GLOBAL_ALLOCS: Cell<usize> = const { Cell::new(0) };
    }
    // SAFETY: forwards to the system allocator.
    unsafe impl GlobalAlloc for CountingGlobal {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = GLOBAL_ALLOCS.try_with(|c| c.set(c.get() + 1));
            // SAFETY: same contract.
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            // SAFETY: same contract.
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let _ = GLOBAL_ALLOCS.try_with(|c| c.set(c.get() + 1));
            // SAFETY: same contract.
            unsafe { System.realloc(p, l, n) }
        }
    }
    #[global_allocator]
    static GLOBAL: CountingGlobal = CountingGlobal;

    fn global_allocs() -> usize {
        GLOBAL_ALLOCS.with(Cell::get)
    }

    use std::{
        ffi::c_void,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use crate::alloc::{Allocator, AllocatorVtable};

    /// A caller allocator backed by the system allocator that counts calls.
    unsafe extern "C" fn c_alloc(
        ctx: *mut c_void,
        len: usize,
        align_log2: u8,
        _: usize,
    ) -> *mut c_void {
        // SAFETY: `ctx` is the `AtomicUsize` owned by the test.
        unsafe { &*ctx.cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
        let l = Layout::from_size_align(len, 1 << align_log2).unwrap();
        // SAFETY: non-zero size (the crate never calls with zero).
        unsafe { System.alloc(l) }.cast()
    }
    unsafe extern "C" fn c_resize(
        _: *mut c_void,
        _: *mut c_void,
        _: usize,
        _: u8,
        _: usize,
        _: usize,
    ) -> bool {
        false
    }
    unsafe extern "C" fn c_remap(
        _: *mut c_void,
        _: *mut c_void,
        _: usize,
        _: u8,
        _: usize,
        _: usize,
    ) -> *mut c_void {
        std::ptr::null_mut()
    }
    unsafe extern "C" fn c_free(
        _: *mut c_void,
        m: *mut c_void,
        len: usize,
        align_log2: u8,
        _: usize,
    ) {
        let l = Layout::from_size_align(len, 1 << align_log2).unwrap();
        // SAFETY: allocated by `c_alloc` with the same layout.
        unsafe { System.dealloc(m.cast(), l) };
    }
    static VT: AllocatorVtable = AllocatorVtable {
        alloc: c_alloc,
        resize: c_resize,
        remap: c_remap,
        free: c_free,
    };

    #[test]
    fn end_makes_no_global_allocations() {
        let caller_allocs = AtomicUsize::new(0);
        let a = Allocator {
            ctx: (&raw const caller_allocs).cast_mut().cast(),
            vtable: &raw const VT,
        };
        let mut p = std::ptr::null_mut();
        // SAFETY: valid allocator and out pointer.
        assert_eq!(
            unsafe { ghostty_osc_new(&raw const a, &mut p) },
            GhosttyResult::Success
        );
        // SAFETY: `p` is live for the whole block.
        unsafe {
            for (osc, title) in [
                ("7;file:///tmp/x", false),
                ("8;id=a;https://example.com", false),
                ("8;;", false),
                ("52;c;aGVsbG8=", false),
                ("133;A", false),
                ("777;notify;title;body", false),
                ("9;4;1;50", false),
                ("9;2;message", false),
                ("22;pointer", false),
                ("66;s=2;hi", false),
                ("3008;start=abc;type=shell", false),
                ("5522;type=read;data", false),
                ("0;a title", true),
            ] {
                ghostty_osc_reset(p);
                for b in osc.bytes() {
                    ghostty_osc_next(p, b);
                }
                let global_before = global_allocs();
                let caller_before = caller_allocs.load(Ordering::SeqCst);
                let cmd = ghostty_osc_end(p, 0x07);
                assert_ne!(ghostty_osc_command_type(cmd), 0, "{osc}");
                assert_eq!(
                    global_allocs(),
                    global_before,
                    "global allocation ending {osc}"
                );
                // Only the NUL-terminated title handed to C is allocated.
                assert_eq!(
                    caller_allocs.load(Ordering::SeqCst) - caller_before,
                    usize::from(title),
                    "caller allocations ending {osc}"
                );
            }
            ghostty_osc_free(p);
        }
    }

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
