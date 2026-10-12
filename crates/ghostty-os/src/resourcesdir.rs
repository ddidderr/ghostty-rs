use std::io;
use std::path::Path;

use crate::path_join;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcesDir {
    /// Avoid accessing these directly, use the app() and host() methods instead.
    pub app_path: Option<String>,
    pub host_path: Option<String>,
}

impl ResourcesDir {
    /// Get the directory to the bundled resources directory accessible
    /// by the application.
    pub fn app(&self) -> Option<&str> {
        self.app_path.as_deref()
    }

    /// Get the directory to the bundled resources directory accessible
    /// by the host environment (i.e. for sandboxed applications). The
    /// returned directory might not be accessible from the application
    /// itself.
    ///
    /// In non-sandboxed environment, this should be the same as app().
    pub fn host(&self) -> Option<&str> {
        self.host_path.as_deref().or(self.app_path.as_deref())
    }
}

/// Sentinels we look for in the path to know we've found the resources
/// directory (Linux).
const SENTINELS: [&str; 2] = ["terminfo/g/ghostty", "terminfo/x/xterm-ghostty"];

/// Gets the directory to the bundled resources directory, if it
/// exists (not all platforms or packages have it).
///
/// Zig's `builtin.mode == .Debug` is mapped to `cfg!(debug_assertions)`.
pub fn resources_dir() -> io::Result<ResourcesDir> {
    let exe = std::env::current_exe().ok();
    let env = std::env::var("GHOSTTY_RESOURCES_DIR").ok();
    Ok(resources_dir_impl(
        cfg!(debug_assertions),
        exe.as_deref(),
        env,
    ))
}

fn resources_dir_impl(debug: bool, exe: Option<&Path>, env: Option<String>) -> ResourcesDir {
    // Use the GHOSTTY_RESOURCES_DIR environment variable in release builds.
    //
    // In debug builds we try using terminfo detection first instead, since
    // if debug Ghostty is launched by an older version of Ghostty, it
    // would inherit the old, stale resources of older Ghostty instead of the
    // freshly built ones under zig-out/share/ghostty.
    if !debug && let Some(dir) = env.as_ref().filter(|d| !d.is_empty()) {
        return ResourcesDir {
            app_path: Some(dir.clone()),
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
        let dir_str = dir.to_string_lossy();

        // We look for a /usr/share style path.
        for sentinel in SENTINELS {
            if let Some(v) = maybe_dir(&dir_str, "share", sentinel) {
                return ResourcesDir {
                    app_path: Some(path_join(&[&v, "ghostty"])),
                    host_path: None,
                };
            }
        }
    }

    // If terminfo detection failed in debug builds (somehow),
    // fallback and use the provided resources dir.
    if debug && let Some(dir) = env.filter(|d| !d.is_empty()) {
        return ResourcesDir {
            app_path: Some(dir),
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
            app_path: Some(dir),
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
fn dev_resources_dir(exe: &Path) -> Option<String> {
    let root = workspace_root().canonicalize().ok()?;
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    if !exe.starts_with(&root) {
        return None;
    }
    let res = root.join("resources");
    SENTINELS
        .iter()
        .any(|s| res.join(s).exists())
        .then(|| res.to_string_lossy().into_owned())
}

/// Little helper to check if the "base/sub/suffix" directory exists and
/// if so return "base/sub". The "suffix" is just used as a way to verify a
/// directory seems roughly right.
pub fn maybe_dir(base: &str, sub: &str, suffix: &str) -> Option<String> {
    let path = format!("{base}/{sub}/{suffix}");

    // Folder doesn't exist. If a different error happens its okay
    // we just ignore it and move on.
    if Path::new(&path).exists() {
        let len = path.len() - suffix.len() - 1;
        return Some(path[..len].to_owned());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_dir_accessors() {
        let mut r = ResourcesDir::default();
        assert_eq!(r.app(), None);
        assert_eq!(r.host(), None);
        r.app_path = Some("/a".into());
        assert_eq!(r.host(), Some("/a"));
        r.host_path = Some("/h".into());
        assert_eq!(r.app(), Some("/a"));
        assert_eq!(r.host(), Some("/h"));
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
        assert_eq!(r.app(), Some(expected.as_str()));
    }

    #[test]
    fn env_in_release_wins_and_in_debug_is_fallback() {
        let td = make_tree();
        let exe = td.path().join("bin/ghostty");
        let env = Some("/env/res".to_owned());
        assert_eq!(
            resources_dir_impl(false, Some(&exe), env.clone()).app(),
            Some("/env/res")
        );
        assert_ne!(
            resources_dir_impl(true, Some(&exe), env.clone()).app(),
            Some("/env/res")
        );
        // Debug fallback when detection fails.
        assert_eq!(
            resources_dir_impl(true, Some(Path::new("/nonexistent/bin/x")), env).app(),
            Some("/env/res")
        );
        // Empty env is ignored.
        assert_eq!(
            resources_dir_impl(false, None, Some(String::new())).app(),
            None
        );
    }

    #[test]
    fn dev_fallback_finds_workspace_resources() {
        let exe = workspace_root().join("target/debug/ghostty");
        let r = resources_dir_impl(true, Some(&exe), None);
        assert!(r.app().is_some_and(|d| d.ends_with("/resources")));
        // Outside the workspace there is no fallback.
        let r = resources_dir_impl(true, Some(Path::new("/nonexistent/bin/x")), None);
        assert_eq!(r.app(), None);
        // Release builds never use it.
        let r = resources_dir_impl(false, Some(&exe), None);
        assert_eq!(r.app(), None);
    }

    #[test]
    fn maybe_dir_checks_suffix() {
        let td = make_tree();
        let base = td.path().to_string_lossy().into_owned();
        assert_eq!(
            maybe_dir(&base, "share", "terminfo/x/xterm-ghostty"),
            Some(format!("{base}/share"))
        );
        assert_eq!(maybe_dir(&base, "share", "terminfo/g/ghostty"), None);
    }
}
