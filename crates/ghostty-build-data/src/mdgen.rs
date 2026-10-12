//! Man page markdown generator (port of `build/mdgen/{mdgen,main_ghostty_1,
//! main_ghostty_5}.zig`). The markdown is converted to roff/html with pandoc
//! by `just docs` / `just install` (Zig: `GhosttyDocs.zig`).

use ghostty_cli::build_info;
use ghostty_input::binding::ActionKey;

use crate::config_fields;

const GHOSTTY_1_HEADER: &str = include_str!("mdgen/ghostty_1_header.md");
const GHOSTTY_1_FOOTER: &str = include_str!("mdgen/ghostty_1_footer.md");
const GHOSTTY_5_HEADER: &str = include_str!("mdgen/ghostty_5_header.md");
const GHOSTTY_5_FOOTER: &str = include_str!("mdgen/ghostty_5_footer.md");

/// Replaces `@@VERSION@@` with the build's version string.
#[must_use]
pub fn substitute(input: &str) -> String {
    input.replace("@@VERSION@@", &build_info::get().version_string)
}

/// Write a definition-list body: first line `:   `, others indented.
fn write_help(w: &mut String, help: &str) {
    for (i, line) in help.split('\n').enumerate() {
        w.push_str(if i == 0 { ":   " } else { "    " });
        w.push_str(line);
        w.push('\n');
    }
    w.push_str("\n\n");
}

pub fn gen_config(w: &mut String, cli: bool) {
    w.push_str("\n# CONFIGURATION OPTIONS\n\n");
    for f in config_fields() {
        w.push_str("**`");
        if cli {
            w.push_str("--");
        }
        w.push_str(f.name);
        w.push_str("`**\n\n");
        if !f.help.is_empty() {
            write_help(w, &f.help);
        }
    }
}

pub fn gen_actions(w: &mut String) {
    w.push_str("\n# COMMAND LINE ACTIONS\n\n");
    for &action in ghostty_cli::Action::ALL {
        match action {
            ghostty_cli::Action::Help => w.push_str("**`--help`**\n\n"),
            ghostty_cli::Action::Version => w.push_str("**`--version`**\n\n"),
            _ => {
                w.push_str("**`+");
                w.push_str(action.name());
                w.push_str("`**\n\n");
            }
        }
        write_help(w, action.help_text());
    }
}

pub fn gen_keybind_actions(w: &mut String) {
    w.push_str("\n# KEYBIND ACTIONS\n\n");
    for &key in ActionKey::ALL {
        let name = key.name();
        if name.starts_with('_') {
            continue;
        }
        w.push_str("**`");
        w.push_str(name);
        w.push_str("`**\n\n");
        write_help(w, key.help_string());
    }
}

/// `ghostty(1)` markdown.
#[must_use]
pub fn ghostty_1() -> String {
    let mut w = substitute(GHOSTTY_1_HEADER);
    gen_actions(&mut w);
    gen_config(&mut w, true);
    w.push_str(&substitute(GHOSTTY_1_FOOTER));
    w
}

/// `ghostty(5)` markdown.
#[must_use]
pub fn ghostty_5() -> String {
    let mut w = substitute(GHOSTTY_5_HEADER);
    gen_config(&mut w, false);
    gen_keybind_actions(&mut w);
    w.push_str(&substitute(GHOSTTY_5_FOOTER));
    w
}
