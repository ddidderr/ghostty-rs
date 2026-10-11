//! `+list-actions` (ZIG/src/cli/list_actions.zig).

use std::io::Write;

use ghostty_config::config_struct;
use ghostty_input::binding::ActionKey;

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
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let out = generate_plaintext(opts.docs);
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(out.as_bytes())?;
    stdout.flush()?;
    Ok(0)
}

/// `helpgen_actions.generate(.plaintext, show_docs)`.
///
/// TODO(integration): this is a local copy of
/// `ghostty_input::helpgen_actions::generate`. The embedded docs of a few
/// actions in ghostty-input (`adjust_selection`, ...) end with an extra blank
/// line compared to Ghostty's `help_strings`, which adds a stray `"  "` line
/// to `--docs` output. Trailing newlines are normalised here; once the data
/// is fixed in ghostty-input this should call `helpgen_actions::generate`.
#[must_use]
pub fn generate_plaintext(show_docs: bool) -> String {
    let mut writer = String::new();
    let mut stream = String::new();
    for &key in ActionKey::ALL {
        let name = key.name();
        if name.starts_with('_') {
            continue;
        }

        // Write previously stored doc comment below all related actions
        if show_docs {
            writer.push_str(&stream);
            writer.push('\n');
            stream.clear();
            writer.push_str(name);
            writer.push_str(":\n");
            for line in key.docs().trim_end_matches('\n').split('\n') {
                stream.push_str("  ");
                stream.push_str(line);
                stream.push('\n');
            }
        } else {
            writer.push_str(name);
            writer.push('\n');
        }
    }

    // Write any remaining buffered documentation
    writer.push_str(&stream);
    writer
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

    #[test]
    fn matches_input_crate_when_docs_are_clean() {
        // The first actions have well-formed docs.
        let ours = generate_plaintext(true);
        let theirs = ghostty_input::helpgen_actions::generate_string(
            ghostty_input::helpgen_actions::Format::Plaintext,
            true,
        );
        assert!(ours.lines().take(100).eq(theirs.lines().take(100)));
    }
}
