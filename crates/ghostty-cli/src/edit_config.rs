//! `+edit-config` (ZIG/src/cli/edit_config.zig).

use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use ghostty_config::config::Config;
use ghostty_os::osbytes::EncodedOs;

use crate::error::Error;
pub use crate::options::NoOptions as Options;

/// The `edit-config` command opens the Ghostty configuration file in the
/// editor specified by the `$VISUAL` or `$EDITOR` environment variables.
///
/// IMPORTANT: This command will not reload the configuration after
/// editing. You will need to manually reload the configuration using the
/// application menu, configured keybind, or by restarting Ghostty. We
/// plan to auto-reload in the future, but Ghostty isn't capable of
/// this yet.
///
/// The filepath opened is the default user-specific configuration
/// file, which is typically located at `$XDG_CONFIG_HOME/ghostty/config.ghostty`.
/// On macOS, this may also be located at
/// `~/Library/Application Support/com.mitchellh.ghostty/config.ghostty`.
/// On macOS, whichever path exists and is non-empty will be prioritized,
/// prioritizing the Application Support directory if neither are
/// non-empty.
///
/// This command prefers the `$VISUAL` environment variable over `$EDITOR`,
/// if both are set. If neither are set, it will print an error
/// and exit.
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut stderr = std::io::stderr().lock();

    let mut opts = Options;
    crate::parse_options(&mut opts, args)?;

    let result = run_inner(&mut stderr);
    // Flushing *shouldn't* fail but...
    let _ = stderr.flush();
    result
}

/// Pick the editor from `$VISUAL` then `$EDITOR` (empty values are skipped).
#[must_use]
pub fn find_editor() -> Option<EncodedOs> {
    // VISUAL vs. EDITOR: https://unix.stackexchange.com/questions/4859/visual-vs-editor-what-s-the-difference
    for var in ["VISUAL", "EDITOR"] {
        if let Some(v) = ghostty_os::getenv(var) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn run_inner(stderr: &mut dyn Write) -> Result<u8, Error> {
    // We load the configuration once because that will write our
    // default configuration files to disk. We don't use the config.
    let _config = Config::load()?;

    // Find the preferred path.
    let path = ghostty_config::file_load::preferred_default_file_path()
        .map_err(|e| Error::Other(e.to_string()))?;

    // We don't currently support Windows because we use the exec syscall.
    #[cfg(windows)]
    {
        write_windows_unsupported(stderr, &path)
    }

    #[cfg(unix)]
    exec_editor(stderr, &path)
}

/// Windows branch of `run`: print the path and fail.
#[cfg(windows)]
fn write_windows_unsupported(stderr: &mut dyn Write, path: &EncodedOs) -> Result<u8, Error> {
    let message = format!(
        "The `ghostty +edit-config` command is not supported on Windows.\n\
         Please edit the configuration file manually at the following path:\n\
         \n\
         {}\n",
        path.as_encoded()
    );
    stderr.write_all(&ghostty_os::osbytes::config_str_to_bytes(&message))?;
    Ok(1)
}

#[cfg(unix)]
fn exec_editor(stderr: &mut dyn Write, path: &EncodedOs) -> Result<u8, Error> {
    // Get our editor
    let Some(editor) = find_editor() else {
        // If we don't have `$EDITOR` set then we can't do anything
        // but we can still print a helpful message.
        let msg = String::from(
            "The $EDITOR or $VISUAL environment variable is not set or is empty.\n\
             This environment variable is required to edit the Ghostty configuration\n\
             via this CLI command.\n\
             \n\
             Please set the environment variable to your preferred terminal\n\
             text editor and try again.\n\
             \n\
             If you prefer to edit the configuration file another way,\n\
             you can find the configuration file at the following path:\n\
             \n",
        );
        // Output the path using the OSC8 sequence so that it is linked.
        // (Paths are raw bytes: decode the reversible escape when writing.)
        let p = path.as_encoded();
        let msg = format!("{msg}\x1b]8;;file://{p}\x1b\\{p}\x1b]8;;\x1b\\\n");
        stderr.write_all(&ghostty_os::osbytes::config_str_to_bytes(&msg))?;
        return Ok(1);
    };

    // Flush before exec so no output is lost.
    let _ = stderr.flush();
    let err = std::process::Command::new(editor.to_os())
        .arg(path.to_os())
        .exec();

    // If we reached this point then exec failed.
    let err = ghostty_os::zig_error::io_error_name(&err);
    let msg = format!(
        "Failed to execute the editor. Error code={err}.\n\
         \n\
         This is usually due to the executable path not existing, invalid\n\
         permissions, or the shell environment not being set up\n\
         correctly.\n\
         \n\
         Editor: {}\n\
         Path: {}\n",
        editor.as_encoded(),
        path.as_encoded()
    );
    stderr.write_all(&ghostty_os::osbytes::config_str_to_bytes(&msg))?;
    Ok(1)
}
