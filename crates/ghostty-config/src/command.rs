//! A command to execute (port of `config/command.zig`).

use std::fmt;

use ghostty_os::osbytes::EncodedOs;

use crate::{
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

/// A command to execute (argv0 and args).
///
/// A command is specified as a simple string such as "nvim a b c".
/// By default, we expect the downstream to do some sort of shell expansion
/// on this string.
///
/// If a command is already expanded and the user does NOT want to do
/// shell expansion (because this usually requires a round trip into
/// /bin/sh or equivalent), specify a `direct:`-prefix. e.g.
/// `direct:nvim a b c`.
///
/// The whitespace before or around the prefix is ignored. For example,
/// `  direct:nvim a b c` and `direct: nvim a b c` are equivalent.
///
/// If the command is not absolute, it'll be looked up via the PATH.
/// For the shell-expansion case, we let the shell do this. For the
/// direct case, we do this directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Execute a command directly, e.g. via `exec`. The format here
    /// is already structured to be ready to passed directly to `exec`
    /// with index zero being the command to execute.
    ///
    /// Index zero is not guaranteed to be an absolute path, and may require
    /// PATH lookup. It is up to the downstream to do this, usually via
    /// delegation to something like `execvp`.
    Direct(Vec<EncodedOs>),

    /// Execute a command via shell expansion. This provides the command
    /// as a single string that is expected to be expanded in some way
    /// (up to the downstream). Usually `/bin/sh -c`.
    Shell(EncodedOs),
}

impl Default for Command {
    fn default() -> Self {
        Command::Shell(EncodedOs::new())
    }
}

/// Iterates over each argument in the command.
#[derive(Debug, Clone)]
pub struct ArgIterator {
    args: std::vec::IntoIter<EncodedOs>,
}

impl Iterator for ArgIterator {
    type Item = EncodedOs;

    fn next(&mut self) -> Option<EncodedOs> {
        self.args.next()
    }
}

/// Zig's `std.process.ArgIteratorGeneral(.{})`.
fn split_general(b: &[u8]) -> Vec<EncodedOs> {
    ghostty_os::arg_iterator::ArgIteratorGeneral::new(
        ghostty_os::arg_iterator::Options::default(),
        b,
    )
    .map(|arg| EncodedOs::from_bytes(&arg))
    .collect()
}

impl Command {
    /// Creates a command as a single string, joining arguments as
    /// necessary with spaces. Its not guaranteed that this is a valid
    /// command; it is only meant to be human readable.
    #[must_use]
    pub fn string(&self) -> String {
        match self {
            Command::Shell(v) => v.as_encoded().to_owned(),
            Command::Direct(v) => v
                .iter()
                .map(EncodedOs::as_encoded)
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Get an iterator over the arguments array.
    ///
    /// For direct commands this just iterates over the array. For shell
    /// commands this uses a best effort shell string parser (like Zig's
    /// `ArgIteratorGeneral`). This is not guaranteed to be 100% accurate,
    /// but it works for common cases.
    #[must_use]
    pub fn arg_iterator(&self) -> ArgIterator {
        let args = match self {
            Command::Direct(v) => v.clone(),
            Command::Shell(v) => split_general(&v.to_bytes()),
        };
        ArgIterator {
            args: args.into_iter(),
        }
    }
}

fn trim_spaces(value: &[u8]) -> &[u8] {
    let start = value.iter().position(|b| *b != b' ').unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| *b != b' ')
        .map_or(start, |i| i + 1);
    &value[start..end]
}

impl ConfigField for Command {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        // Input is required. Whitespace on the edges isn't needed.
        // Commands must be non-empty.
        let input = input.ok_or(ArgError::ValueRequired)?;
        let trimmed = trim_spaces(input);
        if trimmed.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        // If we have a `:` then we MIGHT have a prefix to specify what
        // tag we should use.
        let (direct, str) = match trimmed.iter().position(|b| *b == b':') {
            Some(idx) if &trimmed[..idx] == b"direct" => (true, &trimmed[idx + 1..]),
            Some(idx) if &trimmed[..idx] == b"shell" => (false, &trimmed[idx + 1..]),
            _ => (false, trimmed),
        };

        let str = trim_spaces(str);
        *self = if direct {
            // We're not shell expanding, so the arguments are naively
            // split on spaces.
            Command::Direct(
                str.split(|b| *b == b' ')
                    .map(EncodedOs::from_bytes)
                    .collect(),
            )
        } else {
            Command::Shell(EncodedOs::from_bytes(str))
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            Command::Shell(v) => writeln!(f.writer, "{} = {}", f.name, v.as_encoded()),
            Command::Direct(v) => writeln!(
                f.writer,
                "{} = direct:{}",
                f.name,
                v.iter()
                    .map(EncodedOs::as_encoded)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    #[test]
    fn command_parse_cli_errors() {
        let mut v = Command::default();
        assert_eq!(v.parse_cli(None), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some("")), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some(" ")), Err(ArgError::ValueRequired));
    }

    #[test]
    fn command_parse_cli_shell_expanded() {
        let mut v = Command::default();
        v.parse_cli(Some("echo hello")).unwrap();
        assert_eq!(v, Command::Shell("echo hello".into()));

        // Spaces are stripped
        v.parse_cli(Some(" echo hello ")).unwrap();
        assert_eq!(v, Command::Shell("echo hello".into()));
    }

    #[test]
    fn command_parse_cli_direct() {
        let mut v = Command::default();
        v.parse_cli(Some("direct:echo hello")).unwrap();
        assert_eq!(v, Command::Direct(vec!["echo".into(), "hello".into()]));

        // Spaces around the prefix
        v.parse_cli(Some(" direct:  echo hello")).unwrap();
        assert_eq!(v, Command::Direct(vec!["echo".into(), "hello".into()]));
    }

    #[test]
    fn command_arg_iterator_shell() {
        let v = Command::Shell("echo hello world".into());
        let mut it = v.arg_iterator();
        assert_eq!(it.next().unwrap(), "echo");
        assert_eq!(it.next().unwrap(), "hello");
        assert_eq!(it.next().unwrap(), "world");
        assert!(it.next().is_none());
    }

    #[test]
    fn command_arg_iterator_shell_quotes() {
        let v = Command::Shell("echo \"hello world\" a\\\"b".into());
        let args: Vec<EncodedOs> = v.arg_iterator().collect();
        assert_eq!(args, ["echo", "hello world", "a\"b"]);
    }

    #[test]
    fn command_arg_iterator_shell_splits_cr_lf_tab_like_zig() {
        let v = Command::Shell(EncodedOs::from_bytes(b"bash\n-i\r\tx \"a\nb\"\0ignored"));
        let args: Vec<EncodedOs> = v.arg_iterator().collect();
        assert_eq!(args, ["bash", "-i", "x", "a\nb"]);
    }

    #[test]
    fn command_arg_iterator_direct() {
        let v = Command::Direct(vec!["echo".into(), "hello world".into()]);
        let mut it = v.arg_iterator();
        assert_eq!(it.next().unwrap(), "echo");
        assert_eq!(it.next().unwrap(), "hello world");
        assert!(it.next().is_none());
    }

    #[test]
    fn command_string_shell() {
        let v = Command::Shell("echo hello world".into());
        assert_eq!(v.string(), "echo hello world");
    }

    #[test]
    fn command_string_direct() {
        let v = Command::Direct(vec!["echo".into(), "hello world".into()]);
        assert_eq!(v.string(), "echo hello world");
    }

    #[test]
    fn command_format_config_shell() {
        let mut v = Command::default();
        v.parse_cli(Some("echo hello")).unwrap();
        let mut s = String::new();
        format_entry("a", &v, &mut s).unwrap();
        assert_eq!(s, "a = echo hello\n");
    }

    #[test]
    fn command_format_config_direct() {
        let mut v = Command::default();
        v.parse_cli(Some("direct: echo hello")).unwrap();
        let mut s = String::new();
        format_entry("a", &v, &mut s).unwrap();
        assert_eq!(s, "a = direct:echo hello\n");
    }
}
