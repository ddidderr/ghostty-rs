//! `+validate-config` (ZIG/src/cli/validate_config.zig).

use std::io::Write;

use ghostty_config::{config::Config, config_struct};
use ghostty_os::osbytes::EncodedOs;

use crate::error::Error;

config_struct! {
    pub struct Options {
        /// The path of the config file to validate. If this isn't specified,
        /// then the default config file paths will be validated.
        "config-file" => config_file: Option<String> = None,
    }
}
cli_options!(Options, help);

/// The `validate-config` command is used to validate a Ghostty config file.
///
/// When executed without any arguments, this will load the config from the default
/// location.
///
/// Flags:
///
///   * `--config-file`: can be passed to validate a specific target config file in
///     a non-default location
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    let result = run_inner(&opts, &mut stdout);
    stdout.flush()?;
    result
}

fn run_inner(opts: &Options, stdout: &mut dyn Write) -> Result<u8, Error> {
    let mut cfg = Config::new_default();

    // If a config path is passed, validate it, otherwise validate default configs
    if let Some(config_path) = &opts.config_file {
        // Byte-preserving: the option string carries non-UTF-8 bytes via the
        // reversible `osbytes` escape and the canonical path may too.
        let abs_path = std::fs::canonicalize(EncodedOs::from_encoded(config_path).to_path())?;
        cfg.load_file(&EncodedOs::from_path(&abs_path))?;
        cfg.load_recursive_files()?;
    } else {
        cfg = Config::load()?;
    }

    cfg.finalize()?;

    if !cfg.diagnostics.items().is_empty() {
        for diag in cfg.diagnostics.items() {
            // Diagnostics embed config values in the reversible byte
            // encoding; print the original bytes.
            stdout.write_all(&ghostty_os::osbytes::config_str_to_bytes(&format!(
                "{diag}\n"
            )))?;
        }
        return Ok(1);
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn write_tmp(name: &str, content: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("ghostty-cli-vc-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config");
        std::fs::write(&p, content).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn validate_good_file() {
        let p = write_tmp("good", "font-size = 14\n");
        let opts = Options {
            config_file: Some(p),
        };
        let mut out = Vec::new();
        assert_eq!(0, run_inner(&opts, &mut out).unwrap());
        assert_eq!(0, out.len());
    }

    #[test]
    fn validate_bad_file_prints_diagnostics() {
        let p = write_tmp("bad", "not-a-key = 1\nfont-size = abc\n");
        let opts = Options {
            config_file: Some(p.clone()),
        };
        let mut out = Vec::new();
        assert_eq!(1, run_inner(&opts, &mut out).unwrap());
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(&format!("{p}:1:not-a-key: unknown field\n")),
            "{s}"
        );
        assert!(
            s.contains(&format!("{p}:2:font-size: invalid value \"abc\"\n")),
            "{s}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn diagnostics_print_original_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config");
        std::fs::write(&path, b"bad\xffkey = 1\n").unwrap();
        let opts = Options {
            config_file: Some(path.to_string_lossy().into_owned()),
        };
        let mut out = Vec::new();
        assert_eq!(1, run_inner(&opts, &mut out).unwrap());
        assert!(
            out.windows(7).any(|w| w == b"bad\xffkey"),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
        assert!(!out.contains(&0xef), "no escape encoding in the output");
        let _ = path.as_os_str().as_bytes();
    }

    #[test]
    fn validate_missing_file_is_error() {
        let opts = Options {
            config_file: Some("/nonexistent/ghostty/config".into()),
        };
        assert!(run_inner(&opts, &mut Vec::new()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn validate_symlink_to_non_utf8_target() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join(std::ffi::OsStr::from_bytes(b"cfg\xff"));
        std::fs::write(&target, "font-size = 14\n").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let opts = Options {
            config_file: Some(link.to_string_lossy().into_owned()),
        };
        let mut out = Vec::new();
        assert_eq!(0, run_inner(&opts, &mut out).unwrap());
        assert_eq!(out.len(), 0);
    }
}
