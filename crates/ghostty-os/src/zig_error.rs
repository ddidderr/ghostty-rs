//! `std::io::Error` -> Zig error name (`{}` of an error value, e.g.
//! `error.FileNotFound`), following Zig std's posix errno mapping for the
//! calls Ghostty makes (`openat`, `fstat`, `realpath`, `execve`, ...).

/// Zig error name (without the `error.` prefix) for a platform error code:
/// a POSIX `errno` on unix targets (values come from `libc`, so Linux and
/// Darwin/BSD numbering are both right) or a Win32 `GetLastError` code on
/// Windows (the `CreateProcessW` / file-open mapping of Zig std).
#[must_use]
#[cfg(unix)]
pub fn errno_name(errno: i32) -> &'static str {
    use libc::{
        EACCES,
        EAGAIN,
        EBUSY,
        EEXIST,
        EFBIG,
        EILSEQ,
        EISDIR,
        ELOOP,
        EMFILE,
        ENAMETOOLONG,
        ENFILE,
        ENODEV,
        ENOENT,
        ENOMEM,
        ENOSPC,
        ENOTDIR,
        ENXIO,
        EOPNOTSUPP,
        EOVERFLOW,
        EPERM,
        EROFS,
        ESRCH,
        ETXTBSY,
    };
    match errno {
        EPERM => "PermissionDenied",
        ENOENT => "FileNotFound",
        ESRCH => "ProcessNotFound",
        ENXIO | ENODEV => "NoDevice",
        EAGAIN => "WouldBlock",
        ENOMEM => "SystemResources",
        EACCES => "AccessDenied",
        EBUSY => "DeviceBusy",
        EEXIST => "PathAlreadyExists",
        ENOTDIR => "NotDir",
        EISDIR => "IsDir",
        ENFILE => "SystemFdQuotaExceeded",
        EMFILE => "ProcessFdQuotaExceeded",
        ETXTBSY => "FileBusy",
        EFBIG | EOVERFLOW => "FileTooBig",
        ENOSPC => "NoSpaceLeft",
        EROFS => "ReadOnlyFileSystem",
        ENAMETOOLONG => "NameTooLong",
        ELOOP => "SymLinkLoop",
        EILSEQ => "InvalidWtf8",
        EOPNOTSUPP => "FileLocksNotSupported",
        _ => "Unexpected",
    }
}

/// See the unix variant; this is the Windows (Win32 error code) table.
///
/// Zig's Windows file opens go through `std.os.windows.OpenFile`
/// (`NtCreateFile`), which switches on the NTSTATUS. Rust opens with
/// `CreateFileW`, whose error is `RtlNtStatusToDosError(status)`, so each arm
/// below is the Win32 code of the NTSTATUS arm named in its comment (codes
/// that `OpenFile` does not name are `error.Unexpected`):
///
/// | NTSTATUS | Win32 | Zig error |
/// |---|---|---|
/// | `OBJECT_NAME_INVALID` | `ERROR_INVALID_NAME` | `BadPathName` |
/// | `OBJECT_NAME_NOT_FOUND` / `OBJECT_PATH_NOT_FOUND` | `ERROR_FILE_NOT_FOUND` / `ERROR_PATH_NOT_FOUND` | `FileNotFound` |
/// | `BAD_NETWORK_PATH` / `BAD_NETWORK_NAME` | `ERROR_BAD_NETPATH` / `ERROR_BAD_NET_NAME` | `NetworkNotFound` |
/// | `NO_MEDIA_IN_DEVICE` | `ERROR_NOT_READY` | `NoDevice` |
/// | `SHARING_VIOLATION`, `ACCESS_DENIED`, `USER_MAPPED_FILE`, `FILE_IS_A_DIRECTORY` | `ERROR_SHARING_VIOLATION`, `ERROR_ACCESS_DENIED`, `ERROR_USER_MAPPED_FILE` | `AccessDenied` |
/// | `PIPE_BUSY` / `PIPE_NOT_AVAILABLE` | `ERROR_PIPE_BUSY` | `PipeBusy` |
/// | `OBJECT_NAME_COLLISION` | `ERROR_ALREADY_EXISTS` | `PathAlreadyExists` |
/// | `NOT_A_DIRECTORY` | `ERROR_DIRECTORY` | `NotDir` |
///
/// plus `wToPrefixedFileW`'s `NameTooLong` (`ERROR_FILENAME_EXCED_RANGE`).
#[must_use]
#[cfg(windows)]
pub fn errno_name(code: i32) -> &'static str {
    windows_errno_name(code as u32)
}

/// The Windows table on plain numbers (so it is testable on every host).
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_errno_name(code: u32) -> &'static str {
    // Win32 error codes (winerror.h). Mirrors the NTSTATUS mapping of
    // `std.os.windows.OpenFile` in Zig 0.15.2 (verified against its source).
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const ERROR_PATH_NOT_FOUND: u32 = 3;
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_NOT_READY: u32 = 21;
    const ERROR_SHARING_VIOLATION: u32 = 32;
    const ERROR_BAD_NETPATH: u32 = 53;
    const ERROR_BAD_NET_NAME: u32 = 67;
    const ERROR_FILE_EXISTS: u32 = 80;
    const ERROR_INVALID_NAME: u32 = 123;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
    const ERROR_VIRUS_INFECTED: u32 = 225;
    const ERROR_VIRUS_DELETED: u32 = 226;
    const ERROR_PIPE_BUSY: u32 = 231;
    const ERROR_DIRECTORY: u32 = 267;
    const ERROR_USER_MAPPED_FILE: u32 = 1224;
    match code {
        ERROR_INVALID_NAME => "BadPathName",
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => "FileNotFound",
        ERROR_BAD_NETPATH | ERROR_BAD_NET_NAME => "NetworkNotFound",
        ERROR_NOT_READY => "NoDevice",
        // A sharing violation is `AccessDenied` in `OpenFile` (the
        // running-executable `FileBusy` retry logic is only in Zig 0.16).
        ERROR_SHARING_VIOLATION | ERROR_ACCESS_DENIED | ERROR_USER_MAPPED_FILE => "AccessDenied",
        ERROR_PIPE_BUSY => "PipeBusy",
        // STATUS_OBJECT_NAME_COLLISION surfaces as either code.
        ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS => "PathAlreadyExists",
        ERROR_VIRUS_INFECTED | ERROR_VIRUS_DELETED => "AntivirusInterference",
        ERROR_DIRECTORY => "NotDir",
        ERROR_FILENAME_EXCED_RANGE => "NameTooLong",
        _ => "Unexpected",
    }
}

/// Targets without a native error-code table (wasm).
#[must_use]
#[cfg(not(any(unix, windows)))]
pub fn errno_name(_code: i32) -> &'static str {
    "Unexpected"
}

/// `error.Name` text for an I/O error.
#[must_use]
pub fn io_error_name(err: &std::io::Error) -> String {
    if let Some(errno) = err.raw_os_error() {
        return format!("error.{}", errno_name(errno));
    }
    match err.kind() {
        std::io::ErrorKind::NotFound => "error.FileNotFound".into(),
        std::io::ErrorKind::PermissionDenied => "error.AccessDenied".into(),
        std::io::ErrorKind::InvalidInput => "error.BadPathName".into(),
        _ => "error.Unexpected".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn n(e: i32) -> String {
        io_error_name(&std::io::Error::from_raw_os_error(e))
    }

    #[cfg(unix)]
    #[test]
    fn libc_constants_map() {
        assert_eq!(n(libc::ENOENT), "error.FileNotFound");
        assert_eq!(n(libc::ENOTDIR), "error.NotDir");
        assert_eq!(n(libc::EACCES), "error.AccessDenied");
        assert_eq!(n(libc::EISDIR), "error.IsDir");
        assert_eq!(n(libc::ENAMETOOLONG), "error.NameTooLong");
        assert_eq!(n(libc::ELOOP), "error.SymLinkLoop");
        assert_eq!(n(libc::EAGAIN), "error.WouldBlock");
        assert_eq!(n(libc::EOPNOTSUPP), "error.FileLocksNotSupported");
        assert_eq!(n(9999), "error.Unexpected");
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn linux_table() {
        for (code, name) in [
            (1, "PermissionDenied"),
            (2, "FileNotFound"),
            (11, "WouldBlock"),
            (13, "AccessDenied"),
            (20, "NotDir"),
            (21, "IsDir"),
            (36, "NameTooLong"),
            (40, "SymLinkLoop"),
            (75, "FileTooBig"),
            (84, "InvalidWtf8"),
            (95, "FileLocksNotSupported"),
        ] {
            assert_eq!(errno_name(code), name);
        }
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn darwin_table() {
        for (code, name) in [
            (1, "PermissionDenied"),
            (2, "FileNotFound"),
            (13, "AccessDenied"),
            (20, "NotDir"),
            (21, "IsDir"),
            (35, "WouldBlock"),
            (62, "SymLinkLoop"),
            (63, "NameTooLong"),
            (84, "FileTooBig"),
            (92, "InvalidWtf8"),
            (102, "FileLocksNotSupported"),
        ] {
            assert_eq!(errno_name(code), name);
        }
        // Linux-only numbers must not leak through.
        assert_eq!(errno_name(36), "Unexpected");
        assert_eq!(errno_name(40), "Unexpected");
    }

    #[test]
    fn windows_table() {
        for (code, name) in [
            (2u32, "FileNotFound"),
            (3, "FileNotFound"),
            (5, "AccessDenied"),
            // ERROR_SHARING_VIOLATION: AccessDenied like OpenFile.
            (32, "AccessDenied"),
            (1224, "AccessDenied"),
            (21, "NoDevice"),
            (53, "NetworkNotFound"),
            (67, "NetworkNotFound"),
            (123, "BadPathName"),
            (183, "PathAlreadyExists"),
            (80, "PathAlreadyExists"),
            (225, "AntivirusInterference"),
            (226, "AntivirusInterference"),
            (206, "NameTooLong"),
            (231, "PipeBusy"),
            (267, "NotDir"),
            (87, "Unexpected"),
            (112, "Unexpected"),
            (193, "Unexpected"),
        ] {
            assert_eq!(windows_errno_name(code), name, "code {code}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_io_error_name() {
        let e = std::io::Error::from_raw_os_error(32);
        assert_eq!(io_error_name(&e), "error.AccessDenied");
    }
}
