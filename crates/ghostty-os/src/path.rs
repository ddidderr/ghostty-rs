use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// `std.fs.path.delimiter` / `sep`.
#[cfg(windows)]
const DELIMITER: char = ';';
#[cfg(not(windows))]
const DELIMITER: char = ':';
#[cfg(windows)]
const SEP: char = '\\';
#[cfg(not(windows))]
const SEP: char = '/';

/// Equivalent of `std.fs.max_path_bytes` on Linux.
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("path too long")]
    PathTooLong,
    #[error("access denied")]
    AccessDenied,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Search for "cmd" in the PATH and return the absolute path.
pub fn expand(cmd: &str) -> Result<Option<String>, Error> {
    // If the command already contains a slash, then we return it as-is
    // because it is assumed to be absolute or relative.
    if cmd.contains('/') {
        return Ok(Some(cmd.to_owned()));
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    let path_var = path_var.to_string_lossy();

    let mut seen_eacces = false;
    for search_path in path_var.split(DELIMITER).filter(|s| !s.is_empty()) {
        // We need enough space in our path buffer to store this
        let path_len = search_path.len() + cmd.len() + 1;
        if MAX_PATH_BYTES < path_len {
            return Err(Error::PathTooLong);
        }

        let full_path = format!("{search_path}{SEP}{cmd}");

        // Stat it
        let f = match std::fs::File::open(&full_path) {
            Ok(f) => f,
            Err(e) => match e.kind() {
                io::ErrorKind::NotFound => continue,
                io::ErrorKind::PermissionDenied => {
                    // Accumulate this and return it later so we can try other
                    // paths that we have access to.
                    seen_eacces = true;
                    continue;
                }
                _ => return Err(e.into()),
            },
        };
        let meta = f.metadata()?;
        if !meta.is_dir() && is_executable(&meta) {
            return Ok(Some(full_path));
        }
    }

    if seen_eacces {
        return Err(Error::AccessDenied);
    }

    Ok(None)
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    meta.permissions().mode() & 0o0111 != 0
}

#[cfg(windows)]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // `uname -n` is the *nix equivalent of `hostname.exe` on Windows
    #[test]
    fn expand_hostname() {
        let executable = if cfg!(windows) {
            "hostname.exe"
        } else {
            "uname"
        };
        let path = crate::testutil::with_env_lock(|| expand(executable).unwrap().unwrap());
        assert!(path.len() > executable.len());
    }

    #[test]
    fn expand_does_not_exist() {
        let path =
            crate::testutil::with_env_lock(|| expand("thisreallyprobablydoesntexist123").unwrap());
        assert!(path.is_none());
    }

    #[test]
    fn expand_slash() {
        let path = expand("foo/env").unwrap().unwrap();
        assert_eq!(path.len(), 7);
    }

    #[cfg(unix)]
    #[test]
    fn expand_skips_non_executable_and_dirs() {
        let dir = crate::TempDir::init().unwrap();
        std::fs::write(dir.path().join("plain"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("adir")).unwrap();
        let p = dir.path();
        let p = p.to_str().unwrap();
        temp_env::with_var("PATH", Some(p), || {
            assert!(expand("plain").unwrap().is_none());
            assert!(expand("adir").unwrap().is_none());
        });
    }
}
