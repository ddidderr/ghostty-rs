//! `+crash-report` (ZIG/src/cli/crash_report.zig and the directory listing
//! of ZIG/src/crash/dir.zig).
//!
//! Ghostty uses sentry for crash reporting; only the listing behaviour is
//! ported.

use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use ghostty_config::types_b::Duration;
pub use ghostty_crash::Report;

use crate::error::Error;
pub use crate::options::NoOptions as Options;

/// The default crash directory (Zig: `crash.defaultDir`):
/// `$XDG_STATE_HOME/ghostty/crash`.
pub fn default_dir() -> Result<PathBuf, Error> {
    let dir = ghostty_crash::default_dir().map_err(|e| Error::Other(e.to_string()))?;
    Ok(dir.path)
}

/// List the crash reports (regular files) in `dir`, i.e. the
/// `.ghosttycrash` envelopes written by `ghostty-crash`. A directory that
/// cannot be opened yields no reports.
pub fn list_reports(dir: &std::path::Path) -> Result<Vec<Report>, Error> {
    Ok(ghostty_crash::dir::list_reports(dir)?)
}

/// The `crash-report` command is used to inspect and send crash reports.
///
/// When executed without any arguments, this will list existing crash reports.
///
/// This command currently only supports listing crash reports. Viewing
/// and sending crash reports is unimplemented and will be added in the future.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options;
    crate::parse_options(&mut opts, args)?;

    let stdout = std::io::stdout();
    let is_tty = stdout.is_terminal();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let result = run_inner(&default_dir()?, is_tty, now_ns(), &mut out);
    let _ = out.flush();
    result
}

fn now_ns() -> i128 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i128::try_from(d.as_nanos()).unwrap_or(i128::MAX),
        Err(_) => 0,
    }
}

fn run_inner(
    crash_dir: &std::path::Path,
    is_tty: bool,
    now: i128,
    stdout: &mut dyn Write,
) -> Result<u8, Error> {
    let mut reports = list_reports(crash_dir)?;

    // If we have no reports, then we're done. If we have a tty then we
    // print a message, otherwise we do nothing.
    if reports.is_empty() {
        if is_tty {
            stdout.write_all("No crash reports! \u{1F47B}\n".as_bytes())?;
        }
        return Ok(0);
    }

    // Newest first.
    reports.sort_by_key(|r| std::cmp::Reverse(r.mtime));

    for report in &reports {
        let diff = now - report.mtime;
        let since = if diff <= 0 {
            "now".to_string()
        } else {
            let d = Duration {
                duration: u64::try_from(diff).unwrap_or(u64::MAX),
            };
            format!("{} ago", d.round(1_000_000_000))
        };
        // Display only: the file name is shown as text.
        writeln!(stdout, "{} ({since})", report.name.to_string_lossy())?;
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn missing_dir_lists_nothing() {
        let mut out = Vec::new();
        let rc = run_inner(
            std::path::Path::new("/nonexistent/crash"),
            true,
            0,
            &mut out,
        )
        .unwrap();
        assert_eq!(0, rc);
        assert_eq!("No crash reports! \u{1F47B}\n".as_bytes(), out.as_slice());

        let mut out = Vec::new();
        run_inner(
            std::path::Path::new("/nonexistent/crash"),
            false,
            0,
            &mut out,
        )
        .unwrap();
        assert_eq!(0, out.len());
    }

    #[test]
    fn lists_newest_first() {
        let dir = std::env::temp_dir().join(format!("ghostty-cli-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("a.ghosttycrash"), "x").unwrap();
        std::fs::write(dir.join("b.ghosttycrash"), "y").unwrap();
        let old = std::fs::File::options()
            .write(true)
            .open(dir.join("a.ghosttycrash"))
            .unwrap();
        let a_time = SystemTime::now() - std::time::Duration::from_secs(1000);
        old.set_modified(a_time).unwrap();
        drop(old);
        let a_ns = i128::try_from(a_time.duration_since(UNIX_EPOCH).unwrap().as_nanos()).unwrap();

        let mut out = Vec::new();
        run_inner(&dir, false, a_ns + (3 * 3600 + 5) * 1_000_000_000, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(2, lines.len(), "{s}");
        assert!(lines[0].starts_with("b.ghosttycrash ("), "{s}");
        assert_eq!("a.ghosttycrash (3h 5s ago)", lines[1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
