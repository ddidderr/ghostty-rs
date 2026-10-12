#[cfg(unix)]
use std::{io, os::fd::OwnedFd};

/// pipe() for POSIX. Sets CLOEXEC on the file descriptors.
/// Returns `(read, write)`.
#[cfg(all(unix, not(target_vendor = "apple")))]
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(io::Error::from)
}

/// pipe() for Darwin, which has no `pipe2`: like Zig's `posix.pipe2`
/// fallback, create the pipe and then set CLOEXEC on both ends.
#[cfg(target_vendor = "apple")]
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (r, w) = rustix::pipe::pipe().map_err(io::Error::from)?;
    for fd in [&r, &w] {
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::CLOEXEC).map_err(io::Error::from)?;
    }
    Ok((r, w))
}

/// pipe() for Windows (`CreatePipe` with default security attributes, so the
/// handles are not inheritable). Returns `(read, write)`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn pipe() -> std::io::Result<(
    std::os::windows::io::OwnedHandle,
    std::os::windows::io::OwnedHandle,
)> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use windows_sys::Win32::{Foundation::HANDLE, System::Pipes::CreatePipe};

    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: both out pointers are valid for writes; null security
    // attributes and size 0 request the defaults.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, std::ptr::null(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreatePipe succeeded so both handles are valid and owned by us.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read.cast()),
            OwnedHandle::from_raw_handle(write.cast()),
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    #[test]
    fn pipe_roundtrip_cloexec() {
        let (r, w) = pipe().unwrap();
        let flags = rustix::io::fcntl_getfd(&r).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
        let mut w = std::fs::File::from(w);
        let mut r = std::fs::File::from(r);
        w.write_all(b"hi").unwrap();
        drop(w);
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hi");
    }
}

#[cfg(all(test, windows))]
mod tests_windows {
    use std::io::{Read, Write};

    use super::*;

    #[test]
    fn pipe_roundtrip() {
        let (r, w) = pipe().unwrap();
        let mut w = std::fs::File::from(w);
        let mut r = std::fs::File::from(r);
        w.write_all(b"hi").unwrap();
        drop(w);
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hi");
    }
}
