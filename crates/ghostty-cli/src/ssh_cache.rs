//! `+ssh-cache` (ZIG/src/cli/ssh_cache.zig).

use std::io::Write;

use ghostty_config::config_struct;

use crate::error::Error;

pub mod disk_cache;
pub mod entry;

pub use disk_cache::DiskCache;
pub use entry::Entry;

config_struct! {
    pub struct Options {
        "clear" => clear: bool = false,
        "add" => add: Option<String> = None,
        "remove" => remove: Option<String> = None,
        "host" => host: Option<String> = None,
        "expire-days" => expire_days: Option<u32> = None,
    }
}
cli_options!(Options, help);

/// Manage the SSH terminfo cache for automatic remote host setup.
///
/// When SSH integration is enabled with `shell-integration-features = ssh-terminfo`,
/// Ghostty automatically installs its terminfo on remote hosts. This command
/// manages the cache of successful installations to avoid redundant uploads.
///
/// The cache stores hostnames (or user@hostname combinations) along with timestamps.
/// Entries older than the expiration period are automatically removed during cache
/// operations. By default, entries never expire.
///
/// Only one of `--clear`, `--add`, `--remove`, or `--host` can be specified.
/// If multiple are specified, one of the actions will be executed but
/// it isn't guaranteed which one. This is entirely unsafe so you should split
/// multiple actions into separate commands.
///
/// Examples:
///   ghostty +ssh-cache                          # List all cached hosts
///   ghostty +ssh-cache --host=example.com       # Check if host is cached
///   ghostty +ssh-cache --add=example.com        # Manually add host to cache
///   ghostty +ssh-cache --add=user@example.com   # Add user@host combination
///   ghostty +ssh-cache --remove=example.com     # Remove host from cache
///   ghostty +ssh-cache --clear                  # Clear entire cache
///   ghostty +ssh-cache --expire-days=30         # Set custom expiration period
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    let mut stderr = std::io::BufWriter::new(std::io::stderr().lock());

    let result = run_inner(&opts, &mut stdout, &mut stderr);

    // Flushing *shouldn't* fail but...
    let _ = stdout.flush();
    let _ = stderr.flush();
    result
}

/// Like Zig's `runInner` with the cache at the standard location.
pub fn run_inner(
    opts: &Options,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, Error> {
    // Setup our disk cache to the standard location
    let cache_path = DiskCache::default_path("ghostty").map_err(|e| Error::Other(e.to_string()))?;
    run_with_cache(&DiskCache { path: cache_path }, opts, stdout, stderr)
}

fn invalid_hostname(host: &str, stderr: &mut dyn Write) -> Result<u8, Error> {
    writeln!(stderr, "Error: Invalid hostname format '{host}'")?;
    writeln!(stderr, "Expected format: hostname or user@hostname")?;
    Ok(1)
}

/// Run the action against an explicit cache.
pub fn run_with_cache(
    cache: &DiskCache,
    opts: &Options,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, Error> {
    use disk_cache::{AddResult, Error as CacheError};

    if opts.clear {
        cache.clear().map_err(|e| Error::Other(e.to_string()))?;
        writeln!(stdout, "Cache cleared.")?;
        return Ok(0);
    }

    if let Some(host) = &opts.add {
        let result = match cache.add(host) {
            Ok(r) => r,
            Err(CacheError::HostnameIsInvalid) => return invalid_hostname(host, stderr),
            Err(CacheError::CacheIsLocked) => {
                writeln!(stderr, "Error: Cache is busy, try again")?;
                return Ok(1);
            }
            Err(err) => {
                writeln!(
                    stderr,
                    "Error: Unable to add '{host}' to cache. Error: {err}"
                )?;
                return Ok(1);
            }
        };

        match result {
            AddResult::Added => writeln!(stdout, "Added '{host}' to cache.")?,
            AddResult::Updated => writeln!(stdout, "Updated '{host}' cache entry.")?,
        }
        return Ok(0);
    }

    if let Some(host) = &opts.remove {
        match cache.remove(host) {
            Ok(()) => {}
            Err(CacheError::HostnameIsInvalid) => return invalid_hostname(host, stderr),
            Err(CacheError::CacheIsLocked) => {
                writeln!(stderr, "Error: Cache is busy, try again")?;
                return Ok(1);
            }
            Err(err) => {
                writeln!(
                    stderr,
                    "Error: Unable to remove '{host}' from cache. Error: {err}"
                )?;
                return Ok(1);
            }
        }
        writeln!(stdout, "Removed '{host}' from cache.")?;
        return Ok(0);
    }

    if let Some(host) = &opts.host {
        let cached = match cache.contains(host) {
            Ok(c) => c,
            Err(CacheError::HostnameIsInvalid) => return invalid_hostname(host, stderr),
            Err(err) => {
                writeln!(
                    stderr,
                    "Error: Unable to check host '{host}' in cache. Error: {err}"
                )?;
                return Ok(1);
            }
        };

        if cached {
            writeln!(stdout, "'{host}' has Ghostty terminfo installed.")?;
            return Ok(0);
        }
        writeln!(stdout, "'{host}' does not have Ghostty terminfo installed.")?;
        return Ok(1);
    }

    // Default action: list all hosts
    let entries = cache.list().map_err(|e| Error::Other(e.to_string()))?;
    list_entries(&entries, entry::now_timestamp(), stdout)?;
    Ok(0)
}

fn list_entries(
    entries: &disk_cache::Entries,
    now: i64,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    if entries.is_empty() {
        writeln!(writer, "No hosts in cache.")?;
        return Ok(());
    }

    // Sort entries by hostname for consistent output
    let mut items: Vec<&Entry> = entries.values().collect();
    items.sort_by(|a, b| a.hostname.as_bytes().cmp(b.hostname.as_bytes()));

    writeln!(writer, "Cached hosts ({}):", items.len())?;

    for entry in items {
        let age_days = (now - entry.timestamp) / 86_400;
        if age_days == 0 {
            writeln!(writer, "  {} (today)", entry.hostname)?;
        } else if age_days == 1 {
            writeln!(writer, "  {} (yesterday)", entry.hostname)?;
        } else {
            writeln!(writer, "  {} ({age_days} days ago)", entry.hostname)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn run_cache(cache: &DiskCache, opts: &Options) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let rc = run_with_cache(cache, opts, &mut out, &mut err).unwrap();
        (
            rc,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn opt_add(h: &str) -> Options {
        Options {
            add: Some(h.into()),
            ..Options::default()
        }
    }

    #[test]
    fn add_list_host_remove_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            path: tmp.path().join("ssh_cache"),
        };

        let (rc, out, _) = run_cache(&cache, &Options::default());
        assert_eq!((0, "No hosts in cache.\n"), (rc, out.as_str()));

        let (rc, out, _) = run_cache(&cache, &opt_add("b.com"));
        assert_eq!((0, "Added 'b.com' to cache.\n"), (rc, out.as_str()));
        let (rc, out, _) = run_cache(&cache, &opt_add("b.com"));
        assert_eq!((0, "Updated 'b.com' cache entry.\n"), (rc, out.as_str()));
        run_cache(&cache, &opt_add("a.com"));

        let (rc, out, _) = run_cache(&cache, &Options::default());
        assert_eq!(0, rc);
        assert_eq!("Cached hosts (2):\n  a.com (today)\n  b.com (today)\n", out);

        let host = |h: &str| Options {
            host: Some(h.into()),
            ..Options::default()
        };
        let (rc, out, _) = run_cache(&cache, &host("a.com"));
        assert_eq!(
            (0, "'a.com' has Ghostty terminfo installed.\n"),
            (rc, out.as_str())
        );
        let (rc, out, _) = run_cache(&cache, &host("zzz.com"));
        assert_eq!(
            (1, "'zzz.com' does not have Ghostty terminfo installed.\n"),
            (rc, out.as_str())
        );

        let (rc, out, _) = run_cache(
            &cache,
            &Options {
                remove: Some("a.com".into()),
                ..Options::default()
            },
        );
        assert_eq!((0, "Removed 'a.com' from cache.\n"), (rc, out.as_str()));

        let (rc, out, _) = run_cache(
            &cache,
            &Options {
                clear: true,
                ..Options::default()
            },
        );
        assert_eq!((0, "Cache cleared.\n"), (rc, out.as_str()));
        assert!(!cache.path.exists());
    }

    #[test]
    fn invalid_hostname_message() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            path: tmp.path().join("ssh_cache"),
        };
        let (rc, out, err) = run_cache(&cache, &opt_add("bad host"));
        assert_eq!(1, rc);
        assert_eq!(0, out.len());
        assert_eq!(
            "Error: Invalid hostname format 'bad host'\nExpected format: hostname or user@hostname\n",
            err
        );
    }

    #[test]
    fn list_age_wording() {
        let mut entries = disk_cache::Entries::new();
        for (h, age) in [("a", 0), ("b", 1), ("c", 5)] {
            entries.insert(
                h.to_string(),
                Entry {
                    hostname: h.to_string(),
                    timestamp: 1_000_000 - age * 86_400,
                    terminfo_version: "x".into(),
                },
            );
        }
        let mut out = Vec::new();
        list_entries(&entries, 1_000_000, &mut out).unwrap();
        assert_eq!(
            "Cached hosts (3):\n  a (today)\n  b (yesterday)\n  c (5 days ago)\n",
            String::from_utf8(out).unwrap()
        );
    }
}
