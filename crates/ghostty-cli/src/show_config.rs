//! `+show-config` (ZIG/src/cli/show_config.zig).

use std::io::Write;

use ghostty_config::{config::Config, config_struct, file_formatter::FileFormatter};

use crate::error::Error;

config_struct! {
    pub struct Options {
        /// If true, do not load the user configuration, only load the defaults.
        "default" => default: bool = false,

        /// Only show the options that have been changed from the default.
        /// This has no effect if `--default` is specified.
        "changes-only" => changes_only: bool = true,

        /// If true print the documentation above each option as a comment,
        /// if available.
        "docs" => docs: bool = false,
    }
}
cli_options!(Options, help);

/// The `show-config` command shows the current configuration in a valid Ghostty
/// configuration file format.
///
/// When executed without any arguments this will output the current
/// configuration that is different from the default configuration. If you're
/// using the default configuration this will output nothing.
///
/// If you are a new user and want to see all available options with
/// documentation, run `ghostty +show-config --default --docs`.
///
/// The output is not in any specific order, but the order should be consistent
/// between runs. The output is not guaranteed to be exactly match the input
/// configuration files, but it will result in the same behavior. Comments,
/// whitespace, and other formatting is not preserved from user configuration
/// files.
///
/// Flags:
///
///   * `--default`: Show the default configuration instead of loading
///     the user configuration.
///
///   * `--changes-only`: Only show the options that have been changed
///     from the default. This has no effect if `--default` is specified.
///
///   * `--docs`: Print the documentation above each option as a comment,
///     This is very noisy but is very useful to learn about available
///     options, especially paired with `--default`.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let config = if opts.default {
        Config::new_default()
    } else {
        Config::load()?
    };

    let text = format_config_bytes(&config, &opts)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&text)?;
    stdout.flush()?;
    Ok(0)
}

/// Format `config` according to `opts`.
pub fn format_config(config: &Config, opts: &Options) -> Result<String, Error> {
    let fmt = FileFormatter {
        config,
        changed: !opts.default && opts.changes_only,
        docs: opts.docs,
    };
    let mut out = String::new();
    fmt.format(&mut out)?;
    Ok(out)
}

/// Like [`format_config`] but the original bytes: the config layer holds OS
/// data in the reversible escape encoding, and Zig writes the raw bytes, so
/// export + reload reproduces the same value. Plain UTF-8 output is
/// unchanged.
pub fn format_config_bytes(config: &Config, opts: &Options) -> Result<Vec<u8>, Error> {
    Ok(ghostty_os::osbytes::config_str_to_bytes(&format_config(
        config, opts,
    )?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn default_changes_only_is_empty() {
        let config = Config::new_default();
        let opts = Options::default();
        assert_eq!("", format_config(&config, &opts).unwrap());
    }

    #[test]
    fn default_docs_has_comments() {
        let config = Config::new_default();
        let opts = Options {
            default: true,
            docs: true,
            ..Options::default()
        };
        let s = format_config(&config, &opts).unwrap();
        assert!(s.contains("# The font families to use."));
        assert!(s.contains("\nfont-size = 12\n"));
    }

    #[cfg(unix)]
    #[test]
    fn show_config_round_trips_non_utf8_and_fdd0_paths() {
        use ghostty_os::osbytes::EncodedOs;
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.conf");
        let raw: &[u8] = b"custom-shader = /tmp/a\xff\xef\xb7\x90b\n";
        std::fs::write(&first, raw).unwrap();
        let mut cfg = Config::new_default();
        cfg.load_file(&EncodedOs::from_path(&first)).unwrap();
        let opts = Options::default();
        let out = format_config_bytes(&cfg, &opts).unwrap();
        assert_eq!(out, raw);

        // Reload the exported bytes: same value.
        let second = dir.path().join("b.conf");
        std::fs::write(&second, &out).unwrap();
        let mut cfg2 = Config::new_default();
        cfg2.load_file(&EncodedOs::from_path(&second)).unwrap();
        assert_eq!(cfg.custom_shader, cfg2.custom_shader);
        assert_eq!(format_config_bytes(&cfg2, &opts).unwrap(), raw);
    }

    #[test]
    fn options_parse() {
        let mut opts = Options::default();
        let args: Vec<String> = [
            "ghostty",
            "+show-config",
            "--default",
            "--changes-only=false",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        crate::parse_options(&mut opts, &args).unwrap();
        assert!(opts.default);
        assert!(!opts.changes_only);
        assert!(matches!(
            crate::parse_options(&mut opts, &["g".into(), "--help".into()]),
            Err(Error::ActionHelpRequested)
        ));
    }
}
