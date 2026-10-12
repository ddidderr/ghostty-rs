use std::{
    ffi::OsStr,
    io::{self, Read},
    process::{Child, Command, Stdio},
};

/// Kind of URL being opened (`apprt.action.OpenUrl.Kind`). On Linux the
/// kind is ignored by [`open`] (it only matters for macOS `open -t`).
// TODO(integration): replace with the apprt action's OpenUrl kind once
// ghostty-core defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenUrlKind {
    Text,
    Html,
    #[default]
    Unknown,
}

/// Open a URL in the default handling application (via `xdg-open`).
///
/// `url` is the exact OS string to hand to the opener. Callers holding an
/// [`EncodedOs`](crate::osbytes::EncodedOs) decode it explicitly
/// (`to_os`); plain text (a regex link, an OSC 8 URI) is wrapped with
/// `EncodedOs::from_plain_text` first, so it is never decoded by mistake.
///
/// Any output on stderr is logged as a warning in the application logs.
/// Output on stdout is ignored.
///
/// This function is purposely simple for the sake of providing
/// some portable way to open URLs. If you are implementing an
/// apprt for Ghostty, you should consider doing something special-cased
/// for your platform.
pub fn open(kind: OpenUrlKind, url: &OsStr) -> io::Result<()> {
    // Spawn the process on our same thread so we can detect failure
    // quickly.
    let child = spawn_child(open_command(kind, url)?)?;

    // Create a thread that handles collecting output and reaping
    // the process. This is done in a separate thread because SOME
    // open implementations block and some do not. It's easier to just
    // spawn a thread to handle this so that we never block.
    std::thread::Builder::new()
        .name("open-url".into())
        .spawn(move || open_thread(child))?;
    Ok(())
}

/// Configure and spawn the opener. Like Zig's `Child.init`, stdin is
/// inherited.
fn spawn_child(mut cmd: Command) -> io::Result<Child> {
    // Pipe stdout/stderr so we can collect output from the command.
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    strip_snap_env(&mut cmd);
    cmd.spawn()
}

/// In the snap on Linux the launcher exports LD_LIBRARY_PATH pointing at
/// the snap's bundled libraries. Leaking this into child process can
/// can be problematic, so let's drop it from the env. Zig does this only
/// for `build_config.snap` builds (the `snap` cargo feature here, off by
/// default).
fn strip_snap_env(cmd: &mut Command) {
    if cfg!(feature = "snap") {
        cmd.env_remove("LD_LIBRARY_PATH");
    }
}

/// Build the platform's "open URL" command (Zig `open`'s switch over
/// `builtin.os.tag`).
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
fn open_command(kind: OpenUrlKind, url: &OsStr) -> io::Result<Command> {
    let _ = kind;
    let mut cmd = Command::new("rundll32");
    cmd.arg("url.dll,FileProtocolHandler").arg(url);
    Ok(cmd)
}

/// Linux and the BSDs use `xdg-open`.
#[cfg(not(any(windows, target_os = "macos", target_os = "ios")))]
#[allow(clippy::unnecessary_wraps)]
fn open_command(kind: OpenUrlKind, url: &OsStr) -> io::Result<Command> {
    let _ = kind;
    let mut cmd = Command::new("xdg-open");
    cmd.arg(url);
    Ok(cmd)
}

/// macOS uses `open` (`open -t` for text files).
#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)]
fn open_command(kind: OpenUrlKind, url: &OsStr) -> io::Result<Command> {
    let mut cmd = Command::new("open");
    cmd.args(macos_open_args(kind, url));
    Ok(cmd)
}

/// Arguments of the macOS `open` command for a URL kind.
#[cfg_attr(not(any(target_os = "macos", all(test, unix))), allow(dead_code))]
fn macos_open_args(kind: OpenUrlKind, url: &OsStr) -> Vec<&OsStr> {
    match kind {
        OpenUrlKind::Text => vec![OsStr::new("-t"), url],
        OpenUrlKind::Html | OpenUrlKind::Unknown => vec![url],
    }
}

/// iOS cannot spawn processes (`error.Unimplemented`).
#[cfg(target_os = "ios")]
#[allow(clippy::unnecessary_wraps)]
fn open_command(_kind: OpenUrlKind, _url: &OsStr) -> io::Result<Command> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "unimplemented"))
}

/// 50 KiB is the default value used by std.process.Child.run and should
/// be enough to get the output we care about.
const OUTPUT_MAX_SIZE: usize = 50 * 1024;

/// Read at most [`OUTPUT_MAX_SIZE`] bytes; more is an error
/// (`error.StdoutStreamTooLong` / `StderrStreamTooLong` in Zig).
fn read_bounded(mut r: impl Read, name: &str) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    (&mut r)
        .take(OUTPUT_MAX_SIZE as u64 + 1)
        .read_to_end(&mut buf)?;
    if buf.len() > OUTPUT_MAX_SIZE {
        return Err(io::Error::other(format!("{name} stream too long")));
    }
    Ok(buf)
}

/// Collect both streams concurrently, bounded per stream.
fn collect_output(child: &mut Child) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_thread = std::thread::spawn(move || match stdout {
        Some(r) => read_bounded(r, "stdout"),
        None => Ok(Vec::new()),
    });
    let err = match stderr {
        Some(r) => read_bounded(r, "stderr"),
        None => Ok(Vec::new()),
    };
    let out = out_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stdout reader panicked")));
    Ok((out?, err?))
}

fn open_thread(mut child: Child) {
    let collected = collect_output(&mut child);
    // Zig's `try collectOutput` skips `wait` on error; reap anyway so the
    // child does not stay a zombie (the pipes are closed by now).
    let waited = child.wait();
    let stderr = match collected {
        Ok((_stdout, stderr)) => stderr,
        Err(e) => {
            log::warn!("error collecting open command output err={e}");
            return;
        }
    };
    if let Err(e) = waited {
        log::warn!("error waiting for open command err={e}");
        return;
    }

    // If we have any stderr output we log it. This makes it easier for
    // users to debug why some open commands may not work as expected.
    if !stderr.is_empty() {
        log::warn!("wait stderr={}", String::from_utf8_lossy(&stderr));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn macos_open_command_args() {
        let a = OsStr::new("/a");
        let u = OsStr::new("u");
        assert_eq!(macos_open_args(OpenUrlKind::Text, a), ["-t", "/a"]);
        assert_eq!(macos_open_args(OpenUrlKind::Html, u), ["u"]);
        assert_eq!(macos_open_args(OpenUrlKind::Unknown, u), ["u"]);
    }

    /// The opener receives decoded bytes for encoded paths and the text
    /// unchanged for plain URLs (even when they contain the escape char).
    #[test]
    fn encoded_paths_decode_but_plain_urls_do_not() {
        use std::os::unix::ffi::OsStrExt as _;

        use crate::osbytes::EncodedOs;
        let path = EncodedOs::from_bytes(b"/tmp/a\xffb");
        assert_eq!(path.to_os().as_bytes(), b"/tmp/a\xffb");

        // U+FDD0 U+00FF in an ordinary URL must stay two chars, not byte FF.
        let url = "https://x/\u{fdd0}\u{ff}";
        let plain = EncodedOs::from_plain_text(url);
        assert_eq!(plain.to_os().as_bytes(), url.as_bytes());
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn open_reports_spawn_failure_quickly() {
        // With an empty PATH, xdg-open can't be found; Zig detects this
        // synchronously and so do we.
        temp_env::with_var("PATH", Some("/nonexistent-dir"), || {
            let err = open(OpenUrlKind::Unknown, OsStr::new("https://example.com")).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        });
    }

    #[test]
    fn output_collection_is_bounded_per_stream() {
        let mut ok = Command::new("/bin/sh")
            .args([
                "-c",
                "PATH=/usr/bin:/bin; head -c 51200 /dev/zero; head -c 51200 /dev/zero >&2",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (out, err) = collect_output(&mut ok).unwrap();
        assert_eq!((out.len(), err.len()), (OUTPUT_MAX_SIZE, OUTPUT_MAX_SIZE));
        ok.wait().unwrap();

        for fd in ["", ">&2"] {
            let script = format!("PATH=/usr/bin:/bin; head -c 51201 /dev/zero {fd}");
            let mut big = Command::new("/bin/sh")
                .args(["-c", &script])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            assert!(collect_output(&mut big).is_err(), "{fd}");
            big.wait().unwrap();
        }
    }

    #[test]
    fn snap_env_is_stripped_only_for_snap_builds() {
        let mut cmd = Command::new("true");
        strip_snap_env(&mut cmd);
        let removed = cmd
            .get_envs()
            .any(|(k, v)| k == "LD_LIBRARY_PATH" && v.is_none());
        assert_eq!(removed, cfg!(feature = "snap"));
    }

    /// The opener inherits stdin (Zig `Child.init` default).
    #[test]
    fn opener_inherits_stdin() {
        use std::io::Write as _;
        const CHILD: &str = "GHOSTTY_TEST_OPEN_STDIN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let child = spawn_child(Command::new("/bin/cat")).unwrap();
            let out = child.wait_with_output().unwrap();
            print!("got:{}", String::from_utf8_lossy(&out.stdout));
            return;
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "open::tests::opener_inherits_stdin",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"hello").unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("got:hello"));
    }
}
