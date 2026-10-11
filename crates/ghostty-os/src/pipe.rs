use std::io;
use std::os::fd::OwnedFd;

/// pipe() for POSIX. Sets CLOEXEC on the file descriptors.
/// Returns `(read, write)`.
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

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
