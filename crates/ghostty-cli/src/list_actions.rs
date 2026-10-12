//! `+list-actions` (ZIG/src/cli/list_actions.zig).

use std::io::Write;

use ghostty_config::config_struct;

use crate::error::Error;

config_struct! {
    pub struct Options {
        /// If `true`, print out documentation about the action associated with the
        /// keybinds.
        "docs" => docs: bool = false,
    }
}
cli_options!(Options, help);

/// The `list-actions` command is used to list all the available keybind
/// actions for Ghostty. These are distinct from the CLI Actions which can
/// be listed via `+help`
///
/// Flags:
///
///   * `--docs`: will print out the documentation for each action.
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let out = generate_plaintext(opts.docs);
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(out.as_bytes())?;
    stdout.flush()?;
    Ok(0)
}

/// `helpgen_actions.generate(.plaintext, show_docs)`.
#[must_use]
pub fn generate_plaintext(show_docs: bool) -> String {
    ghostty_input::helpgen_actions::generate_string(
        ghostty_input::helpgen_actions::Format::Plaintext,
        show_docs,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn names_only() {
        let s = generate_plaintext(false);
        assert!(s.starts_with("ignore\nunbind\ncsi\n"));
        assert!(!s.contains(':'));
    }

    #[test]
    fn docs_have_no_stray_blank_lines_between_actions() {
        let s = generate_plaintext(true);
        assert!(s.starts_with("\nignore:\n  Ignore this key combination.\n"));
        assert!(s.contains("      respectively.\n\njump_to_prompt:\n"));
        assert!(s.ends_with('\n'));
    }
}
