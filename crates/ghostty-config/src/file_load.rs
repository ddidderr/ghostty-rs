//! Default config file locations and opening (port of `config/file_load.zig`).
//!
//! macOS Application Support paths are not supported (Linux only).

use std::fs::File;

use ghostty_os::xdg;

/// Default path for the XDG home configuration file.
pub fn default_xdg_path() -> Result<String, xdg::Error> {
    xdg::config(&xdg::Options {
        subdir: Some("ghostty/config.ghostty"),
        home: None,
    })
}

/// Ghostty <1.3.0 default path for the XDG home configuration file.
pub fn legacy_default_xdg_path() -> Result<String, xdg::Error> {
    xdg::config(&xdg::Options {
        subdir: Some("ghostty/config"),
        home: None,
    })
}

/// Preferred default path for the XDG home configuration file.
pub fn preferred_xdg_path() -> Result<String, xdg::Error> {
    // If the XDG path exists, use that.
    let xdg_path = default_xdg_path()?;
    if open(&xdg_path).is_ok() {
        return Ok(xdg_path);
    }

    // Try the legacy path
    let legacy_xdg_path = legacy_default_xdg_path()?;
    if open(&legacy_xdg_path).is_ok() {
        return Ok(legacy_xdg_path);
    }

    // Legacy path and XDG path both don't exist. Return the new one.
    Ok(xdg_path)
}

/// Returns the path to the preferred default configuration file.
/// This is the file where users should place their configuration.
///
/// This doesn't create or populate the file with any default
/// contents; downstream callers must handle this.
pub fn preferred_default_file_path() -> Result<String, xdg::Error> {
    // All supported platforms use XDG only
    preferred_xdg_path()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFileError {
    FileNotFound,
    FileIsEmpty,
    FileOpenFailed,
    NotAFile,
}

impl std::fmt::Display for OpenFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OpenFileError::FileNotFound => "FileNotFound",
            OpenFileError::FileIsEmpty => "FileIsEmpty",
            OpenFileError::FileOpenFailed => "FileOpenFailed",
            OpenFileError::NotAFile => "NotAFile",
        })
    }
}

impl std::error::Error for OpenFileError {}

/// Opens the file at the given path and returns the file handle
/// if it exists and is non-empty. This also constrains the possible
/// errors to a smaller set that we can explicitly handle.
pub fn open(path: &str) -> Result<File, OpenFileError> {
    assert!(std::path::Path::new(path).is_absolute());

    let file = File::open(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            OpenFileError::FileNotFound
        } else {
            log::warn!("unexpected file open error path={path} err={err}");
            OpenFileError::FileOpenFailed
        }
    })?;

    let meta = file.metadata().map_err(|err| {
        log::warn!("error getting file stat path={path} err={err}");
        OpenFileError::FileOpenFailed
    })?;
    if !meta.is_file() {
        return Err(OpenFileError::NotAFile);
    }
    if meta.len() == 0 {
        return Err(OpenFileError::FileIsEmpty);
    }

    Ok(file)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn open_missing_dir_and_empty() {
        assert_eq!(
            open("/nonexistent/ghostty/file").unwrap_err(),
            OpenFileError::FileNotFound
        );
        assert_eq!(open("/tmp").unwrap_err(), OpenFileError::NotAFile);
        let p = std::env::temp_dir().join(format!("ghostty-file-load-{}", std::process::id()));
        std::fs::write(&p, "").unwrap();
        assert_eq!(
            open(p.to_str().unwrap()).unwrap_err(),
            OpenFileError::FileIsEmpty
        );
        std::fs::write(&p, "a=b").unwrap();
        assert!(open(p.to_str().unwrap()).is_ok());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn default_paths_end_with_expected_names() {
        if let (Ok(a), Ok(b)) = (default_xdg_path(), legacy_default_xdg_path()) {
            assert!(a.ends_with("ghostty/config.ghostty"));
            assert!(b.ends_with("ghostty/config"));
        }
    }
}
