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

    // TODO(integration): Zig shells out to the host via
    // FlatpakHostCommand + a pty (`getent passwd <name>`) when running in
    // Flatpak, because the sandbox passwd entry only has a correct username.
    // That needs the `ghostty-termio` Pty; until then we use the sandbox
    // entry and warn.
    if is_flatpak() {
        log::warn!(
            "flatpak detected, but host passwd lookup is not implemented; \
             using sandbox passwd entry"
        );
    }

    Ok(Entry {
        shell: Some(user.shell.to_string_lossy().into_owned()),
        home: Some(user.dir.to_string_lossy().into_owned()),
        name: Some(user.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_entry() {
        // We should be able to get an entry
        let entry = get().unwrap();
        assert!(entry.shell.is_some());
        assert!(entry.home.is_some());
    }
}
