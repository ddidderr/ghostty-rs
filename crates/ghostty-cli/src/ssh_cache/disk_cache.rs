//! An SSH terminfo entry cache that stores its cache data on disk
//! (ZIG/src/cli/ssh-cache/DiskCache.zig). The cache only stores metadata
//! (hostname, terminfo value, etc.) and does not store any sensitive data.

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::Ipv6Addr,
    path::PathBuf,
};

use super::entry::{Entry, now_timestamp};

/// 512KB - sufficient for approximately 10k entries
const MAX_CACHE_SIZE: u64 = 512 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("CacheIsLocked")]
    CacheIsLocked,
    #[error("HostnameIsInvalid")]
    HostnameIsInvalid,
    /// The general error that is returned for any filesystem error
    /// that may have resulted in the XDG lookup failing.
    #[error("XdgLookupFailed")]
    XdgLookupFailed,
    #[error("InvalidCachePath")]
    InvalidCachePath,
    #[error("StreamTooLong")]
    StreamTooLong,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddResult {
    Added,
    Updated,
}

pub type Entries = BTreeMap<String, Entry>;

#[derive(Debug, Clone)]
pub struct DiskCache {
    /// Path to a file where the cache is stored.
    pub path: PathBuf,
}

impl DiskCache {
    /// Returns the default path for the cache for a given program.
    ///
    /// On all platforms, this is `${XDG_STATE_HOME}/ghostty/ssh_cache`.
    pub fn default_path(program: &str) -> Result<PathBuf, Error> {
        let state_dir = ghostty_os::xdg::state(&ghostty_os::xdg::Options {
            subdir: Some(program),
            home: None,
        })
        .map_err(|_| Error::XdgLookupFailed)?;
        Ok(PathBuf::from(state_dir).join("ssh_cache"))
    }

    /// Clear all cache data stored in the disk cache.
    /// This removes the cache file from disk, effectively clearing all cached
    /// SSH terminfo entries.
    pub fn clear(&self) -> Result<(), Error> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Add or update a hostname entry in the cache.
    /// Returns `AddResult::Added` for new entries or `AddResult::Updated` for
    /// existing ones. The cache file is created if it doesn't exist with
    /// secure permissions (0600).
    pub fn add(&self, hostname: &str) -> Result<AddResult, Error> {
        if !is_valid_cache_key(hostname) {
            return Err(Error::HostnameIsInvalid);
        }

        // Create cache directory if needed
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        // Open or create cache file with secure permissions
        let mut open = OpenOptions::new();
        open.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        open.mode(0o600);
        let file = open.open(&self.path)?;
        fixup_permissions(&file)?;

        // Lock
        let _lock = lock(&file)?;

        let mut entries = read_entries(&file)?;

        // Add or update entry
        let result = if let Some(e) = entries.get_mut(hostname) {
            // Update timestamp for existing entry
            e.timestamp = now_timestamp();
            AddResult::Updated
        } else {
            entries.insert(
                hostname.to_string(),
                Entry {
                    hostname: hostname.to_string(),
                    timestamp: now_timestamp(),
                    terminfo_version: "xterm-ghostty".to_string(),
                },
            );
            AddResult::Added
        };

        self.write_cache_file(&entries, None)?;
        Ok(result)
    }

    /// Remove a hostname entry from the cache.
    /// No error is returned if the hostname doesn't exist or the cache file
    /// is missing.
    pub fn remove(&self, hostname: &str) -> Result<(), Error> {
        if !is_valid_cache_key(hostname) {
            return Err(Error::HostnameIsInvalid);
        }

        // Open our file
        let file = match OpenOptions::new().read(true).write(true).open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        fixup_permissions(&file)?;

        // Lock
        let _lock = lock(&file)?;

        // Read existing entries
        let mut entries = read_entries(&file)?;
        entries.remove(hostname);

        self.write_cache_file(&entries, None)
    }

    /// Check if a hostname exists in the cache.
    /// Returns false if the cache file doesn't exist.
    pub fn contains(&self, hostname: &str) -> Result<bool, Error> {
        if !is_valid_cache_key(hostname) {
            return Err(Error::HostnameIsInvalid);
        }

        // Open our file
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };

        let entries = read_entries(&file)?;
        Ok(entries.contains_key(hostname))
    }

    /// List all entries in the cache.
    /// Returns an empty map if the cache file doesn't exist.
    pub fn list(&self) -> Result<Entries, Error> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Entries::new()),
            Err(e) => return Err(e.into()),
        };
        read_entries(&file)
    }

    /// Atomically (temp file + rename) write the non-expired entries.
    fn write_cache_file(&self, entries: &Entries, expire_days: Option<u32>) -> Result<(), Error> {
        let cache_dir = self.path.parent().ok_or(Error::InvalidCachePath)?;
        let cache_basename = self.path.file_name().ok_or(Error::InvalidCachePath)?;

        let mut data = String::new();
        for e in entries.values() {
            // Only write non-expired entries
            if e.is_expired(expire_days) {
                continue;
            }
            e.format(&mut data);
        }

        let mut tmp_name = cache_basename.to_os_string();
        tmp_name.push(format!(".{}.tmp", std::process::id()));
        let tmp_path = cache_dir.join(tmp_name);

        let result = (|| -> std::io::Result<()> {
            let mut open = OpenOptions::new();
            open.write(true).create(true).truncate(true);
            #[cfg(unix)]
            open.mode(0o600);
            let mut f = open.open(&tmp_path)?;
            f.write_all(data.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp_path, &self.path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        Ok(result?)
    }
}

/// Holds an advisory exclusive lock on a file until dropped.
struct Lock<'a>(Option<&'a File>);

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        if let Some(f) = self.0 {
            let _ = f.unlock();
        }
    }
}

/// Zig: `_ = file.tryLock(.exclusive) catch return error.CacheIsLocked;`
/// The boolean result is ignored upstream, so a lock held by someone else
/// does not fail; only a real error does.
///
/// Zig skips locking entirely on Windows
/// (`if (comptime builtin.os.tag != .windows)`), so we do too. On unix
/// `File::try_lock` is `flock(LOCK_EX | LOCK_NB)`, the same call as Zig's
/// `File.tryLock(.exclusive)`.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
fn lock(_file: &File) -> Result<Lock<'_>, Error> {
    Ok(Lock(None))
}

#[cfg(not(windows))]
fn lock(file: &File) -> Result<Lock<'_>, Error> {
    match file.try_lock() {
        Ok(()) => Ok(Lock(Some(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Lock(None)),
        Err(std::fs::TryLockError::Error(_)) => Err(Error::CacheIsLocked),
    }
}

#[cfg_attr(windows, allow(clippy::unnecessary_wraps))]
fn fixup_permissions(file: &File) -> Result<(), Error> {
    // Windows does not support chmod
    #[cfg(windows)]
    {
        let _ = file;
        Ok(())
    }
    #[cfg(unix)]
    fixup_permissions_unix(file)
}

#[cfg(unix)]
fn fixup_permissions_unix(file: &File) -> Result<(), Error> {
    // Ensure file has correct permissions (readable/writable by
    // owner only)
    let meta = file.metadata()?;
    if meta.permissions().mode() & 0o777 != 0o600 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_entries(file: &File) -> Result<Entries, Error> {
    let mut content = Vec::new();
    let mut f = file;
    (&mut f)
        .take(MAX_CACHE_SIZE + 1)
        .read_to_end(&mut content)?;
    if content.len() as u64 > MAX_CACHE_SIZE {
        return Err(Error::StreamTooLong);
    }
    let content = String::from_utf8_lossy(&content);

    let mut entries = Entries::new();
    for line in content.split('\n').filter(|l| !l.is_empty()) {
        let trimmed = line.trim_matches([' ', '\t', '\r']);
        let Some(entry) = Entry::parse(trimmed) else {
            continue;
        };

        match entries.get_mut(&entry.hostname) {
            None => {
                entries.insert(entry.hostname.clone(), entry);
            }
            Some(existing) => {
                // Handle duplicate entries - keep newer timestamp
                if entry.timestamp > existing.timestamp {
                    existing.timestamp = entry.timestamp;
                    if existing.terminfo_version != entry.terminfo_version {
                        existing.terminfo_version = entry.terminfo_version;
                    }
                }
            }
        }
    }

    Ok(entries)
}

// Supports both standalone hostnames and user@hostname format
fn is_valid_cache_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }

    // Check for user@hostname format
    if let Some(at_pos) = key.find('@') {
        let user = &key[..at_pos];
        let hostname = &key[at_pos + 1..];
        return is_valid_user(user) && is_valid_host(hostname);
    }

    is_valid_host(key)
}

// Checks if a host is a valid hostname or IP address
fn is_valid_host(host: &str) -> bool {
    // First check for valid hostnames because this is assumed to be the more
    // likely ssh host format.
    if ghostty_os::hostname::is_valid(host) {
        return true;
    }

    // We also accept valid IP addresses. In practice, IPv4 addresses are also
    // considered valid hostnames due to their overlapping syntax, so we can
    // simplify this check to be IPv6-specific.
    host.parse::<Ipv6Addr>().is_ok()
}

fn is_valid_user(user: &str) -> bool {
    if user.is_empty() || user.len() > 64 {
        return false;
    }
    user.bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #[cfg(not(windows))]
    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        let a = File::create(&path).unwrap();
        let b = File::options().read(true).write(true).open(&path).unwrap();

        let held = super::lock(&a).unwrap();
        assert!(held.0.is_some());
        // A second handle can't take the lock; Zig ignores that result.
        assert!(matches!(
            b.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        assert!(super::lock(&b).unwrap().0.is_none());
        drop(held);
        // Unlocked on drop.
        assert!(b.try_lock().is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn lock_is_noop_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let a = File::create(dir.path().join("lock")).unwrap();
        assert!(super::lock(&a).unwrap().0.is_none());
    }
    use super::*;

    #[test]
    fn disk_cache_default_path() {
        let path = DiskCache::default_path("ghostty").unwrap();
        assert!(!path.as_os_str().is_empty());
        assert!(path.ends_with("ghostty/ssh_cache"));
    }

    #[test]
    fn disk_cache_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache");
        std::fs::write(&path, "HELLO!").unwrap();

        let cache = DiskCache { path: path.clone() };
        cache.clear().unwrap();

        // Verify the file is gone
        assert_eq!(
            std::io::ErrorKind::NotFound,
            File::open(&path).unwrap_err().kind()
        );
        // Clearing again is not an error.
        cache.clear().unwrap();
    }

    #[test]
    fn disk_cache_operations() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache");
        std::fs::write(&path, "HELLO!").unwrap();

        // Setup our cache
        let cache = DiskCache { path };
        assert_eq!(AddResult::Added, cache.add("example.com").unwrap());
        assert_eq!(AddResult::Updated, cache.add("example.com").unwrap());
        assert!(cache.contains("example.com").unwrap());

        // List
        let entries = cache.list().unwrap();
        assert_eq!(1, entries.len());

        // Remove
        cache.remove("example.com").unwrap();
        assert!(!cache.contains("example.com").unwrap());
        assert_eq!(AddResult::Added, cache.add("example.com").unwrap());
    }

    #[test]
    fn disk_cache_cleans_up_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            path: tmp.path().join("cache"),
        };
        assert_eq!(AddResult::Added, cache.add("example.com").unwrap());
        assert_eq!(AddResult::Added, cache.add("example.org").unwrap());

        // Verify only the cache file exists and no temp files left behind
        let names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(vec![std::ffi::OsString::from("cache")], names);
    }

    #[cfg(unix)]
    #[test]
    fn disk_cache_file_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            path: tmp.path().join("sub/cache"),
        };
        cache.add("example.com").unwrap();
        let mode = std::fs::metadata(&cache.path).unwrap().permissions().mode();
        assert_eq!(0o600, mode & 0o777);
    }

    #[test]
    fn disk_cache_invalid_host() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            path: tmp.path().join("sub/cache"),
        };
        cache.add("example.com").unwrap();
        assert!(matches!(
            cache.add("bad host"),
            Err(Error::HostnameIsInvalid)
        ));
        assert!(matches!(
            cache.contains("host name"),
            Err(Error::HostnameIsInvalid)
        ));
    }

    #[test]
    fn duplicate_entries_keep_newer_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache");
        std::fs::write(&path, "a.com|10|old\na.com|20|new\na.com|15|mid\n").unwrap();
        let e = DiskCache { path }.list().unwrap();
        assert_eq!(20, e["a.com"].timestamp);
        assert_eq!("new", e["a.com"].terminfo_version);
    }

    #[test]
    fn is_valid_host_cases() {
        // Valid hostnames
        assert!(is_valid_host("localhost"));
        assert!(is_valid_host("example.com"));
        assert!(is_valid_host("sub.example.com"));

        // IPv4 addresses
        assert!(is_valid_host("127.0.0.1"));
        assert!(is_valid_host("192.168.1.1"));

        // IPv6 addresses
        assert!(is_valid_host("::1"));
        assert!(is_valid_host("2001:db8::1"));
        assert!(is_valid_host("2001:db8:0:1:1:1:1:1"));
        assert!(!is_valid_host("fe80::1%eth0")); // scopes not supported

        // Invalid hosts
        assert!(!is_valid_host(""));
        assert!(!is_valid_host("host\nname"));
        assert!(!is_valid_host(".example.com"));
        assert!(!is_valid_host("host..domain"));
        assert!(!is_valid_host("-hostname"));
        assert!(!is_valid_host("hostname-"));
        assert!(!is_valid_host("host name"));
        assert!(!is_valid_host("host_name"));
        assert!(!is_valid_host("host@domain"));
        assert!(!is_valid_host("host:port"));
    }

    #[test]
    fn is_valid_user_cases() {
        // Valid
        assert!(is_valid_user("user"));
        assert!(is_valid_user("user-user"));
        assert!(is_valid_user("user_name"));
        assert!(is_valid_user("user.name"));
        assert!(is_valid_user("user123"));

        // Invalid
        assert!(!is_valid_user(""));
        assert!(!is_valid_user("user name"));
        assert!(!is_valid_user("user@example"));
        assert!(!is_valid_user("user:group"));
        assert!(!is_valid_user("user\nname"));
        assert!(!is_valid_user(&"a".repeat(65))); // too long
    }

    #[test]
    fn is_valid_cache_key_cases() {
        // Valid
        assert!(is_valid_cache_key("example.com"));
        assert!(is_valid_cache_key("sub.example.com"));
        assert!(is_valid_cache_key("192.168.1.1"));
        assert!(is_valid_cache_key("::1"));
        assert!(is_valid_cache_key("user@example.com"));
        assert!(is_valid_cache_key("user@192.168.1.1"));
        assert!(is_valid_cache_key("user@::1"));

        // Invalid
        assert!(!is_valid_cache_key(""));
        assert!(!is_valid_cache_key(".example.com"));
        assert!(!is_valid_cache_key("@example.com"));
        assert!(!is_valid_cache_key("user@"));
        assert!(!is_valid_cache_key("user@@example"));
        assert!(!is_valid_cache_key("user@.example.com"));
    }
}
