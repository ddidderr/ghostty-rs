//! Build-time help text generation (ZIG/src/helpgen.zig).
//!
//! Zig's `helpgen.zig` parses the sources of the CLI actions and of
//! `input/Binding.zig` and turns the `///` doc comments into help strings
//! that are compiled into the binary. This crate is the Rust counterpart:
//! it is used as a build-dependency by the `build.rs` of `ghostty-cli`
//! (action help, from the `///` docs of each `pub fn run`) and of
//! `ghostty-input` (keybind action docs, from the `///` docs of the
//! variants of `enum Action`), which write Rust source tables into
//! `OUT_DIR` that are pulled in with `include!`.
//!
//! The scanner is deliberately line based (like Zig's token based one it
//! only looks at doc comment runs and the item that follows them), so it
//! needs no parser dependency.

use std::fmt::Write as _;

/// Returns the text of a doc comment line without the `///` marker, or
/// `None` if the line is not a doc comment (`////` is a plain comment in
/// Rust but is accepted by Zig; we follow Rust here).
fn strip_marker(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("///")?;
    if rest.starts_with('/') {
        return None;
    }
    Some(rest)
}

/// Port of `extractDocComments` + `findCommonPrefix`: removes the `///`
/// markers, then removes the common number of leading spaces of all
/// non-blank lines. Returns the lines (without line terminators).
///
/// `comment_lines` are the raw source lines of one contiguous doc comment
/// run. Lines that are not doc comments are ignored.
#[must_use]
pub fn doc_lines(comment_lines: &[&str]) -> Vec<String> {
    let lines: Vec<&str> = comment_lines
        .iter()
        .filter_map(|l| strip_marker(l))
        .collect();
    // Blank lines don't participate in the common prefix.
    let prefix = lines
        .iter()
        .filter_map(|l| l.find(|c| c != ' '))
        .min()
        .unwrap_or(usize::MAX);
    lines
        .iter()
        .map(|l| l[prefix.min(l.len())..].to_string())
        .collect()
}

/// The doc comment lines directly above line `idx` (attributes such as
/// `#[cfg(..)]` between the comment and the item are skipped), in source
/// order. Empty if there is no doc comment.
fn doc_run_above<'a>(lines: &[&'a str], idx: usize) -> Vec<&'a str> {
    let mut i = idx;
    while i > 0 && lines[i - 1].trim_start().starts_with("#[") {
        i -= 1;
    }
    let end = i;
    while i > 0 && strip_marker(lines[i - 1]).is_some() {
        i -= 1;
    }
    lines[i..end].to_vec()
}

/// The doc comment of the `run` function of a CLI action source file
/// (Zig: the doc comment preceding `fn run`), joined with `\n` and with
/// no trailing newline. `None` if there is no `run` function or it has no
/// doc comment.
#[must_use]
pub fn run_fn_doc(source: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let idx = lines.iter().position(|l| {
        let l = l.trim_start();
        l.strip_prefix("pub fn run")
            .or_else(|| l.strip_prefix("fn run"))
            .is_some_and(|r| r.starts_with(['(', '<']))
    })?;
    let run = doc_run_above(&lines, idx);
    if run.is_empty() {
        return None;
    }
    Some(doc_lines(&run).join("\n"))
}

/// The doc comments of the variants of `enum <name>` in `source`, as
/// `(variant, doc)` in declaration order. `doc` is the doc lines joined
/// with `\n` plus one trailing newline (Zig's multiline literal plus the
/// newline the generated table always carried). Variants without a doc
/// comment are returned with `None`.
#[must_use]
pub fn enum_variant_docs(source: &str, name: &str) -> Vec<(String, Option<String>)> {
    let lines: Vec<&str> = source.lines().collect();
    let header = format!("enum {name} {{");
    let Some(start) = lines
        .iter()
        .position(|l| (l.starts_with("pub ") && l.contains(&header)) || *l == header)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if *line == "}" {
            break;
        }
        // Variants are exactly one indentation level deep.
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let run = doc_run_above(&lines, i);
        let doc = (!run.is_empty()).then(|| {
            let mut d = doc_lines(&run).join("\n");
            d.push('\n');
            d
        });
        out.push((ident, doc));
    }
    out
}

/// Generates the body of `ActionKey::docs`: a `match self` expression over
/// the given `(variant, doc)` pairs.
#[must_use]
pub fn keybind_docs_match(variants: &[(String, String)]) -> String {
    let mut s = String::from("match self {\n");
    for (v, d) in variants {
        let _ = writeln!(s, "    ActionKey::{v} => {d:?},");
    }
    s.push_str("}\n");
    s
}

/// Generates the body of `Action::help_text`: a `match self` expression
/// over `(variant, help)` pairs.
#[must_use]
pub fn cli_help_match(actions: &[(String, String)]) -> String {
    let mut s = String::from("match self {\n");
    for (v, d) in actions {
        let _ = writeln!(s, "    Action::{v} => {d:?},");
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_marker_and_common_prefix() {
        let src = ["/// One", "///   two", "    /// three"];
        assert_eq!(doc_lines(&src), ["One", "  two", "three"]);
    }

    #[test]
    fn blank_lines_do_not_affect_prefix() {
        let src = ["///", "/// a", "///", "///  b", "/// "];
        assert_eq!(doc_lines(&src), ["", "a", "", " b", ""]);
    }

    #[test]
    fn code_fences_and_indented_blocks_are_preserved() {
        let src = ["/// ```", "///   indented", "/// ```"];
        assert_eq!(doc_lines(&src), ["```", "  indented", "```"]);
    }

    #[test]
    fn four_slashes_is_not_a_doc_comment() {
        assert_eq!(doc_lines(&["//// nope", "// plain"]), Vec::<String>::new());
    }

    #[test]
    fn run_doc_is_extracted() {
        let src =
            "use x;\n\n/// First.\n///\n/// Second.\n#[must_use]\npub fn run(a: &[String]) {}\n";
        assert_eq!(run_fn_doc(src).as_deref(), Some("First.\n\nSecond."));
    }

    #[test]
    fn run_doc_ignores_other_comments_and_missing() {
        assert_eq!(run_fn_doc("// x\npub fn run() {}"), None);
        assert_eq!(run_fn_doc("/// a\npub fn runner() {}"), None);
        assert_eq!(run_fn_doc("fn other() {}"), None);
    }

    #[test]
    fn enum_variants_are_extracted() {
        let src = "pub enum Action {\n    /// Ignore.\n    ///\n    /// More.\n    Ignore,\n    Undoc,\n    /// Send.\n    ///\n    Csi(String),\n    /// Multi\n    Multi {\n        a: u8,\n    },\n}\nenum Other {\n    /// no\n    X,\n}\n";
        let v = enum_variant_docs(src, "Action");
        assert_eq!(
            v,
            [
                ("Ignore".into(), Some("Ignore.\n\nMore.\n".into())),
                ("Undoc".into(), None),
                ("Csi".into(), Some("Send.\n\n".into())),
                ("Multi".into(), Some("Multi\n".into())),
            ]
        );
    }

    #[test]
    fn generated_match_is_escaped() {
        let m = keybind_docs_match(&[("A".into(), "x \"q\"\n".into())]);
        assert_eq!(
            m,
            "match self {\n    ActionKey::A => \"x \\\"q\\\"\\n\",\n}\n"
        );
    }
}
