#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::flatpak::is_flatpak;

/// Entry that is retrieved from the passwd API. This only contains the fields
/// we care about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub shell: Option<String>,
    pub home: Option<String>,
    pub name: Option<String>,
}

/// Get the passwd entry for the currently executing user.
pub fn get() -> std::io::Result<Entry> {
    let uid = nix::unistd::getuid();
    let user = match nix::unistd::User::from_uid(uid) {
        Ok(Some(user)) => user,
        Ok(None) => {
            // Future: let's check if a better shell is available like zsh
            log::warn!("no pw entry to detect default shell, will default to 'sh'");
            return Ok(Entry::default());
        }
        Err(e) => {
            log::warn!("error retrieving pw entry code={e}");
            return Ok(Entry::default());
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

    Ok(Entry {
        shell: Some(user.shell.to_string_lossy().into_owned()),
        home: Some(user.dir.to_string_lossy().into_owned()),
        name: Some(user.name),
    })
}

/// Flatpak: look up our entry on the host with `getent passwd <name>` over a
/// pty (Zig `passwd.get`, flatpak branch).
///
/// Dependency note: Zig uses `pty.zig`; `ghostty-termio` (which owns the Rust
/// `Pty`) depends on this crate, so a minimal openpt/ptsname pair is opened
/// here with rustix instead. Like Zig, only `shell` and `home` are filled in.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn host_entry(name: &str) -> std::io::Result<Entry> {
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

    Ok(parse_getent(&String::from_utf8_lossy(&output)))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
/// Shell and home are the last two `:`-separated fields of the output.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn parse_getent(output: &str) -> Entry {
    let mut it = output.trim_end_matches([' ', '\r', '\n']).rsplit(':');
    let shell = it.next().map(str::to_owned);
    let home = it.next().map(str::to_owned);
    Entry {
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
        let e = parse_getent("mitchell:x:1000:1000:Mitchell:/home/mitchell:/bin/zsh\r\n");
        assert_eq!(e.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(e.home.as_deref(), Some("/home/mitchell"));
        assert_eq!(e.name, None);
    }

    #[test]
    fn get_entry() {
        // We should be able to get an entry
        let entry = get().unwrap();
        assert!(entry.shell.is_some());
        assert!(entry.home.is_some());
    }
}
