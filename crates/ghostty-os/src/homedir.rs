use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::process::{Command, Stdio};

use crate::osbytes;
#[cfg(unix)]
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
///
/// Non-UTF-8 bytes are preserved through [`crate::osbytes`]; use
/// [`home_os`] for the raw OS string.
pub fn home() -> Result<Option<String>, Error> {
    home_with_capacity(MAX_PATH_BYTES)
}

/// Byte-preserving [`home`].
pub fn home_os() -> Result<Option<OsString>, Error> {
    home_os_with_capacity(MAX_PATH_BYTES)
}

/// Like [`home`] but emulates the fixed output buffer of the Zig version:
/// errors with `BufferTooSmall` if the result is longer than `cap` bytes.
pub fn home_with_capacity(cap: usize) -> Result<Option<String>, Error> {
    Ok(home_os_with_capacity(cap)?.map(|h| osbytes::os_to_config_string(&h)))
}

/// Byte-preserving [`home_with_capacity`].
#[cfg(unix)]
pub fn home_os_with_capacity(cap: usize) -> Result<Option<OsString>, Error> {
    let check = |s: OsString| {
        if cap < osbytes::os_bytes(&s).len() {
            Err(Error::BufferTooSmall)
        } else {
            Ok(Some(s))
        }
    };

    // First: if we have a HOME env var, then we use that.
    if let Some(result) = std::env::var_os("HOME") {
        return check(result);
    }

    // On macOS: [NSFileManager defaultManager].homeDirectoryForCurrentUser.path
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSFileManager;

        let manager = NSFileManager::defaultManager();
        let home_url = manager.homeDirectoryForCurrentUser();
        if let Some(path) = home_url.path() {
            return check(OsString::from(path.to_string()));
        }
    }

    // We try passwd. This doesn't work on multi-user mac but we try it anyways.
    if let Ok(pw) = passwd::get_os()
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
        return check(osbytes::os_from_bytes(trim_space(&run.stdout).to_vec()));
    }

    Ok(None)
}

/// Windows: `%HOMEDRIVE%%HOMEPATH%` (`homeWindows`). Returns `None` if
/// either variable is missing.
#[cfg(windows)]
pub fn home_os_with_capacity(cap: usize) -> Result<Option<OsString>, Error> {
    let drive = std::env::var_os("HOMEDRIVE").map(|v| v.to_string_lossy().into_owned());
    let path = std::env::var_os("HOMEPATH").map(|v| v.to_string_lossy().into_owned());
    Ok(home_windows(drive.as_deref(), path.as_deref(), cap)?.map(OsString::from))
}

/// Pure part of [`home_with_capacity`] on Windows.
#[cfg(any(windows, test))]
fn home_windows(
    drive: Option<&str>,
    path: Option<&str>,
    cap: usize,
) -> Result<Option<String>, Error> {
    let (Some(drive), Some(path)) = (drive, path) else {
        return Ok(None);
    };
    if cap < drive.len() + path.len() {
        return Err(Error::BufferTooSmall);
    }
    Ok(Some(format!("{drive}{path}")))
}

#[cfg(unix)]
fn trim_space(input: &[u8]) -> &[u8] {
    let is_ws = |b: &u8| matches!(b, b' ' | b'\n' | b'\t');
    let start = input.iter().position(|b| !is_ws(b)).unwrap_or(input.len());
    let end = input
        .iter()
        .rposition(|b| !is_ws(b))
        .map_or(start, |i| i + 1);
    &input[start..end]
}

/// Expands a path that starts with a tilde (~) to the home directory of
/// the current user.
///
/// Errors if `home` fails or if the size of the expanded path is larger
/// than the maximum path length.
pub fn expand_home(path: &str) -> Result<String, ExpandError> {
    expand_home_with_capacity(path, MAX_PATH_BYTES)
}

/// Byte-preserving [`expand_home`].
pub fn expand_home_os(path: &OsStr) -> Result<OsString, ExpandError> {
    expand_home_os_with_capacity(path, MAX_PATH_BYTES)
}

/// Like [`expand_home`] with an explicit output capacity (emulating the
/// caller-provided buffer of the Zig version).
pub fn expand_home_with_capacity(path: &str, cap: usize) -> Result<String, ExpandError> {
    expand_home_os_with_capacity(&osbytes::config_str_to_os(path), cap)
        .map(|p| osbytes::os_to_config_string(&p))
}

/// Windows: `~/` is not an idiom generally used on Windows.
#[cfg(windows)]
pub fn expand_home_os_with_capacity(path: &OsStr, _cap: usize) -> Result<OsString, ExpandError> {
    Ok(path.to_owned())
}

/// See the Windows variant above.
#[cfg(unix)]
pub fn expand_home_os_with_capacity(path: &OsStr, cap: usize) -> Result<OsString, ExpandError> {
    let bytes = osbytes::os_bytes(path);
    if !bytes.starts_with(b"~/") {
        return Ok(path.to_owned());
    }
    let Ok(Some(home_dir)) = home_os_with_capacity(cap) else {
        return Err(ExpandError::HomeDetectionFailed);
    };
    let mut out = osbytes::os_bytes(&home_dir);
    let rest = &bytes[1..]; // Skip the ~
    if out.len() + rest.len() > cap {
        return Err(ExpandError::BufferTooSmall);
    }
    out.extend_from_slice(rest);
    Ok(osbytes::os_from_bytes(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_windows_joins_drive_and_path() {
        assert_eq!(
            home_windows(Some("C:"), Some("\\Users\\me"), 4096),
            Ok(Some("C:\\Users\\me".to_owned()))
        );
        assert_eq!(home_windows(None, Some("\\x"), 4096), Ok(None));
        assert_eq!(home_windows(Some("C:"), None, 4096), Ok(None));
        assert_eq!(
            home_windows(Some("C:"), Some("\\Users"), 3),
            Err(Error::BufferTooSmall)
        );
    }
}

#[cfg(all(test, unix))]
mod tests_unix {
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
    fn home_preserves_non_utf8_env() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let h = OsString::from_vec(b"/home/\xffu".to_vec());
        temp_env::with_var("HOME", Some(&h), || {
            assert_eq!(home_os().unwrap().unwrap(), h);
            let e = expand_home_os(OsStr::from_bytes(b"~/x\xfe")).unwrap();
            assert_eq!(e.as_bytes(), b"/home/\xffu/x\xfe");
            // The String API round-trips through the escape encoding.
            let s = expand_home("~/y").unwrap();
            assert_eq!(osbytes::config_str_to_bytes(&s), b"/home/\xffu/y");
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
