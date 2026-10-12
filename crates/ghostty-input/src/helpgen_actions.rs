//! This module is a help generator for keybind actions documentation.
//! It can generate documentation in different formats (plaintext for CLI,
//! markdown for website) while maintaining consistent content.
//! (ZIG/src/input/helpgen_actions.zig)
//!
//! The docs themselves live in [`ActionKey::docs`] (the Zig version reads
//! them from the build-time generated `help_strings`).

use crate::binding::ActionKey;

/// Format options for generating keybind actions documentation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Plain text output with indentation
    Plaintext,
    /// Markdown formatted output
    Markdown,
}

impl Format {
    fn format_field_name(self, w: &mut String, field_name: &str) {
        match self {
            Format::Plaintext => {
                w.push_str(field_name);
                w.push_str(":\n");
            }
            Format::Markdown => {
                w.push_str("## `");
                w.push_str(field_name);
                w.push_str("`\n");
            }
        }
    }

    fn format_doc_line(self, w: &mut String, line: &str) {
        match self {
            Format::Plaintext => {
                w.push_str("  ");
                w.push_str(line);
                w.push('\n');
            }
            Format::Markdown => {
                w.push_str(line);
                w.push('\n');
            }
        }
    }

    fn header(self) -> Option<&'static str> {
        match self {
            Format::Plaintext => None,
            Format::Markdown => Some(
                "---\n\
                 title: Keybinding Action Reference\n\
                 description: Reference of all Ghostty keybinding actions.\n\
                 editOnGithubLink: https://github.com/ghostty-org/ghostty/edit/main/src/input/Binding.zig\n\
                 ---\n\
                 \n\
                 This is a reference of all Ghostty keybinding actions.\n\
                 \n\
                 \n",
            ),
        }
    }
}

/// Generate keybind actions documentation with the specified format
pub fn generate(writer: &mut String, format: Format, show_docs: bool) {
    if let Some(header) = format.header() {
        writer.push_str(header);
    }

    let mut stream = String::new();

    for &key in ActionKey::ALL {
        let name = key.name();
        if name.starts_with('_') {
            continue;
        }

        // Write previously stored doc comment below all related actions
        // (every action has a doc comment).
        if show_docs {
            writer.push_str(&stream);
            writer.push('\n');
            stream.clear();
        }

        if show_docs {
            format.format_field_name(writer, name);
        } else {
            writer.push_str(name);
            writer.push('\n');
        }

        if show_docs {
            let mut iter = key.help_string().split('\n').peekable();
            while let Some(s) = iter.next() {
                // If it is the last line and empty, then skip it.
                if iter.peek().is_none() && s.is_empty() {
                    continue;
                }
                format.format_doc_line(&mut stream, s);
            }
        }
    }

    // Write any remaining buffered documentation
    if !stream.is_empty() {
        writer.push_str(&stream);
    }
}

/// Convenience: generate into a fresh string.
pub fn generate_string(format: Format, show_docs: bool) -> String {
    let mut s = String::new();
    generate(&mut s, format, show_docs);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_with_docs_lists_all_actions() {
        let s = generate_string(Format::Plaintext, true);
        assert!(s.starts_with("\nignore:\n"));
        assert!(s.contains("ignore:\n  Ignore this key combination.\n"));
        for &k in ActionKey::ALL {
            assert!(s.contains(&format!("{}:\n", k.name())));
        }
    }

    #[test]
    fn names_only() {
        let s = generate_string(Format::Plaintext, false);
        assert_eq!(s.lines().count(), ActionKey::ALL.len());
        assert_eq!(s.lines().next(), Some("ignore"));
    }

    #[test]
    fn markdown_header_and_field() {
        let s = generate_string(Format::Markdown, true);
        assert!(s.starts_with("---\ntitle: Keybinding Action Reference\n"));
        assert!(s.contains("## `unbind`\n"));
    }
}
