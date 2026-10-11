//! `+show-config` (ZIG/src/cli/show_config.zig).

use std::io::Write;

use ghostty_config::config::Config;
use ghostty_config::config_struct;
use ghostty_config::file_formatter::FileFormatter;

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

    let text = format_config(&config, &opts)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
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
