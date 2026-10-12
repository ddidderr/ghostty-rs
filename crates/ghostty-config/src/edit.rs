//! Opening the config for editing (port of `config/edit.zig`).

use std::{fs::OpenOptions, io};

use ghostty_os::osbytes::EncodedOs;

use crate::file_load;

/// The path to the configuration that should be opened for editing.
///
/// On Linux, this will use the file at the XDG config path. This is the
/// only valid path for Linux so we don't need to check for other paths.
///
/// On macOS, both XDG and AppSupport paths are valid; the AppSupport paths
/// come first.
///
/// The config file (and its directory) is created if it doesn't exist.
pub fn open_path() -> io::Result<EncodedOs> {
    // Get the path we should open
    let config_path = config_path()?;

    // The config path is an encoded OS string: decode before touching the
    // filesystem.
    let real_path = config_path.to_path()?;

    // Create config directory recursively.
    if let Some(config_dir) = real_path.parent() {
        std::fs::create_dir_all(config_dir)?;
    }

    // Try to create file and go on if it already exists
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&real_path)
    {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }

    Ok(config_path)
}

fn xdg_err(e: ghostty_os::xdg::Error) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, e)
}

/// Returns the config path to use for open for the current OS.
fn config_path() -> io::Result<EncodedOs> {
    let paths = config_path_candidates()?;

    // Find the first path that exists and is non-empty. If no paths are
    // non-empty but at least one exists, we will return the first path that
    // exists.
    let mut exists: Option<&EncodedOs> = None;
    for path in &paths {
        let meta = match path
            .to_path()
            .and_then(std::fs::File::open)
            .and_then(|f| f.metadata())
        {
            Ok(m) => m,
            // File doesn't exist, continue.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            // Some other error, assume it exists and return it.
            Err(e) => return Err(e),
        };

        // If the file is non-empty, return it.
        if meta.len() > 0 {
            return Ok(path.clone());
        }

        // If the file is empty, remember it exists.
        if exists.is_none() {
            exists = Some(path);
        }
    }

    // No paths are non-empty, return the first path that exists.
    if let Some(v) = exists {
        return Ok(v.clone());
    }

    // No paths are non-empty or exist, return the first path.
    Ok(paths[0].clone())
}

/// Returns a list of possible paths the main config file could be
/// in for the current OS.
fn config_path_candidates() -> io::Result<Vec<EncodedOs>> {
    let mut paths = Vec::with_capacity(4);

    #[cfg(target_os = "macos")]
    {
        paths.push(file_load::default_app_support_path().map_err(xdg_err)?);
        paths.push(file_load::legacy_default_app_support_path().map_err(xdg_err)?);
    }

    paths.push(file_load::default_xdg_path().map_err(xdg_err)?);
    paths.push(file_load::legacy_default_xdg_path().map_err(xdg_err)?);
    Ok(paths)
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use super::*;

    /// "Open config" must create/return the file under the *decoded* XDG
    /// directory, not a second file at the escaped pathname.
    #[test]
    fn open_path_uses_decoded_non_utf8_xdg_dir() {
        let td = ghostty_os::TempDir::init().unwrap();
        let xdg = td.path().join(std::ffi::OsStr::from_bytes(b"cfg\xff"));
        let value = std::ffi::OsString::from_vec(xdg.as_os_str().as_bytes().to_vec());
        temp_env::with_var("XDG_CONFIG_HOME", Some(&value), || {
            let p = open_path().unwrap();
            let real = p.to_path().unwrap();
            assert_eq!(real, xdg.join("ghostty/config.ghostty"));
            assert!(real.is_file());
            // Nothing was created at the escaped (encoded) pathname.
            assert_ne!(p.as_encoded().as_bytes(), real.as_os_str().as_bytes());
            assert!(!std::path::Path::new(p.as_encoded()).exists());
        });
    }

    #[test]
    fn candidates_are_xdg_paths_in_priority_order() {
        temp_env::with_var("XDG_CONFIG_HOME", Some("/x/cfg"), || {
            let c = config_path_candidates().unwrap();
            assert_eq!(c[0], "/x/cfg/ghostty/config.ghostty");
            assert_eq!(c[1], "/x/cfg/ghostty/config");
        });
    }
}
