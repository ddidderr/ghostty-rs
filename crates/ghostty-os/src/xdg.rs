//! Implementation of the XDG Base Directory specification
//! (https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html)

use std::ffi::CStr;

use crate::env::getenv_not_empty;
use crate::{homedir, path_join};

#[derive(Debug, Clone, Default)]
pub struct Options<'a> {
    /// Subdirectories to join to the base. This avoids extra allocations
    /// when building up the directory. This is commonly the application.
    pub subdir: Option<&'a str>,

    /// The home directory for the user. If this is not set, we will attempt
    /// to look it up which is an expensive process. By setting this, you can
    /// avoid lookups.
    pub home: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no home directory could be determined")]
    NoHomeDir,
}

/// Get the XDG user config directory.
pub fn config(opts: &Options<'_>) -> Result<String, Error> {
    dir(opts, "XDG_CONFIG_HOME", ".config")
}

/// Get the XDG cache directory.
pub fn cache(opts: &Options<'_>) -> Result<String, Error> {
    dir(opts, "XDG_CACHE_HOME", ".cache")
}

/// Get the XDG state directory.
pub fn state(opts: &Options<'_>) -> Result<String, Error> {
    dir(opts, "XDG_STATE_HOME", ".local/state")
}

/// Unified helper to get XDG directories that follow a common pattern.
fn dir(opts: &Options<'_>, env: &str, default_subdir: &str) -> Result<String, Error> {
    // If we have a cached home dir, use that.
    if let Some(home) = opts.home {
        return Ok(path_join(&[
            home,
            default_subdir,
            opts.subdir.unwrap_or(""),
        ]));
    }

    // First check the env var.
    if let Some(env) = getenv_not_empty(env) {
        // If we have a subdir, then we join it.
        if let Some(subdir) = opts.subdir {
            return Ok(path_join(&[&env, subdir]));
        }

        return Ok(env);
    }

    // Get our home dir
    if let Ok(Some(home)) = homedir::home() {
        return Ok(path_join(&[
            &home,
            default_subdir,
            opts.subdir.unwrap_or(""),
        ]));
    }

    Err(Error::NoHomeDir)
}

/// Parses the xdg-terminal-exec specification. This expects argv[0] to
/// be "xdg-terminal-exec".
pub fn parse_terminal_exec<'a>(argv: &'a [&'a CStr]) -> Option<&'a [&'a CStr]> {
    let first = argv.first()?.to_bytes();
    let base = first.rsplit(|&b| b == b'/').next().unwrap_or(first);
    if base != b"xdg-terminal-exec" {
        return None;
    }

    // We expect at least one argument
    if argv.len() < 2 {
        return Some(&[]);
    }

    // If the first argument is "-e" we skip it.
    let start = if argv[1].to_bytes() == b"-e" { 2 } else { 1 };
    Some(&argv[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default() {
        let value = crate::testutil::with_env_lock(|| config(&Options::default()).unwrap());
        assert!(!value.is_empty());
    }

    #[test]
    fn cache_directory_paths() {
        let mock_home = "/Users/test";

        // Test base path
        let p = cache(&Options {
            home: Some(mock_home),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(p, "/Users/test/.cache");

        // Test with subdir
        let p = cache(&Options {
            home: Some(mock_home),
            subdir: Some("ghostty"),
        })
        .unwrap();
        assert_eq!(p, "/Users/test/.cache/ghostty");
    }

    type DirFn = fn(&Options<'_>) -> Result<String, Error>;
    const CASES: [(&str, DirFn, &str); 3] = [
        ("XDG_CONFIG_HOME", config, ".config"),
        ("XDG_CACHE_HOME", cache, ".cache"),
        ("XDG_STATE_HOME", state, ".local/state"),
    ];

    #[test]
    fn fallback_when_xdg_env_empty() {
        let temp_home = "/tmp/ghostty-test-home";
        for (name, func, default_subdir) in CASES {
            temp_env::with_vars([("HOME", Some(temp_home)), (name, Some(""))], || {
                let expected = path_join(&[temp_home, default_subdir]);
                let actual = func(&Options::default()).unwrap();
                assert_eq!(expected, actual);
            });
        }
    }

    #[test]
    fn fallback_when_xdg_env_empty_and_subdir() {
        let temp_home = "/tmp/ghostty-test-home";
        for (name, func, default_subdir) in CASES {
            temp_env::with_vars([("HOME", Some(temp_home)), (name, Some(""))], || {
                let expected = path_join(&[temp_home, default_subdir, "ghostty"]);
                let actual = func(&Options {
                    subdir: Some("ghostty"),
                    ..Default::default()
                })
                .unwrap();
                assert_eq!(expected, actual);
            });
        }
    }

    #[test]
    fn env_var_used_when_set() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("/x/cfg"))], || {
            assert_eq!(config(&Options::default()).unwrap(), "/x/cfg");
            let o = Options {
                subdir: Some("ghostty"),
                ..Default::default()
            };
            assert_eq!(config(&o).unwrap(), "/x/cfg/ghostty");
        });
    }

    fn c(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap()
    }

    #[test]
    fn parse_terminal_exec_cases() {
        fn run(args: &[&str]) -> Option<Vec<String>> {
            let owned: Vec<std::ffi::CString> = args.iter().map(|s| c(s)).collect();
            let refs: Vec<&CStr> = owned.iter().map(std::ffi::CString::as_c_str).collect();
            parse_terminal_exec(&refs)
                .map(|s| s.iter().map(|x| x.to_str().unwrap().to_owned()).collect())
        }

        assert_eq!(run(&["a", "b", "c"]), None);
        assert_eq!(run(&["xdg-terminal-exec"]), Some(vec![]));
        assert_eq!(
            run(&["xdg-terminal-exec", "a", "b", "c"]),
            Some(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(
            run(&["xdg-terminal-exec", "-e", "a", "b", "c"]),
            Some(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(
            run(&["xdg-terminal-exec", "a", "-e", "b", "c"]),
            Some(vec!["a".into(), "-e".into(), "b".into(), "c".into()])
        );
        assert_eq!(run(&["/usr/bin/xdg-terminal-exec"]), Some(vec![]));
    }
}
