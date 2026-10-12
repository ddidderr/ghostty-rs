//! Fish completions (port of `extra/fish.zig`).

use std::fmt::Write as _;

use ghostty_cli::Action;
use ghostty_config::cli_args::FieldKind;

use crate::{actions, config_fields, join_names, join_packed};

#[must_use]
pub fn completions() -> String {
    let mut w = String::new();
    let acts = actions();
    let plus = |sep: &str| {
        acts.iter()
            .map(|a| format!("+{}", a.name()))
            .collect::<Vec<_>>()
            .join(sep)
    };

    let _ = writeln!(w, "set -l commands \"{}\"", plus(" "));
    w.push_str("complete -c ghostty -f\n");
    w.push_str("complete -c ghostty -s e -l help -f\n");
    w.push_str(
        "complete -c ghostty -n \"not __fish_seen_subcommand_from $commands\" -l version -f\n",
    );

    for f in config_fields() {
        w.push_str("complete -c ghostty -n \"not __fish_seen_subcommand_from $commands\" -l ");
        w.push_str(f.name);
        w.push_str(if f.kind == FieldKind::Bool {
            " "
        } else {
            " -r"
        });
        if f.name.starts_with("font-family") {
            w.push_str(" -f  -a \"(ghostty +list-fonts | grep '^[A-Z]')\"");
        } else if f.name == "theme" {
            w.push_str(" -f -a \"(ghostty +list-themes | sed -E 's/^(.*) \\(.*\\$/\\1/')\"");
        } else if f.name == "working-directory" {
            w.push_str(" -f -k -a \"(__fish_complete_directories)\"");
        } else {
            w.push_str(if f.kind == FieldKind::RepeatablePath {
                " -F"
            } else {
                " -f"
            });
            match f.kind {
                FieldKind::Enum(names) => {
                    let _ = write!(w, " -a \"{}\"", join_names(names));
                }
                FieldKind::Packed(names) => {
                    let _ = write!(w, " -a \"{}\"", join_packed(names));
                }
                _ => {}
            }
        }
        if !f.help.is_empty() {
            let _ = write!(w, " -d \"{}\"", get_description(&f.help));
        }
        w.push('\n');
    }

    let _ = writeln!(
        w,
        "complete -c ghostty -n \"string match -q -- '+*' (commandline -pt)\" -f -a \"{}\"",
        plus(" ")
    );

    for a in acts {
        if matches!(a, Action::Help | Action::Version) {
            continue;
        }
        for (name, kind) in a.option_fields() {
            let _ = write!(
                w,
                "complete -c ghostty -n \"__fish_seen_subcommand_from +{}\" -l {name}",
                a.name()
            );
            if kind != FieldKind::Bool {
                w.push_str(" -r");
            }
            w.push_str(if name == "config-file" { " -F" } else { " -f" });
            match kind {
                FieldKind::Enum(names) | FieldKind::OptionalEnum(names) => {
                    let _ = write!(w, " -a \"{}\"", join_names(names));
                }
                _ => {}
            }
            w.push('\n');
        }
    }
    w
}

/// First sentence of the help text on one line, with quotes escaped
/// (Zig: `getDescription`).
#[must_use]
pub fn get_description(help: &str) -> String {
    let b = help.as_bytes();
    let mut out = Vec::with_capacity(b.len() * 2);
    let mut prev_was_space = false;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'.' => {
                out.push(b'.');
                if i + 1 >= b.len() {
                    break;
                }
                let next = b[i + 1];
                if next == b' ' || next == b'\n' {
                    break;
                }
            }
            b'\n' => {
                if !prev_was_space && !out.is_empty() {
                    out.push(b' ');
                    prev_was_space = true;
                }
            }
            b'"' => {
                out.extend_from_slice(b"\\\"");
                prev_was_space = false;
            }
            _ => {
                out.push(c);
                prev_was_space = c == b' ';
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_description_test() {
        let input = "First sentence with \"quotes\"\nand newlines. Second sentence.";
        assert_eq!(
            get_description(input),
            "First sentence with \\\"quotes\\\" and newlines."
        );
    }
}
