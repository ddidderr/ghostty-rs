//! `+help` (ZIG/src/cli/help.zig).

use std::io::Write;

use ghostty_config::config_struct;

use crate::error::Error;
use crate::ghostty::Action;

config_struct! {
    // Note that this options struct doesn't implement the `help` decl like other
    // actions. That is because the help command is special and wants to handle its
    // own logic around help detection.
    pub struct Options {
        /// This must be registered so that it isn't an error to pass `--help`
        "help" => help: bool = false,
    }
}
cli_options!(Options);

/// The `help` command shows general help about Ghostty. Recognized as either
/// `-h, `--help`, or like other actions `+help`.
///
/// You can also specify `--help` or `-h` along with any action such as
/// `+list-themes` to see help for a specific action.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    write_help(&mut out)?;
    out.flush()?;
    Ok(0)
}

/// Write the general help text.
pub fn write_help(stdout: &mut dyn Write) -> std::io::Result<()> {
    stdout.write_all(
        b"Usage: ghostty [+action] [options]

Run the Ghostty terminal emulator or a specific helper action.

If no `+action` is specified, run the Ghostty terminal emulator.
All configuration keys are available as command line options.
To specify a configuration key, use the `--<key>=<value>` syntax
where key and value are the same format you'd put into a configuration
file. For example, `--font-size=12` or `--font-family=\"Fira Code\"`.

To see a list of all available configuration options, please see
the `src/config/Config.zig` file. A future update will allow seeing
the list of configuration options from the command line.

A special command line argument `-e <command>` can be used to run
the specific command inside the terminal emulator. For example,
`ghostty -e top` will run the `top` command inside the terminal.

On macOS, launching the terminal emulator from the CLI is not
supported and only actions are supported. Use `open -na Ghostty.app`
instead, or `open -na ghostty.app --args --foo=bar --baz=quz` to pass
arguments.

Available actions:

",
    )?;

    for action in Action::ALL {
        writeln!(stdout, "  +{}", action.name())?;
    }

    stdout.write_all(
        b"
Specify `+<action> --help` to see the help for a specific action,
where `<action>` is one of actions listed above.
",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn help_lists_actions() {
        let mut buf = Vec::new();
        write_help(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Available actions:\n\n  +version\n  +help\n  +list-fonts\n"));
        assert!(s.ends_with("where `<action>` is one of actions listed above.\n"));
    }
}
