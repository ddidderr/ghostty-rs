//! Creates a temporary directory at runtime that can be safely used to
//! store temporary data and is destroyed on drop.

use std::{
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

use crate::file::alloc_tmp_dir;

// The amount of random bytes to get to determine our filename.
const RANDOM_BYTES: usize = 16;

// Base64 alphabet, replacing the standard `+/` with `-_` so that it can
// be used in a file name on any filesystem.
const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Unpadded base64 using [`B64_ALPHABET`].
fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let n = match chunk.len() {
            3 => u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8 | u32::from(chunk[2]),
            2 => u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8,
            _ => u32::from(chunk[0]) << 16,
        };
        let chars = chunk.len() + 1;
        for i in 0..chars {
            let idx = (n >> (18 - 6 * i)) & 0x3f;
            out.push(B64_ALPHABET[idx as usize] as char);
        }
    }
    out
}

#[derive(Debug)]
pub struct TempDir {
    /// Parent directory
    parent: PathBuf,

    /// Name of the directory (basename only).
    name: String,
}

impl TempDir {
    /// Create the temporary directory.
    pub fn init() -> io::Result<TempDir> {
        let parent = match alloc_tmp_dir() {
            Some(p) => PathBuf::from(p),
            None => std::env::current_dir()?,
        };
        // Verify the parent can be opened (Zig opens it as a Dir).
        std::fs::read_dir(&parent)?;

        // We now loop forever until we can find a directory that we can create.
        loop {
            let mut rand_buf = [0u8; RANDOM_BYTES];
            rustix::rand::getrandom(&mut rand_buf, rustix::rand::GetRandomFlags::empty())?;
            let name = b64_encode(&rand_buf);

            match std::fs::DirBuilder::new()
                .mode(0o755)
                .create(parent.join(&name))
            {
                Ok(()) => return Ok(TempDir { parent, name }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Name returns the name of the directory. This is just the basename
    /// and is not the full absolute path.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The parent directory containing the temporary directory.
    pub fn parent(&self) -> &Path {
        &self.parent
    }

    /// Full path to the temporary directory.
    pub fn path(&self) -> PathBuf {
        self.parent.join(&self.name)
    }

    /// Finish with the temporary directory. This deletes all contents in the
    /// directory.
    pub fn deinit(self) {}
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_dir_all(self.path()) {
            log::error!("error deleting temp dir err={err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_lifecycle() {
        let td = TempDir::init().unwrap();
        let name = td.name().to_owned();
        assert!(!name.is_empty());
        assert_eq!(name.len(), 22);

        // Can open a new handle to it proves it exists.
        let parent = td.parent().to_owned();
        assert!(std::fs::read_dir(parent.join(&name)).is_ok());
        std::fs::write(td.path().join("file"), b"x").unwrap();

        // Should be deleted after we deinit
        td.deinit();
        let err = std::fs::read_dir(parent.join(&name)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn b64_encoder() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg");
        assert_eq!(b64_encode(b"fo"), "Zm8");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(b64_encode(&[0u8; 16]).len(), 22);
    }
}
