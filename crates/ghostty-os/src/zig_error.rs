//! `std::io::Error` -> Zig error name (`{}` of an error value, e.g.
//! `error.FileNotFound`), following Zig std's posix errno mapping for the
//! calls Ghostty makes (`openat`, `fstat`, `realpath`, `execve`, ...).

/// Zig error name (without the `error.` prefix) for a Linux errno.
#[must_use]
pub fn errno_name(errno: i32) -> &'static str {
    match errno {
        1 => "PermissionDenied",        // EPERM
        2 => "FileNotFound",            // ENOENT
        3 => "ProcessNotFound",         // ESRCH
        6 | 19 => "NoDevice",           // ENXIO, ENODEV
        11 => "WouldBlock",             // EAGAIN
        12 => "SystemResources",        // ENOMEM
        13 => "AccessDenied",           // EACCES
        16 => "DeviceBusy",             // EBUSY
        17 => "PathAlreadyExists",      // EEXIST
        20 => "NotDir",                 // ENOTDIR
        21 => "IsDir",                  // EISDIR
        23 => "SystemFdQuotaExceeded",  // ENFILE
        24 => "ProcessFdQuotaExceeded", // EMFILE
        26 => "FileBusy",               // ETXTBSY
        27 | 75 => "FileTooBig",        // EFBIG, EOVERFLOW
        28 => "NoSpaceLeft",            // ENOSPC
        30 => "ReadOnlyFileSystem",     // EROFS
        36 => "NameTooLong",            // ENAMETOOLONG
        40 => "SymLinkLoop",            // ELOOP
        84 => "InvalidWtf8",            // EILSEQ
        95 => "FileLocksNotSupported",  // EOPNOTSUPP
        _ => "Unexpected",
    }
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let n = |e| io_error_name(&std::io::Error::from_raw_os_error(e));
        assert_eq!(n(2), "error.FileNotFound");
        assert_eq!(n(20), "error.NotDir");
        assert_eq!(n(13), "error.AccessDenied");
        assert_eq!(n(21), "error.IsDir");
        assert_eq!(n(36), "error.NameTooLong");
        assert_eq!(n(40), "error.SymLinkLoop");
        assert_eq!(n(9999), "error.Unexpected");
    }
}
