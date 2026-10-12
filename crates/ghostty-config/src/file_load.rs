//! Default config file locations and opening (port of `config/file_load.zig`).
//!
//! On macOS the Application Support paths are used in addition to XDG; the
//! helpers that only need `open` + two candidate paths are platform
//! independent (and unit tested everywhere), the Application Support lookup
//! itself uses `NSFileManager` and is macOS-only.

use std::fs::File;

use ghostty_os::{osbytes::EncodedOs, xdg};

/// Default path for the XDG home configuration file.
pub fn default_xdg_path() -> Result<EncodedOs, xdg::Error> {
    xdg::config(&xdg::Options {
        subdir: Some("ghostty/config.ghostty"),
        home: None,
    })
}

/// Ghostty <1.3.0 default path for the XDG home configuration file.
pub fn legacy_default_xdg_path() -> Result<EncodedOs, xdg::Error> {
    xdg::config(&xdg::Options {
        subdir: Some("ghostty/config"),
        home: None,
    })
}

/// Preferred default path for the XDG home configuration file.
pub fn preferred_xdg_path() -> Result<EncodedOs, xdg::Error> {
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

/// Default path for the macOS Application Support configuration file.
#[cfg(target_os = "macos")]
pub fn default_app_support_path() -> Result<EncodedOs, xdg::Error> {
    app_support_dir("config.ghostty")
}

/// Ghostty <1.3.0 default path for the macOS Application Support
/// configuration file.
#[cfg(target_os = "macos")]
pub fn legacy_default_app_support_path() -> Result<EncodedOs, xdg::Error> {
    app_support_dir("config")
}

#[cfg(target_os = "macos")]
fn app_support_dir(sub_path: &str) -> Result<EncodedOs, xdg::Error> {
    ghostty_os::macos::app_support_dir(sub_path)
        .map(|p| EncodedOs::from_path(&p))
        .map_err(|_| xdg::Error::AppleApiFailed)
}

/// Preferred default path for the macOS Application Support configuration
/// file.
#[cfg(target_os = "macos")]
pub fn preferred_app_support_path() -> Result<EncodedOs, xdg::Error> {
    Ok(preferred_of(
        default_app_support_path()?,
        legacy_default_app_support_path()?,
    ))
}

/// Pick between a new path and its legacy path: the new one if it exists,
/// else the legacy one if it exists, else the new one.
#[cfg(any(target_os = "macos", test))]
fn preferred_of(new: EncodedOs, legacy: EncodedOs) -> EncodedOs {
    // If the new path exists, use that.
    if open(&new).is_ok() {
        return new;
    }

    // Try the legacy path
    if open(&legacy).is_ok() {
        return legacy;
    }

    // Legacy path and new path both don't exist. Return the new one.
    new
}

/// macOS: prefer the Application Support file if it exists, else the XDG
/// one if it exists, else Application Support.
#[cfg(any(target_os = "macos", test))]
fn preferred_macos(
    app_support_path: EncodedOs,
    xdg_path: &dyn Fn() -> Result<EncodedOs, xdg::Error>,
) -> Result<EncodedOs, xdg::Error> {
    // macOS prefers the Application Support directory if it exists.
    if open(&app_support_path).is_ok() {
        return Ok(app_support_path);
    }

    // Try the XDG path if it exists
    let xdg_path = xdg_path()?;
    if open(&xdg_path).is_ok() {
        return Ok(xdg_path);
    }

    // If neither file exists, use app support
    Ok(app_support_path)
}

/// Returns the path to the preferred default configuration file.
/// This is the file where users should place their configuration.
///
/// This doesn't create or populate the file with any default
/// contents; downstream callers must handle this.
#[cfg(target_os = "macos")]
pub fn preferred_default_file_path() -> Result<EncodedOs, xdg::Error> {
    preferred_macos(preferred_app_support_path()?, &preferred_xdg_path)
}

/// Returns the path to the preferred default configuration file.
/// This is the file where users should place their configuration.
///
/// This doesn't create or populate the file with any default
/// contents; downstream callers must handle this.
#[cfg(not(target_os = "macos"))]
pub fn preferred_default_file_path() -> Result<EncodedOs, xdg::Error> {
    // All other platforms use XDG only
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
pub fn open(path: &EncodedOs) -> Result<File, OpenFileError> {
    let real = path.to_path().map_err(|err| {
        log::warn!("invalid config path: {err}");
        OpenFileError::FileOpenFailed
    })?;
    assert!(path.is_zig_absolute());

    let file = File::open(&real).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            OpenFileError::FileNotFound
        } else {
            log::warn!(
                "unexpected file open error path={} err={}",
                real.display(),
                ghostty_os::zig_error::io_error_name(&err)
            );
            OpenFileError::FileOpenFailed
        }
    })?;

    let meta = file.metadata().map_err(|err| {
        log::warn!(
            "error getting file stat path={} err={}",
            real.display(),
            ghostty_os::zig_error::io_error_name(&err)
        );
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

    fn e(s: &str) -> EncodedOs {
        EncodedOs::from_encoded(s)
    }

    #[test]
    fn open_missing_dir_and_empty() {
        assert_eq!(
            open(&e("/nonexistent/ghostty/file")).unwrap_err(),
            OpenFileError::FileNotFound
        );
        assert_eq!(open(&e("/tmp")).unwrap_err(), OpenFileError::NotAFile);
        let p = std::env::temp_dir().join(format!("ghostty-file-load-{}", std::process::id()));
        std::fs::write(&p, "").unwrap();
        assert_eq!(
            open(&EncodedOs::from_path(&p)).unwrap_err(),
            OpenFileError::FileIsEmpty
        );
        std::fs::write(&p, "a=b").unwrap();
        assert!(open(&EncodedOs::from_path(&p)).is_ok());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn preferred_of_prefers_existing_new_then_legacy() {
        let dir = std::env::temp_dir().join(format!("ghostty-pref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let new = EncodedOs::from_path(&dir.join("config.ghostty"));
        let legacy = EncodedOs::from_path(&dir.join("config"));
        let _ = std::fs::remove_file(new.to_path().unwrap());
        let _ = std::fs::remove_file(legacy.to_path().unwrap());

        // Neither exists: the new one.
        assert_eq!(preferred_of(new.clone(), legacy.clone()), new);
        // Only legacy exists: the legacy one.
        std::fs::write(legacy.to_path().unwrap(), "a=b").unwrap();
        assert_eq!(preferred_of(new.clone(), legacy.clone()), legacy);
        // Both exist: the new one.
        std::fs::write(new.to_path().unwrap(), "a=b").unwrap();
        assert_eq!(preferred_of(new.clone(), legacy.clone()), new);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn preferred_macos_order() {
        let dir = std::env::temp_dir().join(format!("ghostty-pref-mac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = EncodedOs::from_path(&dir.join("app"));
        let xdg_p = EncodedOs::from_path(&dir.join("xdg"));
        let _ = std::fs::remove_file(app.to_path().unwrap());
        let _ = std::fs::remove_file(xdg_p.to_path().unwrap());
        let xdg_fn = || Ok(xdg_p.clone());

        // Neither exists: app support.
        assert_eq!(preferred_macos(app.clone(), &xdg_fn).unwrap(), app);
        // XDG exists only: XDG.
        std::fs::write(xdg_p.to_path().unwrap(), "a=b").unwrap();
        assert_eq!(preferred_macos(app.clone(), &xdg_fn).unwrap(), xdg_p);
        // App support exists: app support wins.
        std::fs::write(app.to_path().unwrap(), "a=b").unwrap();
        assert_eq!(preferred_macos(app.clone(), &xdg_fn).unwrap(), app);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_paths_end_with_expected_names() {
        if let (Ok(a), Ok(b)) = (default_xdg_path(), legacy_default_xdg_path()) {
            assert!(a.ends_with_plain("ghostty/config.ghostty"));
            assert!(b.ends_with_plain("ghostty/config"));
        }
    }
}
