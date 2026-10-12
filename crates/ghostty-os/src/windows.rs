//! Windows API helpers (`os/windows.zig`). Zig re-exports a handful of
//! kernel32 items and declares the ConPTY externs that `std` lacks; here the
//! `windows-sys` crate provides all of those, so this module only holds the
//! small shared helpers plus the constants Zig computes by hand.

/// Re-export so dependants can name Windows API items through one path.
pub use windows_sys as sys;

/// `ProcThreadAttributeNumber.ProcThreadAttributePseudoConsole`.
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE_NUMBER: u32 = 22;
const PROC_THREAD_ATTRIBUTE_NUMBER: u32 = 0x0000_FFFF;
const PROC_THREAD_ATTRIBUTE_THREAD: u32 = 0x0001_0000;
const PROC_THREAD_ATTRIBUTE_INPUT: u32 = 0x0002_0000;
const PROC_THREAD_ATTRIBUTE_ADDITIVE: u32 = 0x0004_0000;

/// Corresponds to the `ProcThreadAttributeValue` define in WinBase.h.
pub const fn proc_thread_attribute_value(
    number: u32,
    thread: bool,
    input: bool,
    additive: bool,
) -> u32 {
    (number & PROC_THREAD_ATTRIBUTE_NUMBER)
        | if thread {
            PROC_THREAD_ATTRIBUTE_THREAD
        } else {
            0
        }
        | if input {
            PROC_THREAD_ATTRIBUTE_INPUT
        } else {
            0
        }
        | if additive {
            PROC_THREAD_ATTRIBUTE_ADDITIVE
        } else {
            0
        }
}

/// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` (`windows.exp`).
pub const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: u32 = proc_thread_attribute_value(
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE_NUMBER,
    false,
    true,
    false,
);

/// `CREATE_UNICODE_ENVIRONMENT`.
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
/// `EXTENDED_STARTUPINFO_PRESENT`.
pub const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
/// `FILE_FLAG_FIRST_PIPE_INSTANCE`.
pub const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
/// `STATUS_PENDING` / `STILL_ACTIVE`.
pub const STILL_ACTIVE: u32 = 0x0000_0103;

/// UTF-8 to a NUL-terminated UTF-16 buffer (`utf8ToUtf16LeAllocZ`).
pub fn to_wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The NetBIOS name of this computer (`GetComputerNameA`), if available.
#[allow(unsafe_code)]
pub fn computer_name() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameA;

    let mut buf = [0u8; 257];
    let mut size = 256u32;
    // SAFETY: `buf` is valid for `size` + 1 bytes and `size` is a valid
    // in/out pointer.
    if unsafe { GetComputerNameA(buf.as_mut_ptr(), &raw mut size) } == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..size as usize]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudoconsole_attribute_value() {
        // Matches the PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE define (0x20016).
        assert_eq!(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, 0x0002_0016);
        assert_eq!(
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE
        );
    }

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(to_wide_nul("ab"), vec![b'a'.into(), b'b'.into(), 0]);
    }
}
