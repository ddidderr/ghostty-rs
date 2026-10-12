//! `+new-window` (ZIG/src/cli/new_window.zig).
//!
//! Sending the IPC needs the app runtime (`apprt.App.performIpc`); the root
//! binary registers it with [`set_ipc_handler`] so this crate does not
//! depend on GTK.

use std::sync::OnceLock;

use ghostty_apprt::ipc;
use ghostty_config::cli_args::{ArgError, ArgIter, CliTarget};
use ghostty_os::osbytes::EncodedOs;

use crate::error::Error;

/// The runtime's IPC entrypoint (`apprt.App.performIpc`). `Ok(true)` means
/// the action was performed; `Err` carries an error message (use
/// [`IPC_FAILED`] when the runtime already printed a specific message).
pub type IpcHandler = fn(&ipc::Target, &ipc::Action) -> Result<bool, String>;

/// Error string for `error.IPCFailed`: the runtime already reported it.
pub const IPC_FAILED: &str = "IPCFailed";

static IPC_HANDLER: OnceLock<IpcHandler> = OnceLock::new();

/// Register the runtime IPC handler. Only the first call has an effect.
pub fn set_ipc_handler(f: IpcHandler) {
    let _ = IPC_HANDLER.set(f);
}

/// Options of `+new-window`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// If set, open up a new window in a custom instance of Ghostty.
    pub class: Option<String>,

    /// Did the user specify a `--working-directory` argument on the command line?
    pub working_directory_seen: bool,

    /// All of the arguments after `+new-window`. They will be sent to Ghostty
    /// for processing.
    pub arguments: Vec<Vec<u8>>,
}

impl Options {
    fn check_arg(&mut self, arg: &[u8]) -> Result<Option<Vec<u8>>, ArgError> {
        if let Some(rest) = arg.strip_prefix(b"--class=") {
            self.class = Some(
                std::str::from_utf8(ghostty_config::cli_args::trim_ascii_whitespace(rest))
                    .map_err(|_| ArgError::InvalidValue)?
                    .to_owned(),
            );
            return Ok(None);
        }

        if let Some(rest) = arg.strip_prefix(b"--working-directory=") {
            let stripped = ghostty_config::cli_args::trim_ascii_whitespace(rest);
            if stripped == b"home" || stripped == b"inherit" {
                return Ok(Some(arg.to_vec()));
            }
            // Preserve the original argument bytes while expanding its path.
            let expanded = ghostty_os::expand_home(&EncodedOs::from_bytes(stripped))
                .map_err(|e| ArgError::Other(e.to_string()))?;
            let realpath = expanded
                .to_path()
                .and_then(ghostty_os::path::realpath)
                .map_err(|e| ArgError::Other(e.to_string()))?;
            self.working_directory_seen = true;
            return Ok(Some(
                [
                    b"--working-directory=".as_slice(),
                    &EncodedOs::from_path(&realpath).to_bytes(),
                ]
                .concat(),
            ));
        }

        Ok(Some(arg.to_vec()))
    }
}

impl CliTarget for Options {
    fn parse_into_field(&mut self, _key: &str, _value: Option<&[u8]>) -> Result<(), ArgError> {
        Err(ArgError::InvalidField)
    }

    /// Manual parse hook, collect all of the arguments after `+new-window`.
    fn parse_manually_hook(
        &mut self,
        arg: &[u8],
        iter: &mut dyn ArgIter,
    ) -> Result<bool, ArgError> {
        let mut e_seen = arg == b"-e";

        // Include the argument that triggered the manual parse hook.
        if let Some(a) = self.check_arg(arg)? {
            self.arguments.push(a);
        }

        // Gather up the rest of the arguments to use as the command.
        while let Some(param) = iter.next_arg() {
            if e_seen {
                self.arguments.push(param);
                continue;
            }
            if param == b"-e" {
                e_seen = true;
                self.arguments.push(param);
                continue;
            }
            if let Some(a) = self.check_arg(&param)? {
                self.arguments.push(a);
            }
        }

        Ok(false)
    }

    fn supports_help(&self) -> bool {
        true
    }

    fn help(&mut self) -> Result<(), ArgError> {
        Err(ArgError::other(crate::error::HELP_ERROR_NAME))
    }
}

/// The `new-window` will use native platform IPC to open up a new window in a
/// running instance of Ghostty.
///
/// If the `--class` flag is not set, the `new-window` command will try and
/// connect to a running instance of Ghostty based on what optimizations the
/// Ghostty CLI was compiled with. Otherwise the `new-window` command will try
/// and contact a running Ghostty instance that was configured with the same
/// `class` as was given on the command line.
///
/// All of the arguments after the `+new-window` argument (except for the
/// `--class` flag) will be sent to the remote Ghostty instance and will be
/// parsed as command line flags. These flags will override certain settings
/// when creating the first surface in the new window. Currently, only
/// `--working-directory`, `--command`, and `--title` are supported. `-e` will
/// also work as an alias for `--command`, except that if `-e` is found on the
/// command line all following arguments will become part of the command and no
/// more arguments will be parsed for configuration settings.
///
/// If `--working-directory` is found on the command line and is a relative
/// path (i.e. doesn't start with `/`) it will be resolved to an absolute path
/// relative to the current working directory that the `ghostty +new-window`
/// command is run from. `~/` prefixes will also be expanded to the user's home
/// directory.
///
/// If `--working-directory` is _not_ found on the command line, the working
/// directory that `ghostty +new-window` is run from will be passed to Ghostty.
///
/// GTK uses an application ID to identify instances of applications. If Ghostty
/// is compiled with release optimizations, the default application ID will be
/// `com.mitchellh.ghostty`. If Ghostty is compiled with debug optimizations,
/// the default application ID will be `com.mitchellh.ghostty-debug`.  The
/// `class` configuration entry can be used to set up a custom application
/// ID. The class name must follow the requirements defined [in the GTK
/// documentation](https://docs.gtk.org/gio/type_func.Application.id_is_valid.html)
/// or it will be ignored and Ghostty will use the default as defined above.
///
/// On GTK, D-Bus activation must be properly configured. Ghostty does not need
/// to be running for this to open a new window, making it suitable for binding
/// to keys in your window manager (if other methods for configuring global
/// shortcuts are unavailable). D-Bus will handle launching a new instance
/// of Ghostty if it is not already running. See the Ghostty website for
/// information on properly configuring D-Bus activation.
///
/// Only supported on GTK.
///
/// Flags:
///
///   * `--class=<class>`: If set, open up a new window in a custom instance of
///     Ghostty. The class must be a valid GTK application ID.
///
///   * `--command`: The command to be executed in the first surface of the new window.
///
///   * `--working-directory=<directory>`: The working directory to pass to Ghostty.
///
///   * `--title`: A title that will override the title of the first surface in
///     the new window. The title override may be edited or removed later.
///
///   * `-e`: Any arguments after this will be interpreted as a command to
///     execute inside the first surface of the new window instead of the
///     default command.
///
/// Available since: 1.2.0
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut stderr = crate::zig_writer::ZigWriter::new(crate::zig_writer::stderr_sink(), 1024);
    let result = run_args(args, &mut stderr);
    let _ = std::io::Write::flush(&mut stderr);
    result
}

fn run_args(args: &[impl AsRef<[u8]>], stderr: &mut dyn std::io::Write) -> Result<u8, Error> {
    let mut opts = Options::default();
    let mut iter =
        ghostty_config::cli_args::ArgsIterator::new(args.iter().skip(1).map(AsRef::as_ref));
    match ghostty_config::cli_args::parse(&mut opts, &mut iter) {
        Ok(()) => {}
        Err(ArgError::Other(n)) if n == crate::error::HELP_ERROR_NAME => {
            return Err(Error::ActionHelpRequested);
        }
        Err(err) => {
            writeln!(stderr, "Error parsing args: {err}")?;
            return Ok(1);
        }
    }

    if !opts.working_directory_seen {
        // This should be inserted at the beginning of the list, just in case `-e` was used.
        let wd = ghostty_os::path::realpath(".")?;
        opts.arguments.insert(
            0,
            [
                b"--working-directory=".as_slice(),
                &EncodedOs::from_path(&wd).to_bytes(),
            ]
            .concat(),
        );
    }

    let target = match opts.class {
        Some(class) => ipc::Target::Class(class),
        None => ipc::Target::Detect,
    };
    let action = ipc::Action::NewWindow(ipc::NewWindow {
        arguments: if opts.arguments.is_empty() {
            None
        } else {
            Some(opts.arguments)
        },
    });

    // Without a registered runtime this behaves like `apprt.none`, whose
    // performIpc always returns false.
    let performed = match IPC_HANDLER.get() {
        Some(handler) => match handler(&target, &action) {
            Ok(v) => v,
            Err(e) if e == IPC_FAILED => {
                // The apprt should have printed a more specific error message
                // already.
                return Ok(1);
            }
            Err(e) => {
                write!(stderr, "Sending the IPC failed: {e}")?;
                return Ok(1);
            }
        },
        None => false,
    };
    if performed {
        return Ok(0);
    }

    // If we get here, the platform is not supported.
    writeln!(stderr, "+new-window is not supported on this platform.")?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use ghostty_config::cli_args::{VecIterator, parse};

    use super::*;

    fn parse_str(s: &str) -> Options {
        let mut opts = Options::default();
        let mut it = VecIterator::from_str_args(s);
        parse(&mut opts, &mut it).unwrap();
        opts
    }

    #[test]
    fn gathers_arguments_and_class() {
        let o = parse_str("--class=foo.bar --title=hi --command=ls");
        assert_eq!(Some("foo.bar".to_string()), o.class);
        assert_eq!(
            vec!["--title=hi", "--command=ls"],
            o.arguments
                .iter()
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
        );
        assert!(!o.working_directory_seen);
    }

    #[test]
    fn everything_after_e_is_command() {
        let o = parse_str("--title=x -e --class=nope --working-directory=/nonexistent");
        assert_eq!(None, o.class);
        assert_eq!(
            vec![
                "--title=x",
                "-e",
                "--class=nope",
                "--working-directory=/nonexistent"
            ],
            o.arguments
                .iter()
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn working_directory_is_resolved() {
        let o = parse_str("--working-directory=/");
        assert!(o.working_directory_seen);
        // The root resolves to `Z:\` under Wine / `C:\` on Windows.
        let root = format!(
            "--working-directory={}",
            ghostty_os::path::realpath("/").unwrap().display()
        );
        assert_eq!(
            vec![root.as_str()],
            o.arguments
                .iter()
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
        );
        let o = parse_str("--working-directory=home");
        assert!(!o.working_directory_seen);
        assert_eq!(
            vec!["--working-directory=home"],
            o.arguments
                .iter()
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn help_is_gathered_not_special() {
        // Like Zig, the manual hook runs before the help check.
        let o = parse_str("--help");
        assert_eq!(
            vec!["--help"],
            o.arguments
                .iter()
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ipc_handler_registration_is_once() {
        #[allow(clippy::unnecessary_wraps)]
        fn h(_: &ipc::Target, _: &ipc::Action) -> Result<bool, String> {
            Ok(true)
        }
        set_ipc_handler(h);
        let r = IPC_HANDLER.get().unwrap()(
            &ipc::Target::Detect,
            &ipc::Action::NewWindow(ipc::NewWindow { arguments: None }),
        );
        assert_eq!(r, Ok(true));
    }
}
