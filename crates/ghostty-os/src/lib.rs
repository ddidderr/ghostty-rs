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
#[cfg(not(target_arch = "wasm32"))]
pub mod cf_release_thread;
#[cfg(not(target_arch = "wasm32"))]
pub mod cgroup;
#[cfg(not(target_arch = "wasm32"))]
pub mod dbus;
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub mod env;
#[cfg(not(target_arch = "wasm32"))]
pub mod file;
pub mod file_type;
#[cfg(unix)]
pub mod flatpak;
pub mod osbytes;
#[cfg(windows)]
pub mod flatpak {
    //! Flatpak does not exist on Windows.

    /// Always false on Windows.
    pub fn is_flatpak() -> bool {
        false
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub mod homedir;
#[cfg(not(target_arch = "wasm32"))]
pub mod hostname;
#[cfg(not(target_arch = "wasm32"))]
pub mod i18n;
#[cfg(not(target_arch = "wasm32"))]
mod i18n_canon_table;
pub mod i18n_locales;
#[cfg(not(target_arch = "wasm32"))]
pub mod kernel_info;
#[cfg(not(target_arch = "wasm32"))]
pub mod locale;
#[cfg(unix)]
pub mod mach;
#[cfg(unix)]
pub mod macos;
pub mod mouse;
#[cfg(not(target_arch = "wasm32"))]
pub mod open;
#[cfg(unix)]
pub mod passwd;
#[cfg(windows)]
pub mod passwd {
    //! `passwd.zig` does not exist on Windows (it `@compileError`s). This
    //! stand-in reports an empty entry, which is what Zig callers' fallback
    //! paths expect.

    /// Entry retrieved from the passwd API (same shape as the Unix one).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Entry {
        pub shell: Option<String>,
        pub home: Option<String>,
        pub name: Option<String>,
    }

    /// Byte-preserving entry (same shape as the Unix one).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct EntryOs {
        pub shell: Option<std::ffi::OsString>,
        pub home: Option<std::ffi::OsString>,
        pub name: Option<std::ffi::OsString>,
    }

    /// No passwd database on Windows: always an empty entry.
    pub fn get() -> std::io::Result<Entry> {
        Ok(Entry::default())
    }

    /// See [`get`].
    pub fn get_os() -> std::io::Result<EntryOs> {
        Ok(EntryOs::default())
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub mod path;
#[cfg(not(target_arch = "wasm32"))]
pub mod pipe;
#[cfg(not(target_arch = "wasm32"))]
pub mod resourcesdir;
pub mod shell;
pub mod string_encoding;
#[cfg(unix)]
pub mod systemd;
#[cfg(not(target_arch = "wasm32"))]
pub mod temp_dir;
pub mod uri;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
#[cfg(windows)]
pub mod windows;
pub mod windows_nls;

#[cfg(target_arch = "wasm32")]
mod unsupported;
#[cfg(target_arch = "wasm32")]
pub use unsupported::homedir::expand_home;
#[cfg(target_arch = "wasm32")]
pub use unsupported::{ResourcesDir, homedir, is_flatpak, passwd, resources_dir, xdg};
#[cfg(not(target_arch = "wasm32"))]
pub mod xdg;

#[cfg(test)]
pub(crate) mod testutil;

// Functions and types (mirrors the flat exports of `os/main.zig`).
#[cfg(not(target_arch = "wasm32"))]
pub use dbus::launched_by_dbus_activation;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::{DesktopEnvironment, desktop_environment, launched_from_desktop};
#[cfg(not(target_arch = "wasm32"))]
pub use env::{
    EnvMap,
    append_env,
    append_env_always,
    append_env_os,
    filter_snap_paths,
    get_env_map,
    getenv,
    getenv_not_empty,
    getenv_not_empty_os,
    getenv_os,
    prepend_env,
    prepend_env_os,
};
#[cfg(not(target_arch = "wasm32"))]
pub use file::{Rlimit, alloc_tmp_dir_os, fix_max_files, restore_max_files};
#[cfg(unix)]
pub use flatpak::FlatpakHostCommand;
#[cfg(not(target_arch = "wasm32"))]
pub use flatpak::is_flatpak;
#[cfg(not(target_arch = "wasm32"))]
pub use homedir::{expand_home, home};
#[cfg(not(target_arch = "wasm32"))]
pub use kernel_info::get_kernel_info;
#[cfg(not(target_arch = "wasm32"))]
pub use locale::ensure_locale;
pub use mouse::click_interval;
#[cfg(not(target_arch = "wasm32"))]
pub use open::open;
#[cfg(not(target_arch = "wasm32"))]
pub use pipe::pipe;
#[cfg(not(target_arch = "wasm32"))]
pub use resourcesdir::{ResourcesDir, resources_dir};
pub use shell::ShellEscapeWriter;
#[cfg(unix)]
pub use systemd::launched_by_systemd;
#[cfg(not(target_arch = "wasm32"))]
pub use temp_dir::TempDir;

/// The native path separator (`std.fs.path.sep`).
#[cfg(windows)]
const SEP: char = '\\';
#[cfg(not(any(windows, target_arch = "wasm32")))]
const SEP: char = '/';

/// Join path components like `std.fs.path.join` for the host platform:
/// empty components are skipped and exactly one separator is placed
/// between components.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn path_join(parts: &[&str]) -> String {
    path_join_with(SEP, parts)
}

/// [`path_join`] with an explicit separator (so both flavours are testable
/// on any host).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn path_join_with(sep: char, parts: &[&str]) -> String {
    // Windows accepts `/` as a separator too; Zig treats both as separators
    // when joining.
    let is_sep = |c: char| c == sep || (sep == '\\' && c == '/');
    let mut out = String::new();
    for p in parts {
        if p.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(p);
            continue;
        }
        let prev_sep = out.ends_with(is_sep);
        let this_sep = p.starts_with(is_sep);
        match (prev_sep, this_sep) {
            (true, true) => out.push_str(&p[1..]),
            (false, false) => {
                out.push(sep);
                out.push_str(p);
            }
            _ => out.push_str(p),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::path_join_with;

    #[test]
    fn path_join_basics() {
        let j = |p: &[&str]| path_join_with('/', p);
        assert_eq!(j(&["/a", ".cache", ""]), "/a/.cache");
        assert_eq!(j(&["/a/", "/b"]), "/a/b");
        assert_eq!(j(&["", "b"]), "b");
        assert_eq!(j(&["/a", "b", "c"]), "/a/b/c");
    }

    #[test]
    fn path_join_windows_flavour() {
        let j = |p: &[&str]| path_join_with('\\', p);
        assert_eq!(j(&["C:\\a", ".cache", ""]), "C:\\a\\.cache");
        assert_eq!(j(&["C:\\a\\", "\\b"]), "C:\\a\\b");
        assert_eq!(j(&["C:\\a", "/b"]), "C:\\a/b");
        assert_eq!(j(&["", "b"]), "b");
    }
}
