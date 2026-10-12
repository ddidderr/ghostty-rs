//! Crash report directory handling (port of `crash/dir.zig`).

use std::{
    fs,
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// The file extension of crash reports written by the handler.
pub const REPORT_EXTENSION: &str = "ghosttycrash";

/// Returns a [`Dir`] for the default directory
/// (`$XDG_STATE_HOME/ghostty/crash`).
pub fn default_dir() -> io::Result<Dir> {
    let path = ghostty_os::xdg::state(&ghostty_os::xdg::Options {
        subdir: Some("ghostty/crash"),
        home: None,
    })
    .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Dir {
        path: PathBuf::from(path),
    })
}

/// A directory where crash reports are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dir {
    pub path: PathBuf,
}

impl Dir {
    /// Returns an iterator over the crash reports in this directory. The
    /// iterator may have no reports (including when the directory can't be
    /// opened).
    #[must_use]
    pub fn iterator(&self) -> ReportIterator {
        ReportIterator {
            dir: self.path.clone(),
            it: fs::read_dir(&self.path).ok(),
        }
    }
}

/// Iterates the regular files of a crash directory.
pub struct ReportIterator {
    dir: PathBuf,
    it: Option<fs::ReadDir>,
}

impl ReportIterator {
    /// Next report, or `Ok(None)` at the end.
    pub fn next_report(&mut self) -> io::Result<Option<Report>> {
        let Some(it) = self.it.as_mut() else {
            return Ok(None);
        };
        loop {
            let Some(entry) = it.next() else {
                return Ok(None);
            };
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // statFile follows symlinks; entries here are regular files.
            let mtime = fs::metadata(self.dir.join(entry.file_name()))?.modified()?;
            let mtime = match mtime.duration_since(UNIX_EPOCH) {
                Ok(d) => i128::try_from(d.as_nanos()).unwrap_or(i128::MAX),
                Err(e) => -i128::try_from(e.duration().as_nanos()).unwrap_or(i128::MAX),
            };
            return Ok(Some(Report { name, mtime }));
        }
    }
}

impl Iterator for ReportIterator {
    type Item = io::Result<Report>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_report().transpose()
    }
}

/// A crash report file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub name: String,
    /// Modification time in nanoseconds since the unix epoch.
    pub mtime: i128,
}

/// Convenience used by the CLI: all reports of `dir`.
pub fn list_reports(dir: &Path) -> io::Result<Vec<Report>> {
    Dir {
        path: dir.to_path_buf(),
    }
    .iterator()
    .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn missing_dir_is_empty() {
        let d = Dir {
            path: "/nonexistent/ghostty/crash".into(),
        };
        assert!(d.iterator().next().is_none());
    }

    #[test]
    fn lists_only_files() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a.ghosttycrash"), "x").unwrap();
        fs::create_dir(t.path().join("sub")).unwrap();
        let r = list_reports(t.path()).unwrap();
        assert_eq!(1, r.len());
        assert_eq!("a.ghosttycrash", r[0].name);
    }

    #[test]
    fn default_dir_uses_xdg_state() {
        let d = default_dir().unwrap();
        assert!(d.path.ends_with("ghostty/crash"));
    }
}
