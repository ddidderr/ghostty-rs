// Generated from the Zig multiline strings of extra/sublime.zig. Do not edit.
pub const C0: &str = r####"%YAML 1.2
---
# See http://www.sublimetext.com/docs/syntax.html
name: Ghostty Config
file_extensions:
  - ghostty
scope: source.ghostty

contexts:
  main:
    # Comments
    - match: '^\s*#.*$'
      scope: comment.line.number-sign.ghostty

    # Keywords
    - match: '\b("####;
pub const C1: &str = r####")\b'
      scope: keyword.other.ghostty
"####;
