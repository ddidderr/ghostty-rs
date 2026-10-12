use std::{
    ffi::{OsStr, OsString},
    io,
    process::{Command, Stdio},
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
/// `url` is the core's "config string" form (see [`crate::osbytes`]): a
/// non-UTF-8 path is carried in an escaped form and is decoded to the real
/// path bytes here, right before it is handed to the opener.
///
/// Any output on stderr is logged as a warning in the application logs.
/// Output on stdout is ignored.
///
/// This function is purposely simple for the sake of providing
/// some portable way to open URLs. If you are implementing an
/// apprt for Ghostty, you should consider doing something special-cased
/// for your platform.
pub fn open(kind: OpenUrlKind, url: &str) -> io::Result<()> {
    let url: OsString = crate::osbytes::config_str_to_os(url);
    let mut cmd = open_command(kind, &url)?;

    // Pipe stdout/stderr so we can collect output from the command.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // In the snap on Linux the launcher exports LD_LIBRARY_PATH pointing at
    // the snap's bundled libraries. Leaking this into child process can
    // can be problematic, so let's drop it from the env.
    // TODO(integration): Zig does this only for `build_config.snap` builds;
    // we do it when running inside a snap (`SNAP` is set).
    if std::env::var_os("SNAP").is_some() {
        cmd.env_remove("LD_LIBRARY_PATH");
    }

    // Spawn the process on our same thread so we can detect failure
    // quickly.
    let child = cmd.spawn()?;

    // Create a thread that handles collecting output and reaping
    // the process. This is done in a separate thread because SOME
    // open implementations block and some do not. It's easier to just
    // spawn a thread to handle this so that we never block.
    std::thread::Builder::new()
        .name("open-url".into())
        .spawn(move || open_thread(child))?;
    Ok(())
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

fn open_thread(child: std::process::Child) {
    // 50 KiB is the default value used by std.process.Child.run and should
    // be enough to get the output we care about.
    const OUTPUT_MAX_SIZE: usize = 50 * 1024;

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("error waiting for open command err={e}");
            return;
        }
    };

    // If we have any stderr output we log it. This makes it easier for
    // users to debug why some open commands may not work as expected.
    let stderr = &output.stderr[..output.stderr.len().min(OUTPUT_MAX_SIZE)];
    if !stderr.is_empty() {
        log::warn!("wait stderr={}", String::from_utf8_lossy(stderr));
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

    #[test]
    fn encoded_non_utf8_path_is_decoded_for_the_opener() {
        use std::os::unix::ffi::OsStrExt as _;
        let enc = crate::osbytes::bytes_to_config_string(b"/tmp/a\xffb");
        let os = crate::osbytes::config_str_to_os(&enc);
        assert_eq!(os.as_bytes(), b"/tmp/a\xffb");
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn open_reports_spawn_failure_quickly() {
        // With an empty PATH, xdg-open can't be found; Zig detects this
        // synchronously and so do we.
        temp_env::with_var("PATH", Some("/nonexistent-dir"), || {
            let err = open(OpenUrlKind::Unknown, "https://example.com").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        });
    }
}
