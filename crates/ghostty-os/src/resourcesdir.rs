use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use crate::osbytes::EncodedOs;

fn enc(p: &Path) -> EncodedOs {
    EncodedOs::from_path(p)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcesDir {
    /// Avoid accessing these directly, use the app() and host() methods instead.
    pub app_path: Option<EncodedOs>,
    pub host_path: Option<EncodedOs>,
}

impl ResourcesDir {
    /// Get the directory to the bundled resources directory accessible
    /// by the application.
    pub fn app(&self) -> Option<&EncodedOs> {
        self.app_path.as_ref()
    }

    /// Get the directory to the bundled resources directory accessible
    /// by the host environment (i.e. for sandboxed applications). The
    /// returned directory might not be accessible from the application
    /// itself.
    ///
    /// In non-sandboxed environment, this should be the same as app().
    pub fn host(&self) -> Option<&EncodedOs> {
        self.host_path.as_ref().or(self.app_path.as_ref())
    }

    /// Byte-preserving [`ResourcesDir::app`]: the string fields carry
    /// non-UTF-8 bytes via [`crate::osbytes`], this decodes them.
    pub fn app_os(&self) -> Option<PathBuf> {
        self.app().and_then(|p| p.to_path().ok())
    }

    /// Byte-preserving [`ResourcesDir::host`].
    pub fn host_os(&self) -> Option<PathBuf> {
        self.host().and_then(|p| p.to_path().ok())
    }
}

/// Sentinels we look for in the path to know we've found the resources
/// directory (Linux).
#[cfg(not(any(windows, target_os = "macos", target_os = "freebsd")))]
const SENTINELS: [&str; 2] = ["terminfo/g/ghostty", "terminfo/x/xterm-ghostty"];

/// Sentinels on FreeBSD (`site-terminfo`, like Zig).
#[cfg(target_os = "freebsd")]
const SENTINELS: [&str; 2] = ["site-terminfo/g/ghostty", "site-terminfo/x/xterm-ghostty"];

/// Sentinel on macOS (terminfo directories are named by hex there).
#[cfg(target_os = "macos")]
const SENTINELS: [&str; 1] = ["terminfo/78/xterm-ghostty"];

/// Sentinel on Windows (a single terminfo source file).
#[cfg(windows)]
const SENTINELS: [&str; 1] = ["terminfo/ghostty.terminfo"];

/// The in-repo `resources/` tree always carries the Linux-style terminfo
/// layout (also on macOS, where the installed layout uses hex dirs).
const DEV_SENTINELS: [&str; 1] = ["terminfo/x/xterm-ghostty"];

/// Gets the directory to the bundled resources directory, if it
/// exists (not all platforms or packages have it).
///
/// Zig's `builtin.mode == .Debug` is mapped to the Cargo `dev` profile
/// (`cfg(ghostty_debug_build)`, set by `build.rs`), not to `debug_assertions`:
/// the repository's `release` profile enables assertions like ReleaseSafe.
pub fn resources_dir() -> io::Result<ResourcesDir> {
    let exe = std::env::current_exe().ok();
    let env = std::env::var_os("GHOSTTY_RESOURCES_DIR");
    Ok(resources_dir_impl(
        cfg!(ghostty_debug_build),
        exe.as_deref(),
        env,
    ))
}

fn resources_dir_impl(debug: bool, exe: Option<&Path>, env: Option<OsString>) -> ResourcesDir {
    // Use the GHOSTTY_RESOURCES_DIR environment variable in release builds.
    //
    // In debug builds we try using terminfo detection first instead, since
    // if debug Ghostty is launched by an older version of Ghostty, it
    // would inherit the old, stale resources of older Ghostty instead of the
    // freshly built ones under zig-out/share/ghostty.
    if !debug && let Some(dir) = env.as_ref().filter(|d| !d.is_empty()) {
        return ResourcesDir {
            app_path: Some(EncodedOs::from_os(dir)),
            host_path: None,
        };
    }

    // Get the path to our running binary
    let Some(mut exe) = exe else {
        return ResourcesDir::default();
    };
    let exe_start = exe;

    // We have an exe path! Climb the tree looking for the terminfo
    // bundle as we expect it.
    while let Some(dir) = exe.parent() {
        exe = dir;
        // On MacOS, we look for the app bundle path.
        #[cfg(target_vendor = "apple")]
        for sentinel in SENTINELS {
            if let Some(v) = maybe_dir_os(dir, "Contents/Resources", sentinel) {
                return ResourcesDir {
                    app_path: Some(enc(&v.join("ghostty"))),
                    host_path: None,
                };
            }
        }

        // We look for a /usr/share style path (/usr/local/share on FreeBSD).
        let share = if cfg!(target_os = "freebsd") {
            "local/share"
        } else {
            "share"
        };
        for sentinel in SENTINELS {
            if let Some(v) = maybe_dir_os(dir, share, sentinel) {
                return ResourcesDir {
                    app_path: Some(enc(&v.join("ghostty"))),
                    host_path: None,
                };
            }
        }
    }

    // If terminfo detection failed in debug builds (somehow),
    // fallback and use the provided resources dir.
    if debug && let Some(dir) = env.filter(|d| !d.is_empty()) {
        return ResourcesDir {
            app_path: Some(EncodedOs::from_os(&dir)),
            host_path: None,
        };
    }

    // Rust-specific: development builds run out of `<workspace>/target`, where
    // there is no installed `share/` tree. Fall back to the in-repo
    // `<workspace>/resources` directory (it carries the same sentinel,
    // `terminfo/x/xterm-ghostty`, see scripts/gen-terminfo.sh) if the binary
    // lives inside this workspace.
    if debug && let Some(dir) = dev_resources_dir(exe_start) {
        return ResourcesDir {
            app_path: Some(enc(&dir)),
            host_path: None,
        };
    }

    ResourcesDir::default()
}

/// The workspace root this crate was compiled in (`crates/ghostty-os/../..`).
fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Returns `<workspace>/resources` if `exe` is inside the workspace this
/// crate was built from and the directory contains our sentinel.
fn dev_resources_dir(exe: &Path) -> Option<PathBuf> {
    let root = workspace_root().canonicalize().ok()?;
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    if !exe.starts_with(&root) {
        return None;
    }
    let res = root.join("resources");
    SENTINELS
        .iter()
        .chain(DEV_SENTINELS.iter())
        .any(|s| res.join(s).exists())
        .then_some(res)
}

/// Byte-preserving [`maybe_dir`].
pub fn maybe_dir_os(base: &Path, sub: &str, suffix: &str) -> Option<PathBuf> {
    let dir = base.join(sub);
    // Folder doesn't exist. If a different error happens its okay
    // we just ignore it and move on.
    dir.join(suffix).exists().then_some(dir)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn resources_dir_accessors() {
        let mut r = ResourcesDir::default();
        assert_eq!(r.app().map(EncodedOs::as_encoded), None);
        assert_eq!(r.host().map(EncodedOs::as_encoded), None);
        r.app_path = Some(EncodedOs::from_encoded("/a"));
        assert_eq!(r.host().map(EncodedOs::as_encoded), Some("/a"));
        r.host_path = Some(EncodedOs::from_encoded("/h"));
        assert_eq!(r.app().map(EncodedOs::as_encoded), Some("/a"));
        assert_eq!(r.host().map(EncodedOs::as_encoded), Some("/h"));
    }

    fn make_tree() -> crate::TempDir {
        let td = crate::TempDir::init().unwrap();
        std::fs::create_dir_all(td.path().join("share/terminfo/x")).unwrap();
        std::fs::write(td.path().join("share/terminfo/x/xterm-ghostty"), b"").unwrap();
        std::fs::create_dir_all(td.path().join("bin")).unwrap();
        td
    }

    #[test]
    fn detects_share_dir_from_exe() {
        let td = make_tree();
        let exe = td.path().join("bin/ghostty");
        let r = resources_dir_impl(true, Some(&exe), None);
        let expected = format!("{}/share/ghostty", td.path().display());
        assert_eq!(r.app().map(EncodedOs::as_encoded), Some(expected.as_str()));
    }

    #[test]
    fn env_in_release_wins_and_in_debug_is_fallback() {
        let td = make_tree();
        let exe = td.path().join("bin/ghostty");
        let env = Some(OsString::from("/env/res"));
        assert_eq!(
            resources_dir_impl(false, Some(&exe), env.clone())
                .app()
                .map(EncodedOs::as_encoded),
            Some("/env/res")
        );
        assert_ne!(
            resources_dir_impl(true, Some(&exe), env.clone())
                .app()
                .map(EncodedOs::as_encoded),
            Some("/env/res")
        );
        // Debug fallback when detection fails.
        assert_eq!(
            resources_dir_impl(true, Some(Path::new("/nonexistent/bin/x")), env)
                .app()
                .map(EncodedOs::as_encoded),
            Some("/env/res")
        );
        // Empty env is ignored.
        assert_eq!(
            resources_dir_impl(false, None, Some(OsString::new()))
                .app()
                .map(EncodedOs::as_encoded),
            None
        );
    }

    #[test]
    fn dev_fallback_finds_workspace_resources() {
        let exe = workspace_root().join("target/debug/ghostty");
        let r = resources_dir_impl(true, Some(&exe), None);
        assert!(r.app().is_some_and(|d| d.ends_with_plain("/resources")));
        // Outside the workspace there is no fallback.
        let r = resources_dir_impl(true, Some(Path::new("/nonexistent/bin/x")), None);
        assert_eq!(r.app().map(EncodedOs::as_encoded), None);
        // Release builds never use it.
        let r = resources_dir_impl(false, Some(&exe), None);
        assert_eq!(r.app().map(EncodedOs::as_encoded), None);
    }

    #[test]
    fn non_utf8_env_dir_is_honored_and_exe_dir_detected() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let env = OsString::from_vec(b"/env/\xffres".to_vec());
        let r = resources_dir_impl(false, None, Some(env.clone()));
        assert_eq!(r.app_os().unwrap().as_os_str(), env.as_os_str());

        // Executable lives under a directory with a non-UTF-8 name.
        let td = crate::TempDir::init().unwrap();
        let root = td.path().join(std::ffi::OsStr::from_bytes(b"r\xfeoot"));
        std::fs::create_dir_all(root.join("share/terminfo/x")).unwrap();
        std::fs::write(root.join("share/terminfo/x/xterm-ghostty"), b"").unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let r = resources_dir_impl(true, Some(&root.join("bin/ghostty")), None);
        assert_eq!(r.app_os().unwrap(), root.join("share/ghostty"));
    }
}
