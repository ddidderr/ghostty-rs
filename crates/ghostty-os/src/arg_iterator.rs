//! Port of Zig 0.16 `std.process.ArgIteratorGeneral` (`process/Args.zig`).
//!
//! Splits a command line on space, tab, CR and LF. Double quotes group;
//! backslashes are only special before a double quote (or a single quote
//! when `single_quotes` is enabled). A NUL byte terminates the command line,
//! exactly like Zig's sentinel handling.

/// `ArgIteratorGeneral` options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Treat `#` at a token start as a comment up to end of line.
    pub comments: bool,
    /// Treat `'` as a quote character.
    pub single_quotes: bool,
}

/// Iterator over the arguments of a command line.
#[derive(Debug, Clone)]
pub struct ArgIteratorGeneral<'a> {
    cmd_line: &'a [u8],
    index: usize,
    options: Options,
}

impl<'a> ArgIteratorGeneral<'a> {
    #[must_use]
    pub fn new(options: Options, cmd_line: &'a [u8]) -> Self {
        ArgIteratorGeneral {
            cmd_line,
            index: 0,
            options,
        }
    }

    /// The byte at the cursor, or 0 at the end (the sentinel).
    fn character(&self) -> u8 {
        self.cmd_line.get(self.index).copied().unwrap_or(0)
    }

    /// Skips whitespace (and comments). Returns false at the terminator.
    fn skip_whitespace(&mut self) -> bool {
        loop {
            match self.character() {
                0 => return false,
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b'#' => {
                    if self.options.comments {
                        loop {
                            match self.cmd_line.get(self.index).copied().unwrap_or(0) {
                                b'\n' => break,
                                0 => return false,
                                _ => self.index += 1,
                            }
                        }
                        // The loop `continue`s, which advances past the newline.
                    } else {
                        break;
                    }
                }
                _ => break,
            }
            self.index += 1;
        }
        true
    }

    /// Skips the next argument. Returns false at the end.
    pub fn skip_arg(&mut self) -> bool {
        if !self.skip_whitespace() {
            return false;
        }

        let mut backslash_count: usize = 0;
        let mut in_quote = false;
        loop {
            let c = self.character();
            match c {
                0 => return true,
                b'"' | b'\'' => {
                    if !self.options.single_quotes && c == b'\'' {
                        backslash_count = 0;
                    } else {
                        let quote_is_real = backslash_count.is_multiple_of(2);
                        if quote_is_real {
                            in_quote = !in_quote;
                        }
                    }
                }
                b'\\' => backslash_count += 1,
                b' ' | b'\t' | b'\r' | b'\n' => {
                    if !in_quote {
                        return true;
                    }
                    backslash_count = 0;
                }
                _ => backslash_count = 0,
            }
            self.index += 1;
        }
    }

    /// The next argument, or `None` at the end.
    pub fn next_arg(&mut self) -> Option<Vec<u8>> {
        if !self.skip_whitespace() {
            return None;
        }

        let mut out: Vec<u8> = Vec::new();
        let mut backslash_count: usize = 0;
        let mut in_quote = false;
        let emit_backslashes =
            |out: &mut Vec<u8>, n: usize| out.extend(std::iter::repeat_n(b'\\', n));
        loop {
            let c = self.character();
            match c {
                0 => {
                    emit_backslashes(&mut out, backslash_count);
                    return Some(out);
                }
                b'"' | b'\'' => {
                    if !self.options.single_quotes && c == b'\'' {
                        emit_backslashes(&mut out, backslash_count);
                        backslash_count = 0;
                        out.push(c);
                    } else {
                        let quote_is_real = backslash_count.is_multiple_of(2);
                        emit_backslashes(&mut out, backslash_count / 2);
                        backslash_count = 0;
                        if quote_is_real {
                            in_quote = !in_quote;
                        } else {
                            out.push(b'"');
                        }
                    }
                }
                b'\\' => backslash_count += 1,
                b' ' | b'\t' | b'\r' | b'\n' => {
                    emit_backslashes(&mut out, backslash_count);
                    backslash_count = 0;
                    if in_quote {
                        out.push(c);
                    } else {
                        return Some(out);
                    }
                }
                _ => {
                    emit_backslashes(&mut out, backslash_count);
                    backslash_count = 0;
                    out.push(c);
                }
            }
            self.index += 1;
        }
    }
}

impl Iterator for ArgIteratorGeneral<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        self.next_arg()
    }
}

#[cfg(test)]
#[allow(clippy::byte_char_slices)]
mod tests {
    use super::*;

    fn response_file(input: &str, expected: &[&str]) {
        let it = ArgIteratorGeneral::new(
            Options {
                comments: true,
                single_quotes: true,
            },
            input.as_bytes(),
        );
        let got: Vec<Vec<u8>> = it.collect();
        let want: Vec<Vec<u8>> = expected.iter().map(|s| s.as_bytes().to_vec()).collect();
        assert_eq!(got, want, "{input:?}");
    }

    #[test]
    fn response_file_arg_parsing() {
        response_file("a b\nc d\\", &["a", "b", "c", "d\\"]);
        response_file("a b c d\\", &["a", "b", "c", "d\\"]);
        response_file(
            "j\n k l # this is a comment \\\\ \\\\\\ \\\\\\\\ \"none\" \"\\\\\" \"\\\\\\\"\n \"m\" #another comment\n",
            &["j", "k", "l", "m"],
        );
        response_file(
            " \"\" q \"\"\n \"r s # t\" \"u\\\" v\" #another comment\n",
            &["", "q", "", "r s # t", "u\" v"],
        );
        response_file(
            " -l\"advapi32\" a# b#c d#\ne\\\\\\",
            &["-ladvapi32", "a#", "b#c", "d#", "e\\\\\\"],
        );
        response_file(" 'foo' \"bar\"", &["foo", "bar"]);
    }

    fn general(input: &[u8]) -> Vec<Vec<u8>> {
        ArgIteratorGeneral::new(Options::default(), input).collect()
    }

    #[test]
    fn all_zig_separators_split() {
        for sep in [b' ', b'\t', b'\r', b'\n'] {
            assert_eq!(
                general(&[b'a', sep, b'b']),
                vec![b"a".to_vec(), b"b".to_vec()]
            );
            assert_eq!(general(&[sep, sep, b'a', sep, sep]), vec![b"a".to_vec()]);
        }
        // VT and FF are not separators.
        assert_eq!(general(b"a\x0bb\x0cc"), vec![b"a\x0bb\x0cc".to_vec()]);
        // Quoted separators are kept.
        assert_eq!(
            general(b"\"a\nb\r\tc\" d"),
            vec![b"a\nb\r\tc".to_vec(), b"d".to_vec()]
        );
    }

    #[test]
    fn quoting_and_backslash_rules() {
        assert_eq!(general(b"a\\b"), vec![b"a\\b".to_vec()]);
        assert_eq!(general(b"\\\\\\\"x"), vec![b"\\\"x".to_vec()]);
        assert_eq!(general(b"\\\\\"x y\""), vec![b"\\x y".to_vec()]);
        // Single quotes are ordinary characters by default.
        assert_eq!(general(b"'a b'"), vec![b"'a".to_vec(), b"b'".to_vec()]);
        // Unterminated quote runs to the end.
        assert_eq!(general(b"\"a b"), vec![b"a b".to_vec()]);
        // Trailing backslashes are emitted verbatim.
        assert_eq!(general(b"a\\\\"), vec![b"a\\\\".to_vec()]);
        // Empty quoted argument.
        assert_eq!(general(b"\"\" x"), vec![b"".to_vec(), b"x".to_vec()]);
    }

    #[test]
    fn nul_terminates_the_command_line() {
        assert_eq!(general(b"a b\0c d"), vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(general(b"\0a"), Vec::<Vec<u8>>::new());
        assert_eq!(general(b"\"a\0b\" c"), vec![b"a".to_vec()]);
    }

    #[test]
    fn skip_matches_next() {
        let mut it = ArgIteratorGeneral::new(Options::default(), b"a \"b c\"\nd");
        assert!(it.skip_arg());
        assert_eq!(it.next_arg(), Some(b"b c".to_vec()));
        assert!(it.skip_arg());
        assert!(!it.skip_arg());
    }
}
