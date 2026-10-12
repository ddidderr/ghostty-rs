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
#[cfg(not(target_arch = "wasm32"))]
pub mod flatpak;
#[cfg(not(target_arch = "wasm32"))]
pub mod homedir;
#[cfg(not(target_arch = "wasm32"))]
pub mod hostname;
#[cfg(not(target_arch = "wasm32"))]
pub mod i18n;
pub mod i18n_locales;
#[cfg(not(target_arch = "wasm32"))]
pub mod kernel_info;
#[cfg(not(target_arch = "wasm32"))]
pub mod locale;
pub mod mouse;
#[cfg(not(target_arch = "wasm32"))]
pub mod open;
#[cfg(not(target_arch = "wasm32"))]
pub mod passwd;
#[cfg(not(target_arch = "wasm32"))]
pub mod path;
#[cfg(not(target_arch = "wasm32"))]
pub mod pipe;
#[cfg(not(target_arch = "wasm32"))]
pub mod resourcesdir;
pub mod shell;
pub mod string_encoding;
#[cfg(not(target_arch = "wasm32"))]
pub mod systemd;
#[cfg(not(target_arch = "wasm32"))]
pub mod temp_dir;
pub mod uri;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

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
    prepend_env,
    prepend_env_os,
};
#[cfg(not(target_arch = "wasm32"))]
pub use file::{Rlimit, alloc_tmp_dir, fix_max_files, restore_max_files};
#[cfg(not(target_arch = "wasm32"))]
pub use flatpak::{FlatpakHostCommand, is_flatpak};
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
#[cfg(not(target_arch = "wasm32"))]
pub use systemd::launched_by_systemd;
#[cfg(not(target_arch = "wasm32"))]
pub use temp_dir::TempDir;

/// Join path components like `std.fs.path.join` (POSIX): empty components
/// are skipped and exactly one separator is placed between components.
#[cfg(not(target_arch = "wasm32"))]
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
