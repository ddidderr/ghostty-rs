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
#[must_use]
#[cfg(windows)]
#[allow(non_upper_case_globals)] // windows-sys spells two constants `..._64k`
pub fn errno_name(code: i32) -> &'static str {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED,
        ERROR_AUTODATASEG_EXCEEDS_64k,
        ERROR_BAD_EXE_FORMAT,
        ERROR_BAD_FORMAT,
        ERROR_COMMITMENT_LIMIT,
        ERROR_DIRECTORY,
        ERROR_DISK_FULL,
        ERROR_DYNLINK_FROM_INVALID_RING,
        ERROR_EXE_MACHINE_TYPE_MISMATCH,
        ERROR_EXE_MARKED_INVALID,
        ERROR_FILE_NOT_FOUND,
        ERROR_FILENAME_EXCED_RANGE,
        ERROR_INFLOOP_IN_RELOC_CHAIN,
        ERROR_INVALID_EXE_SIGNATURE,
        ERROR_INVALID_MINALLOCSIZE,
        ERROR_INVALID_MODULETYPE,
        ERROR_INVALID_NAME,
        ERROR_INVALID_SEGDPL,
        ERROR_INVALID_STACKSEG,
        ERROR_INVALID_STARTING_CODESEG,
        ERROR_IOPL_NOT_ENABLED,
        ERROR_ITERATED_DATA_EXCEEDS_64k,
        ERROR_PATH_NOT_FOUND,
        ERROR_RELOC_CHAIN_XEEDS_SEGLIM,
        ERROR_RING2SEG_MUST_BE_MOVABLE,
        ERROR_SHARING_VIOLATION,
    };
    match code as u32 {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_DIRECTORY => "FileNotFound",
        ERROR_ACCESS_DENIED => "AccessDenied",
        ERROR_INVALID_NAME => "InvalidName",
        ERROR_FILENAME_EXCED_RANGE => "NameTooLong",
        ERROR_SHARING_VIOLATION => "FileBusy",
        ERROR_COMMITMENT_LIMIT => "SystemResources",
        ERROR_DISK_FULL => "NoSpaceLeft",
        ERROR_BAD_FORMAT
        | ERROR_INVALID_STARTING_CODESEG
        | ERROR_INVALID_STACKSEG
        | ERROR_INVALID_MODULETYPE
        | ERROR_INVALID_EXE_SIGNATURE
        | ERROR_EXE_MARKED_INVALID
        | ERROR_BAD_EXE_FORMAT
        | ERROR_ITERATED_DATA_EXCEEDS_64k
        | ERROR_INVALID_MINALLOCSIZE
        | ERROR_DYNLINK_FROM_INVALID_RING
        | ERROR_IOPL_NOT_ENABLED
        | ERROR_INVALID_SEGDPL
        | ERROR_AUTODATASEG_EXCEEDS_64k
        | ERROR_RING2SEG_MUST_BE_MOVABLE
        | ERROR_RELOC_CHAIN_XEEDS_SEGLIM
        | ERROR_INFLOOP_IN_RELOC_CHAIN
        | ERROR_EXE_MACHINE_TYPE_MISMATCH => "InvalidExe",
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

    #[cfg(windows)]
    #[test]
    fn windows_table() {
        for (code, name) in [
            (2, "FileNotFound"),
            (3, "FileNotFound"),
            (5, "AccessDenied"),
            (32, "FileBusy"),
            (112, "NoSpaceLeft"),
            (123, "InvalidName"),
            (193, "InvalidExe"),
            (206, "NameTooLong"),
            (267, "FileNotFound"),
            (1455, "SystemResources"),
            (87, "Unexpected"),
        ] {
            assert_eq!(errno_name(code), name);
        }
        let e = std::io::Error::from_raw_os_error(5);
        assert_eq!(io_error_name(&e), "error.AccessDenied");
    }
}
