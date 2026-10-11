//! The "os" package contains utilities for interfacing with the operating
//! system. These aren't restricted to syscalls or low-level operations, but
//! also OS-specific features and conventions.
//!
//! Port of `src/os/*.zig` (Linux only) plus `src/file_type.zig`. See
//! `docs/rewrite/notes/ghostty-os.md` for deviations and skipped files.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::assert_is_empty,
        clippy::items_after_statements
    )
)]

pub mod args;
pub mod cgroup;
pub mod dbus;
pub mod desktop;
pub mod env;
pub mod file;
pub mod file_type;
pub mod flatpak;
pub mod homedir;
pub mod hostname;
pub mod i18n;
pub mod i18n_locales;
pub mod kernel_info;
pub mod locale;
pub mod mouse;
pub mod open;
pub mod passwd;
pub mod path;
pub mod pipe;
pub mod resourcesdir;
pub mod shell;
pub mod string_encoding;
pub mod systemd;
pub mod temp_dir;
pub mod uri;
pub mod xdg;

#[cfg(test)]
pub(crate) mod testutil;

// Functions and types (mirrors the flat exports of `os/main.zig`).
pub use dbus::launched_by_dbus_activation;
pub use desktop::{DesktopEnvironment, desktop_environment, launched_from_desktop};
pub use env::{append_env, append_env_always, get_env_map, getenv, getenv_not_empty, prepend_env};
pub use file::{Rlimit, alloc_tmp_dir, fix_max_files, restore_max_files};
pub use flatpak::{FlatpakHostCommand, is_flatpak};
pub use homedir::{expand_home, home};
pub use kernel_info::get_kernel_info;
pub use locale::ensure_locale;
pub use mouse::click_interval;
pub use open::open;
pub use pipe::pipe;
pub use resourcesdir::{ResourcesDir, resources_dir};
pub use shell::ShellEscapeWriter;
pub use systemd::launched_by_systemd;
pub use temp_dir::TempDir;

/// Join path components like `std.fs.path.join` (POSIX): empty components
/// are skipped and exactly one separator is placed between components.
pub(crate) fn path_join(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts {
        if p.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(p);
            continue;
        }
        let prev_sep = out.ends_with('/');
        let this_sep = p.starts_with('/');
        match (prev_sep, this_sep) {
            (true, true) => out.push_str(&p[1..]),
            (false, false) => {
                out.push('/');
                out.push_str(p);
            }
            _ => out.push_str(p),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::path_join;

    #[test]
    fn path_join_basics() {
        assert_eq!(path_join(&["/a", ".cache", ""]), "/a/.cache");
        assert_eq!(path_join(&["/a/", "/b"]), "/a/b");
        assert_eq!(path_join(&["", "b"]), "b");
        assert_eq!(path_join(&["/a", "b", "c"]), "/a/b/c");
    }
}
