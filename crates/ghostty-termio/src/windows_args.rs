//! Pure (platform independent) encoders for the Windows `CreateProcessW`
//! inputs, ported from `Command.zig` (`createWindowsEnvBlock` and
//! `windowsCreateCommandLine`, both copied from Zig's `std.process.Child`).
//!
//! They live outside the `cfg(windows)` code so they can be unit tested on
//! every host.

use std::ffi::OsStr;

/// Errors from encoding strings for the Windows APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// The input is not valid Unicode (Zig: `error.InvalidUtf8`).
    #[error("invalid utf-8")]
    InvalidUtf8,
}

/// Zig's Windows Command validates UTF-8, including inherited environment strings.
/// An unpaired UTF-16 surrogate in an OS string is therefore an error.
pub fn os_str_to_utf16(s: &OsStr) -> Result<Vec<u16>, EncodeError> {
    ghostty_os::osbytes::try_utf8_to_wide(&ghostty_os::osbytes::os_bytes(s))
        .map_err(|_| EncodeError::InvalidUtf8)
}

/// Create a UTF-16 environment block for `CreateProcessW` with
/// `CREATE_UNICODE_ENVIRONMENT`: a sequence of `KEY=VALUE\0` entries
/// terminated by an additional `\0`. Like Zig's copy of std's function four
/// NUL code units are appended after the last entry's own terminator (an empty
/// map is just those four), and entries keep the iteration order.
pub fn create_windows_env_block<'a>(
    env: impl IntoIterator<Item = (&'a OsStr, &'a OsStr)>,
) -> Result<Vec<u16>, EncodeError> {
    let mut result: Vec<u16> = Vec::new();
    for (key, value) in env {
        result.extend(os_str_to_utf16(key)?);
        result.push(u16::from(b'='));
        result.extend(os_str_to_utf16(value)?);
        result.push(0);
    }
    result.extend([0, 0, 0, 0]);
    Ok(result)
}

/// Build the `lpCommandLine` for `CreateProcessW` from an argv, quoting
/// arguments following the rules the MSVC runtime uses to split them again
/// (`CommandLineToArgvW`): arguments containing space, tab, newline or a
/// double quote are quoted; backslashes are doubled only when they precede a
/// quote (including the closing quote).
pub fn windows_create_command_line<S: AsRef<str>>(argv: &[S]) -> String {
    let mut buf = String::new();
    for (arg_i, arg) in argv.iter().enumerate() {
        let arg = arg.as_ref();
        if arg_i != 0 {
            buf.push(' ');
        }
        if !arg.contains([' ', '\t', '\n', '"']) {
            buf.push_str(arg);
            continue;
        }
        buf.push('"');
        let mut backslash_count = 0usize;
        for c in arg.chars() {
            match c {
                '\\' => backslash_count += 1,
                '"' => {
                    push_n(&mut buf, '\\', backslash_count * 2 + 1);
                    buf.push('"');
                    backslash_count = 0;
                }
                c => {
                    push_n(&mut buf, '\\', backslash_count);
                    buf.push(c);
                    backslash_count = 0;
                }
            }
        }
        push_n(&mut buf, '\\', backslash_count * 2);
        buf.push('"');
    }
    buf
}

/// [`windows_create_command_line`] over UTF-16 arguments (which may hold
/// lone surrogates, so they cannot be a `str`). The quoting rules are the
/// same: only ASCII characters are significant, so the logic is identical on
/// code units.
pub fn windows_create_command_line_wide(argv: &[Vec<u16>]) -> Vec<u16> {
    let (sp, tab, nl, quote, bs) = (
        u16::from(b' '),
        u16::from(b'\t'),
        u16::from(b'\n'),
        u16::from(b'"'),
        u16::from(b'\\'),
    );
    let mut buf: Vec<u16> = Vec::new();
    for (arg_i, arg) in argv.iter().enumerate() {
        if arg_i != 0 {
            buf.push(sp);
        }
        if !arg.iter().any(|c| [sp, tab, nl, quote].contains(c)) {
            buf.extend_from_slice(arg);
            continue;
        }
        buf.push(quote);
        let mut backslash_count = 0usize;
        for &c in arg {
            if c == bs {
                backslash_count += 1;
            } else if c == quote {
                buf.extend(std::iter::repeat_n(bs, backslash_count * 2 + 1));
                buf.push(quote);
                backslash_count = 0;
            } else {
                buf.extend(std::iter::repeat_n(bs, backslash_count));
                buf.push(c);
                backslash_count = 0;
            }
        }
        buf.extend(std::iter::repeat_n(bs, backslash_count * 2));
        buf.push(quote);
    }
    buf
}

/// The argv for a `shell` command on Windows (`execCommand`): the command is
/// run as `%WINDIR%\System32\cmd.exe /C <command>`.
pub fn windows_shell_args(windir: &str, command: &str) -> Vec<String> {
    let mut cmd = windir.to_owned();
    for part in ["System32", "cmd.exe"] {
        if !cmd.is_empty() && !cmd.ends_with(['\\', '/']) {
            cmd.push('\\');
        }
        cmd.push_str(part);
    }
    vec![cmd, "/C".to_owned(), command.to_owned()]
}

fn push_n(s: &mut String, c: char, n: usize) {
    s.extend(std::iter::repeat_n(c, n));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl(args: &[&str]) -> String {
        windows_create_command_line(args)
    }

    #[test]
    fn inherited_environment_rejects_non_utf8() {
        let bad = ghostty_os::osbytes::EncodedOs::from_wide(&[0xd800]).to_os();
        assert_eq!(
            create_windows_env_block([(std::ffi::OsStr::new("KEY"), bad.as_os_str())]),
            Err(EncodeError::InvalidUtf8)
        );
        assert_eq!(
            create_windows_env_block([(bad.as_os_str(), std::ffi::OsStr::new("value"))]),
            Err(EncodeError::InvalidUtf8)
        );
    }

    #[test]
    fn wide_command_line_matches_str_version_and_keeps_lone_surrogates() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        for args in [
            vec!["a", "b c", "d\"e", "f\\", "g h\\"],
            vec!["cmd.exe", "/C", "echo \"hi\" there"],
        ] {
            let wide: Vec<Vec<u16>> = args.iter().map(|a| w(a)).collect();
            assert_eq!(
                windows_create_command_line_wide(&wide),
                w(&windows_create_command_line(&args))
            );
        }
        // A lone surrogate inside a quoted argument is carried unchanged.
        let lone = vec![u16::from(b'a'), u16::from(b' '), 0xD800];
        let out = windows_create_command_line_wide(&[lone]);
        assert_eq!(out, [34, 97, 32, 0xD800, 34]);
    }

    #[test]
    fn shell_args_use_windir_cmd() {
        assert_eq!(
            windows_shell_args(r"C:\Windows", "echo hi"),
            vec![r"C:\Windows\System32\cmd.exe", "/C", "echo hi"]
        );
        // A trailing separator isn't doubled.
        assert_eq!(
            windows_shell_args(r"C:\Windows\", "x")[0],
            r"C:\Windows\System32\cmd.exe"
        );
    }

    #[test]
    fn command_line_plain_args_are_not_quoted() {
        assert_eq!(cl(&["cmd.exe"]), "cmd.exe");
        assert_eq!(cl(&["a", "b", "c"]), "a b c");
        assert_eq!(cl(&[]), "");
        // Backslashes alone never trigger quoting and are left as-is.
        assert_eq!(
            cl(&[r"C:\dir\file", r"\\server\share"]),
            r"C:\dir\file \\server\share"
        );
    }

    #[test]
    fn command_line_quotes_whitespace_and_quotes() {
        assert_eq!(cl(&["a b"]), r#""a b""#);
        assert_eq!(cl(&["a\tb"]), "\"a\tb\"");
        assert_eq!(cl(&["a\nb"]), "\"a\nb\"");
        assert_eq!(cl(&[r#"a"b"#]), r#""a\"b""#);
        assert_eq!(cl(&[""]), "");
    }

    #[test]
    fn command_line_backslashes_before_quotes() {
        // Backslashes before a quote are doubled plus one to escape the quote.
        assert_eq!(cl(&[r#"a\"b"#]), r#""a\\\"b""#);
        assert_eq!(cl(&[r#"a\\"b"#]), r#""a\\\\\"b""#);
        // Backslashes elsewhere in a quoted argument are kept.
        assert_eq!(cl(&[r"C:\Program Files\x"]), r#""C:\Program Files\x""#);
        // Trailing backslashes are doubled so they don't escape the closing quote.
        assert_eq!(cl(&[r"C:\Program Files\"]), r#""C:\Program Files\\""#);
        assert_eq!(cl(&[r"a b\\"]), r#""a b\\\\""#);
    }

    #[test]
    fn command_line_mixed() {
        assert_eq!(
            cl(&["cmd.exe", "/c", "echo hello world", r#"say "hi""#]),
            r#"cmd.exe /c "echo hello world" "say \"hi\"""#
        );
    }

    /// Reference implementation of the `CommandLineToArgvW` splitting rules
    /// for the subset of command lines we generate.
    fn split(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let mut arg = String::new();
            let mut in_quotes = false;
            while i < chars.len() && (in_quotes || chars[i] != ' ') {
                let mut bs = 0;
                while i < chars.len() && chars[i] == '\\' {
                    bs += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    arg.extend(std::iter::repeat_n('\\', bs / 2));
                    if bs % 2 == 1 {
                        arg.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    i += 1;
                } else {
                    arg.extend(std::iter::repeat_n('\\', bs));
                    if i < chars.len() && (in_quotes || chars[i] != ' ') {
                        arg.push(chars[i]);
                        i += 1;
                    }
                }
            }
            out.push(arg);
        }
        out
    }

    #[test]
    fn command_line_round_trips() {
        let cases: &[&[&str]] = &[
            &["prog", "plain"],
            &["prog", "with space", "tab\there"],
            &["prog", r#"quote"inside"#, r"\", r"trail\\"],
            &["prog", r#"a\"b c"#, r"C:\Program Files\app\"],
            &["prog", r#"""#, r#"\""#, r#"x y\\\"z"#],
        ];
        for argv in cases {
            let line = windows_create_command_line(argv);
            let back = split(&line);
            let want: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            assert_eq!(back, want, "line: {line}");
        }
    }

    fn block_to_entries(block: &[u16]) -> Vec<String> {
        // Entries are NUL terminated; the block ends with extra NULs.
        let mut entries = Vec::new();
        let mut cur = Vec::new();
        for &c in block {
            if c == 0 {
                if !cur.is_empty() {
                    entries.push(String::from_utf16(&cur).unwrap());
                    cur.clear();
                }
            } else {
                cur.push(c);
            }
        }
        entries
    }

    #[test]
    fn env_block_layout() {
        let env = [
            (OsStr::new("A"), OsStr::new("1")),
            (OsStr::new("PATH"), OsStr::new("C:\\bin;D:\\bin")),
        ];
        let block = create_windows_env_block(env).unwrap();
        let expected: Vec<u16> = "A=1\0PATH=C:\\bin;D:\\bin\0\0\0\0\0"
            .encode_utf16()
            .collect();
        assert_eq!(block, expected);
        assert_eq!(
            block_to_entries(&block),
            vec!["A=1".to_owned(), "PATH=C:\\bin;D:\\bin".to_owned()]
        );
    }

    #[test]
    fn env_block_empty_map_has_four_nuls() {
        let block = create_windows_env_block(std::iter::empty()).unwrap();
        assert_eq!(block, vec![0, 0, 0, 0]);
    }

    #[test]
    fn env_block_encodes_non_ascii_as_utf16() {
        let env = [(OsStr::new("K"), OsStr::new("héllo \u{1F600}"))];
        let block = create_windows_env_block(env).unwrap();
        let text: Vec<u16> = "K=héllo \u{1F600}\0".encode_utf16().collect();
        assert_eq!(&block[..text.len()], text.as_slice());
        // The emoji is a surrogate pair.
        assert!(block.iter().any(|&c| (0xD800..0xDC00).contains(&c)));
        assert_eq!(&block[text.len()..], &[0, 0, 0, 0]);
    }

    #[cfg(unix)]
    #[test]
    fn env_block_rejects_invalid_unicode() {
        use std::os::unix::ffi::OsStrExt;
        let bad = OsStr::from_bytes(&[0xff, 0xfe]);
        let env = [(OsStr::new("K"), bad)];
        assert_eq!(create_windows_env_block(env), Err(EncodeError::InvalidUtf8));
    }
}
