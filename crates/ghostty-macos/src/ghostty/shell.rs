//! `Ghostty.Shell` (`Ghostty.Shell.swift`).

/// Characters to escape in the shell.
const ESCAPE_CHARACTERS: &str = "\\ ()[]{}<>\"'`!#$&;|*?\t";

/// Escape shell-sensitive characters in a string by prefixing each with a
/// backslash. Suitable for inserting paths/URLs into a live terminal buffer.
#[must_use]
pub fn escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if ESCAPE_CHARACTERS.contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

/// `quoteUnsafe = /[^\w@%+=:,.\/-]/`: true if `c` needs quoting.
fn is_quote_unsafe(c: char) -> bool {
    !(c.is_alphanumeric() || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-'))
}

/// Returns a shell-quoted version of the string, like Python's shlex.quote.
/// Suitable for building shell command lines that will be executed.
#[must_use]
pub fn quote(s: &str) -> String {
    if !(s.is_empty() || s.chars().any(is_quote_unsafe)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_cases() {
        let cases = [
            ("hello", "hello"),
            ("", ""),
            ("file name", "file\\ name"),
            ("a\\b", "a\\\\b"),
            ("(foo)", "\\(foo\\)"),
            ("[bar]", "\\[bar\\]"),
            ("{baz}", "\\{baz\\}"),
            ("<qux>", "\\<qux\\>"),
            ("say\"hi\"", "say\\\"hi\\\""),
            ("it's", "it\\'s"),
            ("`cmd`", "\\`cmd\\`"),
            ("wow!", "wow\\!"),
            ("#comment", "\\#comment"),
            ("$HOME", "\\$HOME"),
            ("a&b", "a\\&b"),
            ("a;b", "a\\;b"),
            ("a|b", "a\\|b"),
            ("*.txt", "\\*.txt"),
            ("file?.log", "file\\?.log"),
            ("col1\tcol2", "col1\\\tcol2"),
            ("$(echo 'hi')", "\\$\\(echo\\ \\'hi\\'\\)"),
            ("/tmp/my file (1).txt", "/tmp/my\\ file\\ \\(1\\).txt"),
        ];
        for (input, expected) in cases {
            assert_eq!(escape(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn quote_cases() {
        let cases = [
            ("", "''"),
            ("filename", "filename"),
            ("abcABC123@%_-+=:,./", "abcABC123@%_-+=:,./"),
            ("file name", "'file name'"),
            ("file$name", "'file$name'"),
            ("file!name", "'file!name'"),
            ("file\\name", "'file\\name'"),
            ("it's", "'it'\"'\"'s'"),
            ("file$'name'", "'file$'\"'\"'name'\"'\"''"),
        ];
        for (input, expected) in cases {
            assert_eq!(quote(input), expected, "input {input:?}");
        }
    }
}
