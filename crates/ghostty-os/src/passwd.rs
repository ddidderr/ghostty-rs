use std::ffi::OsString;

#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::flatpak::is_flatpak;
use crate::osbytes::EncodedOs;

/// Entry that is retrieved from the passwd API. This only contains the fields
/// we care about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub shell: Option<EncodedOs>,
    pub home: Option<EncodedOs>,
    pub name: Option<EncodedOs>,
}

/// Byte-preserving [`Entry`] (Zig slices are arbitrary bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryOs {
    pub shell: Option<OsString>,
    pub home: Option<OsString>,
    pub name: Option<OsString>,
}

impl From<EntryOs> for Entry {
    fn from(e: EntryOs) -> Self {
        let f = |v: Option<OsString>| v.map(|v| EncodedOs::from_os(&v));
        Entry {
            shell: f(e.shell),
            home: f(e.home),
            name: f(e.name),
        }
    }
}

/// Get the passwd entry for the currently executing user. Non-UTF-8 bytes
/// are carried via the reversible [`crate::osbytes`] encoding; see
/// [`get_os`] for the raw bytes.
pub fn get() -> std::io::Result<Entry> {
    get_os().map(Entry::from)
}

/// Byte-preserving [`get`].
pub fn get_os() -> std::io::Result<EntryOs> {
    let uid = nix::unistd::getuid();
    let user = match nix::unistd::User::from_uid(uid) {
        Ok(Some(user)) => user,
        Ok(None) => {
            // Future: let's check if a better shell is available like zsh
            log::warn!("no pw entry to detect default shell, will default to 'sh'");
            return Ok(EntryOs::default());
        }
        Err(e) => {
            log::warn!("error retrieving pw entry code={e}");
            return Ok(EntryOs::default());
        }
    };

    // If we're in flatpak then our entry is always empty so we grab it
    // by shelling out to the host. note that we do HAVE an entry in the
    // sandbox but only the username is correct.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if is_flatpak() {
        log::info!("flatpak detected, will use host command to get our entry");
        return host_entry(&user.name);
    }

    Ok(EntryOs {
        shell: Some(user.shell.into_os_string()),
        home: Some(user.dir.into_os_string()),
        name: Some(OsString::from(user.name)),
    })
}

/// Flatpak: look up our entry on the host with `getent passwd <name>` over a
/// pty (Zig `passwd.get`, flatpak branch).
///
/// Dependency note: Zig uses `pty.zig`; `ghostty-termio` (which owns the Rust
/// `Pty`) depends on this crate, so a minimal openpt/ptsname pair is opened
/// here with rustix instead. Like Zig, only `shell` and `home` are filled in.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn host_entry(name: &str) -> std::io::Result<EntryOs> {
    use std::io::Read;

    use rustix::{
        fs::{Mode, OFlags},
        pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    };

    let err = std::io::Error::from;
    let master =
        openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).map_err(err)?;
    grantpt(&master).map_err(err)?;
    unlockpt(&master).map_err(err)?;
    let slave_name = ptsname(&master, Vec::new()).map_err(err)?;
    let slave = rustix::fs::open(
        slave_name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(err)?;

    // Note: we wrap our getent call in a /bin/sh login shell because
    // some operating systems (NixOS tested) don't set the PATH for various
    // utilities properly until we get a login shell.
    let mut cmd = crate::flatpak::FlatpakHostCommand::new(
        vec![
            "/bin/sh".into(),
            "-l".into(),
            "-c".into(),
            format!("getent passwd {name}"),
        ],
        slave.try_clone()?,
        slave.try_clone()?,
        slave.try_clone()?,
    );
    cmd.spawn().map_err(std::io::Error::other)?;
    cmd.wait().map_err(std::io::Error::other)?;

    // Once started, we can close the child side. We do this after
    // wait right now but that is fine too. This lets us read the
    // parent and detect EOF.
    drop(cmd);
    drop(slave);

    // Read all of our output (max total size is 1024 bytes, like Zig).
    let mut file = std::fs::File::from(master);
    let mut output = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            // EIO is triggered at the end since we closed our child side.
            // This is just EOF for this.
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => break,
            Err(e) => return Err(e),
        };
        output.extend_from_slice(&buf[..n]);
        if output.len() > buf.len() {
            break;
        }
    }

    Ok(parse_getent(&output))
}

/// Shell and home are the last two `:`-separated fields of the output.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn parse_getent(output: &[u8]) -> EntryOs {
    let end = output
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\r' | b'\n'))
        .map_or(0, |i| i + 1);
    let mut it = output[..end].rsplit(|&b| b == b':');
    let shell = it.next().map(|b| crate::osbytes::os_from_bytes(b.to_vec()));
    let home = it.next().map(|b| crate::osbytes::os_from_bytes(b.to_vec()));
    EntryOs {
        shell,
        home,
        name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn parse_getent_output() {
        let e = parse_getent(b"mitchell:x:1000:1000:Mitchell:/home/mitchell:/bin/zsh\r\n");
        assert_eq!(e.shell.as_deref(), Some(std::ffi::OsStr::new("/bin/zsh")));
        assert_eq!(
            e.home.as_deref(),
            Some(std::ffi::OsStr::new("/home/mitchell"))
        );
        assert_eq!(e.name, None);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn parse_getent_keeps_non_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let e = parse_getent(b"u:x:1:1::/home/\xff:/bin/\xfesh\n");
        assert_eq!(e.shell.unwrap().as_bytes(), b"/bin/\xfesh");
        assert_eq!(e.home.unwrap().as_bytes(), b"/home/\xff");
    }

    #[test]
    fn get_entry() {
        // We should be able to get an entry
        let entry = get().unwrap();
        assert!(entry.shell.is_some());
        assert!(entry.home.is_some());
    }
}
