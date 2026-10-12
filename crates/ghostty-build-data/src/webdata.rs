//! Website data generators (port of `build/GhosttyWebdata.zig` and
//! `build/webgen/main_{config,actions,commands}.zig`). They produce the
//! `config.mdx`, `actions.mdx` (keybind actions) and `commands.mdx` (CLI
//! actions) files installed to `share/ghostty/webdata/`.

use ghostty_input::binding::ActionKey;

use crate::config_fields;

/// `webgen/main_config.zig`: the configuration reference.
#[must_use]
pub fn config_mdx() -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Block {
        Text,
        Code,
        CalloutNote,
        CalloutWarning,
    }

    fn end_block(w: &mut String, block: Option<Block>) {
        match block {
            Some(Block::Code) => w.push_str("```\n"),
            Some(Block::CalloutNote) => w.push_str("</Note>\n"),
            Some(Block::CalloutWarning) => w.push_str("</Warning>\n"),
            Some(Block::Text) | None => {}
        }
    }

    fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
        s.len() >= prefix.len()
            && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    }

    let mut w = String::from(
        "---\n\
title: Reference\n\
description: Reference of all Ghostty configuration options.\n\
editOnGithubLink: https://github.com/ghostty-org/ghostty/edit/main/src/config/Config.zig\n\
---\n\
\n\
This is a reference of all Ghostty configuration options. These\n\
options are ordered roughly by how common they are to be used\n\
and grouped with related options. I recommend utilizing your\n\
browser's search functionality to find the option you're looking\n\
for.\n\
\n\
In the future, we'll have a more user-friendly way to view and\n\
organize these options.\n\
\n",
    );

    let fields = config_fields();
    for (i, field) in fields.iter().enumerate() {
        if field.help.is_empty() {
            continue;
        }
        w.push_str("## `");
        w.push_str(field.name);
        w.push_str("`\n");

        // For all subsequent fields with no docs, they are grouped with the
        // previous field. (Zig breaks at `_` fields; those are already
        // filtered out of `config_fields`, which would otherwise leave a
        // gap, so the group may only extend to the next raw field.)
        for next in &fields[i + 1..] {
            if !next.help.is_empty() {
                break;
            }
            w.push_str("## `");
            w.push_str(next.name);
            w.push_str("`\n");
        }

        w.push('\n');

        let mut block: Option<Block> = None;
        for s in field.help.split('\n') {
            // Empty line resets our block.
            if s.is_empty() {
                end_block(&mut w, block);
                block = None;
                w.push('\n');
                continue;
            }

            let first = block.is_none();
            if block.is_none() {
                if s.starts_with("    ") {
                    block = Some(Block::Code);
                    w.push_str("```\n");
                } else if starts_with_ignore_case(s, "note:") {
                    block = Some(Block::CalloutNote);
                    w.push_str("<Note>\n");
                } else if starts_with_ignore_case(s, "warning:") {
                    block = Some(Block::CalloutWarning);
                    w.push_str("<Warning>\n");
                } else {
                    block = Some(Block::Text);
                }
            }

            w.push_str(match block {
                Some(Block::CalloutNote) if first => &s["note:".len()..],
                Some(Block::CalloutWarning) if first => &s["warning:".len()..],
                Some(Block::Code) => s.strip_prefix("    ").unwrap_or(s),
                _ => s,
            });
            w.push('\n');
        }
        end_block(&mut w, block);
        w.push('\n');
    }
    w
}

/// `webgen/main_actions.zig` (`input/helpgen_actions.zig`, markdown with
/// docs): the keybind action reference.
#[must_use]
pub fn actions_mdx() -> String {
    let mut w = String::from(
        "---\n\
title: Keybinding Action Reference\n\
description: Reference of all Ghostty keybinding actions.\n\
editOnGithubLink: https://github.com/ghostty-org/ghostty/edit/main/src/input/Binding.zig\n\
---\n\
\n\
This is a reference of all Ghostty keybinding actions.\n\
\n",
    );

    // Doc comments are written below all related (consecutive undocumented)
    // actions, so they are buffered until the next documented action.
    let mut stream = String::new();
    for &key in ActionKey::ALL {
        let name = key.name();
        if name.starts_with('_') {
            continue;
        }
        let docs = key.help_string();
        let has_docs = !key.docs().is_empty();

        if has_docs {
            w.push_str(&stream);
            w.push('\n');
            stream.clear();
        }

        w.push_str("## `");
        w.push_str(name);
        w.push_str("`\n");

        if has_docs {
            let mut iter = docs.split('\n').peekable();
            while let Some(s) = iter.next() {
                // If it is the last line and empty, then skip it.
                if iter.peek().is_none() && s.is_empty() {
                    continue;
                }
                stream.push_str(s);
                stream.push('\n');
            }
        }
    }
    if !stream.is_empty() {
        w.push_str(&stream);
    }
    w
}

/// `webgen/main_commands.zig`: the CLI action reference.
#[must_use]
pub fn commands_mdx() -> String {
    let mut w = String::from(
        "---\n\
title: Reference\n\
description: Reference of all Ghostty action subcommands.\n\
editOnGithubLink: https://github.com/ghostty-org/ghostty/tree/main/src/cli\n\
---\n\
Ghostty includes a number of utility actions that can be accessed as subcommands.\n\
Actions provide utilities to work with config, list keybinds, list fonts, demo themes,\n\
and debug.\n",
    );

    for &action in ghostty_cli::Action::ALL {
        let name = action.name();
        w.push_str("## ");
        w.push_str(name);
        w.push('\n');

        let help = action.help_text();
        if help.is_empty() {
            continue;
        }
        for s in help.split('\n') {
            w.push_str(s);
            w.push('\n');
        }
        w.push_str("\n```\n");
        match action {
            ghostty_cli::Action::Help | ghostty_cli::Action::Version => {
                w.push_str("ghostty --");
            }
            _ => w.push_str("ghostty +"),
        }
        w.push_str(name);
        w.push_str("\n```\n\n");
    }
    w
}
