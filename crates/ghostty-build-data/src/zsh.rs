//! Zsh completions (port of `extra/zsh.zig`).

use std::fmt::Write as _;

use ghostty_config::cli_args::FieldKind;

use crate::chunks_zsh::{C0, C1, C2, C3};
use crate::{actions, config_fields, join_names, join_packed};

const EQUALS_REQUIRED: &str = "=-:::";

#[must_use]
pub fn completions() -> String {
    let mut w = String::new();
    w.push_str(C0);
    w.push_str("_config() {\n");
    w.push_str("  _arguments \\\n");
    w.push_str("    \"--help\" \\\n");
    w.push_str("    \"--version\" \\\n");
    for f in config_fields() {
        let _ = write!(w, "    \"--{}", f.name);
        if f.name.starts_with("font-family") {
            let _ = write!(w, "{EQUALS_REQUIRED}_fonts");
        } else if f.name == "theme" {
            let _ = write!(w, "{EQUALS_REQUIRED}_themes");
        } else if f.name == "working-directory" {
            let _ = write!(w, "{EQUALS_REQUIRED}{{_files -/}}");
        } else if f.kind == FieldKind::RepeatablePath {
            let _ = write!(w, "{EQUALS_REQUIRED}_files");
        } else {
            match f.kind {
                FieldKind::Bool => {}
                FieldKind::Enum(names) => {
                    let _ = write!(w, "{EQUALS_REQUIRED}({})", join_names(names));
                }
                FieldKind::Packed(names) => {
                    let _ = write!(w, "{EQUALS_REQUIRED}({})", join_packed(names));
                }
                _ => {
                    let _ = write!(w, "{EQUALS_REQUIRED}( )");
                }
            }
        }
        w.push_str("\" \\\n");
    }
    w.push_str("\n}\n\n");

    w.push_str(C1);
    let padding = "        ";
    for a in actions() {
        let _ = writeln!(w, "{padding}'+{}'", a.name());
    }
    w.push_str(C2);

    for a in actions() {
        let opts = a.option_fields();
        if opts.is_empty() {
            continue;
        }
        let _ = writeln!(w, "{padding}(+{})", a.name());
        let _ = writeln!(w, "{padding}  _arguments \\");
        for (name, kind) in &opts {
            let _ = write!(w, "{padding}    '--{name}");
            match kind {
                FieldKind::Bool => {}
                FieldKind::Enum(names) => {
                    let _ = write!(w, "{EQUALS_REQUIRED}({})", join_names(names));
                }
                FieldKind::OptionalEnum(names) => {
                    let _ = write!(w, "{EQUALS_REQUIRED}({})", join_names(names));
                }
                FieldKind::OptionalBool | FieldKind::OptionalOther => {
                    w.push_str(EQUALS_REQUIRED);
                    if *name == "config-file" {
                        w.push_str("_files");
                    } else {
                        w.push_str("( )");
                    }
                }
                _ => {
                    w.push_str(EQUALS_REQUIRED);
                    if *name == "config-file" {
                        w.push_str("_files");
                    } else {
                        w.push_str("( )");
                    }
                }
            }
            w.push_str("' \\\n");
        }
        let _ = writeln!(w, "{padding};;");
    }
    w.push_str(C3);
    w
}
