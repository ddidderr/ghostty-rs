use std::ffi::{CString, NulError};
use std::io::{self, Write};

/// Builder for constructing space-separated shell command strings.
#[derive(Debug, Default, Clone)]
pub struct ShellCommandBuilder {
    buffer: String,
}

impl ShellCommandBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an argument to the command with automatic space separation.
    /// Empty arguments are ignored.
    pub fn append_arg(&mut self, arg: &str) {
        if arg.is_empty() {
            return;
        }
        if !self.buffer.is_empty() {
            self.buffer.push(' ');
        }
        self.buffer.push_str(arg);
    }

    /// The command written so far.
    pub fn written(&self) -> &str {
        &self.buffer
    }

    /// Get the final null-terminated command string.
    pub fn to_owned_slice(self) -> Result<CString, NulError> {
        CString::new(self.buffer)
    }
}

/// Writer that escapes characters that shells treat specially to reduce the
/// risk of injection attacks or other such weirdness. Specifically excludes
/// linefeeds so that they can be used to delineate lists of file paths.
pub struct ShellEscapeWriter<W: Write> {
    child: W,
}

impl<W: Write> ShellEscapeWriter<W> {
    pub fn new(child: W) -> Self {
        Self { child }
    }

    pub fn into_inner(self) -> W {
        self.child
    }
}

impl<W: Write> Write for ShellEscapeWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        for &byte in data {
            match byte {
                b'\\' | b'"' | b'\'' | b'$' | b'`' | b'*' | b'?' | b' ' | b'|' | b'(' | b')' => {
                    self.child.write_all(&[b'\\', byte])?;
                }
                _ => self.child.write_all(&[byte])?,
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.child.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_builder() {
        // Empty command
        let cmd = ShellCommandBuilder::new();
        assert_eq!("", cmd.written());

        // Single arg
        let mut cmd = ShellCommandBuilder::new();
        cmd.append_arg("bash");
        assert_eq!("bash", cmd.written());

        // Multiple args
        let mut cmd = ShellCommandBuilder::new();
        cmd.append_arg("bash");
        cmd.append_arg("--posix");
        cmd.append_arg("-l");
        assert_eq!("bash --posix -l", cmd.written());

        // Empty arg
        let mut cmd = ShellCommandBuilder::new();
        cmd.append_arg("bash");
        cmd.append_arg("");
        assert_eq!("bash", cmd.written());

        // toOwnedSlice
        let mut cmd = ShellCommandBuilder::new();
        cmd.append_arg("bash");
        cmd.append_arg("--posix");
        let result = cmd.to_owned_slice().unwrap();
        assert_eq!(result.to_str().unwrap(), "bash --posix");
        assert_eq!(result.as_bytes_with_nul().last(), Some(&0));
    }

    fn escape(input: &str) -> String {
        let mut shell = ShellEscapeWriter::new(Vec::new());
        shell.write_all(input.as_bytes()).unwrap();
        String::from_utf8(shell.into_inner()).unwrap()
    }

    #[test]
    fn shell_escape_1() {
        assert_eq!(escape("abc"), "abc");
    }

    #[test]
    fn shell_escape_2() {
        assert_eq!(escape("a c"), "a\\ c");
    }

    #[test]
    fn shell_escape_3() {
        assert_eq!(escape("a?c"), "a\\?c");
    }

    #[test]
    fn shell_escape_4() {
        assert_eq!(escape("a\\c"), "a\\\\c");
    }

    #[test]
    fn shell_escape_5() {
        assert_eq!(escape("a|c"), "a\\|c");
    }

    #[test]
    fn shell_escape_6() {
        assert_eq!(escape("a\"c"), "a\\\"c");
    }

    #[test]
    fn shell_escape_7() {
        assert_eq!(escape("a(1)"), "a\\(1\\)");
    }

    #[test]
    fn shell_escape_keeps_newlines() {
        assert_eq!(escape("a\nb"), "a\nb");
    }
}
