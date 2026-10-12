//! Opening the config for editing (port of `config/edit.zig`).

use std::{fs::OpenOptions, io, path::Path};

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
pub fn open_path() -> io::Result<String> {
    // Get the path we should open
    let config_path = config_path()?;

    // Create config directory recursively.
    if let Some(config_dir) = Path::new(&config_path).parent() {
        std::fs::create_dir_all(config_dir)?;
    }

    // Try to create file and go on if it already exists
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
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
fn config_path() -> io::Result<String> {
    let paths = config_path_candidates()?;

    // Find the first path that exists and is non-empty. If no paths are
    // non-empty but at least one exists, we will return the first path that
    // exists.
    let mut exists: Option<&String> = None;
    for path in &paths {
        let meta = match std::fs::File::open(path).and_then(|f| f.metadata()) {
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
fn config_path_candidates() -> io::Result<Vec<String>> {
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
