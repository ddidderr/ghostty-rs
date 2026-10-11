use std::process::{Command, Stdio};

use crate::passwd;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The buffer used for output is not large enough to store the value.
    #[error("buffer too small")]
    BufferTooSmall,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExpandError {
    #[error("home detection failed")]
    HomeDetectionFailed,
    #[error("buffer too small")]
    BufferTooSmall,
}

/// Equivalent of `std.fs.max_path_bytes` on Linux.
const MAX_PATH_BYTES: usize = 4096;

/// Determine the home directory for the currently executing user. This
/// is generally an expensive process so the value should be cached.
pub fn home() -> Result<Option<String>, Error> {
    home_with_capacity(MAX_PATH_BYTES)
}

/// Like [`home`] but emulates the fixed output buffer of the Zig version:
/// errors with `BufferTooSmall` if the result is longer than `cap` bytes.
pub fn home_with_capacity(cap: usize) -> Result<Option<String>, Error> {
    let check = |s: String| {
        if cap < s.len() {
            Err(Error::BufferTooSmall)
        } else {
            Ok(Some(s))
        }
    };

    // First: if we have a HOME env var, then we use that.
    if let Some(result) = std::env::var_os("HOME") {
        return check(result.to_string_lossy().into_owned());
    }

    // We try passwd. This doesn't work on multi-user mac but we try it anyways.
    if let Ok(pw) = passwd::get()
        && let Some(result) = pw.home
    {
        return check(result);
    }

    // If all else fails, have the shell tell us...
    let run = Command::new("/bin/sh")
        .args(["-c", "cd && pwd"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Ok(run) = run
        && run.status.success()
    {
        let out = String::from_utf8_lossy(&run.stdout);
        return check(trim_space(&out).to_owned());
    }

    Ok(None)
}

fn trim_space(input: &str) -> &str {
    input.trim_matches([' ', '\n', '\t'])
}

/// Expands a path that starts with a tilde (~) to the home directory of
/// the current user.
///
/// Errors if `home` fails or if the size of the expanded path is larger
/// than the maximum path length.
pub fn expand_home(path: &str) -> Result<String, ExpandError> {
    expand_home_with_capacity(path, MAX_PATH_BYTES)
}

/// Like [`expand_home`] with an explicit output capacity (emulating the
/// caller-provided buffer of the Zig version).
pub fn expand_home_with_capacity(path: &str, cap: usize) -> Result<String, ExpandError> {
    if !path.starts_with("~/") {
        return Ok(path.to_owned());
    }
    let Ok(Some(home_dir)) = home_with_capacity(cap) else {
        return Err(ExpandError::HomeDetectionFailed);
    };
    let rest = &path[1..]; // Skip the ~
    let expanded_len = home_dir.len() + rest.len();

    if expanded_len > cap {
        return Err(ExpandError::BufferTooSmall);
    }

    Ok(format!("{home_dir}{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::with_env_lock;

    #[test]
    fn expand_home_unix() {
        with_env_lock(|| {
            let cap = MAX_PATH_BYTES;
            let home_dir = expand_home_with_capacity("~/", cap).unwrap();
            // Joining the home directory `~` with the path `/`
            // the result should end with a separator here. (e.g. `/home/user/`)
            assert!(home_dir.ends_with('/'));

            let downloads = expand_home_with_capacity("~/Downloads/shader.glsl", cap).unwrap();
            let expected_downloads = format!("{home_dir}Downloads/shader.glsl");
            assert_eq!(expected_downloads, downloads);

            assert_eq!("~", expand_home_with_capacity("~", cap).unwrap());
            assert_eq!("~abc/", expand_home_with_capacity("~abc/", cap).unwrap());
            assert_eq!(
                "/home/user",
                expand_home_with_capacity("/home/user", cap).unwrap()
            );
            assert_eq!("", expand_home_with_capacity("", cap).unwrap());

            // Expect an error if the buffer is large enough to hold the home directory,
            // but not the expanded path
            assert_eq!(
                Err(ExpandError::BufferTooSmall),
                expand_home_with_capacity("~/Downloads", home_dir.len())
            );
        });
    }

    #[test]
    fn home_is_detected() {
        with_env_lock(|| {
            let result = home().unwrap();
            assert!(result.is_some());
            assert!(!result.unwrap().is_empty());
        });
    }

    #[test]
    fn home_prefers_env() {
        temp_env::with_var("HOME", Some("/some/home"), || {
            assert_eq!(home().unwrap().as_deref(), Some("/some/home"));
            assert_eq!(expand_home("~/x").unwrap(), "/some/home/x");
            assert_eq!(home_with_capacity(3), Err(Error::BufferTooSmall));
        });
    }
}
