//! Bash completions (port of `extra/bash.zig`).

use std::fmt::Write as _;

use ghostty_config::cli_args::FieldKind;

use crate::chunks_bash::{C0, C1, C2, C3, C4, C5, C6};
use crate::{actions, config_fields, join_names, join_packed};

const COMPGEN_PREFIX: &str = "_compreply compgen -W \"";
const COMPGEN_SUFFIX: &str = "\" -- \"$cur\"; _add_spaces ;;";

fn is_flag(kind: FieldKind) -> bool {
    matches!(kind, FieldKind::Bool | FieldKind::OptionalBool)
}

#[must_use]
pub fn completions() -> String {
    let pad1 = "  ";
    let pad2 = "    ";
    let pad3 = "      ";
    let pad4 = "        ";
    let pad5 = "          ";
    let mut w = String::new();
    let fields = config_fields();

    w.push_str(C0);
    for f in &fields {
        if is_flag(f.kind) {
            let _ = writeln!(w, "{pad2}config+=\" '--{} '\"", f.name);
        } else {
            let _ = writeln!(w, "{pad2}config+=\" --{}=\"", f.name);
        }
    }

    w.push_str(C1);
    for f in &fields {
        let _ = write!(w, "{pad3}--{}) ", f.name);
        if f.name.starts_with("font-family") {
            w.push_str("_fonts ;;");
        } else if f.name == "theme" {
            w.push_str("_themes ;;");
        } else if f.name == "working-directory" {
            w.push_str("_dirs ;;");
        } else if f.kind == FieldKind::RepeatablePath {
            w.push_str("_files ;;");
        } else {
            match f.kind {
                FieldKind::Enum(names) => {
                    let _ = write!(w, "{COMPGEN_PREFIX}{}{COMPGEN_SUFFIX}", join_names(names));
                }
                FieldKind::Packed(names) => {
                    let _ = write!(w, "{COMPGEN_PREFIX}{}{COMPGEN_SUFFIX}", join_packed(names));
                }
                _ => w.push_str("return ;;"),
            }
        }
        w.push('\n');
    }

    w.push_str(C2);
    let acts = actions();
    for a in acts {
        let opts = a.option_fields();
        if opts.is_empty() {
            continue;
        }
        let bash_name = a.name().replace('-', "_");
        let _ = write!(w, "{pad2}local {bash_name}=\"");
        for (count, (name, kind)) in opts.iter().enumerate() {
            if count > 0 {
                w.push(' ');
            }
            if is_flag(*kind) {
                let _ = write!(w, "'--{name} '");
            } else {
                let _ = write!(w, "--{name}=");
            }
        }
        w.push_str(" --help\"\n");
    }

    w.push_str(C3);
    for a in acts {
        let opts = a.option_fields();
        if opts.is_empty() {
            continue;
        }
        let bash_name = a.name().replace('-', "_");
        let _ = writeln!(w, "{pad3}+{})", a.name());
        let _ = writeln!(w, "{pad4}case $prev in");
        for (name, kind) in &opts {
            let _ = write!(w, "{pad5}--{name}) ");
            match kind {
                FieldKind::Bool => w.push_str("return ;;"),
                FieldKind::Enum(names) | FieldKind::OptionalEnum(names) => {
                    let _ = write!(w, "{COMPGEN_PREFIX}{}{COMPGEN_SUFFIX}", join_names(names));
                }
                FieldKind::OptionalBool | FieldKind::OptionalOther => {
                    if *name == "config-file" {
                        w.push_str("return ;;");
                    } else {
                        w.push_str("return;;");
                    }
                }
                _ => {
                    if *name == "config-file" {
                        w.push_str("_files ;;");
                    } else {
                        w.push_str("return;;");
                    }
                }
            }
            w.push('\n');
        }
        let _ = writeln!(
            w,
            "{pad5}*) _compreply compgen -W \"${bash_name}\" -- \"$cur\" ;;"
        );
        w.push_str(C4);
    }

    w.push_str(C5);
    for a in acts {
        let _ = writeln!(w, "{pad1}topLevel+=\" +{}\"", a.name());
    }
    w.push_str(C6);
    w
}
