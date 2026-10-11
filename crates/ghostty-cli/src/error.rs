//! The error type returned by CLI actions.

use ghostty_config::cli_args::ArgError;

/// The name used by [`ArgError::Other`] to signal that `--help`/`-h` was
/// passed (Zig: `Action.help_error`).
pub const HELP_ERROR_NAME: &str = "ActionHelpRequested";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// This should be returned by actions that want to print the help text
    /// (Zig: `error.ActionHelpRequested`).
    #[error("ActionHelpRequested")]
    ActionHelpRequested,

    /// Argument parsing failed.
    #[error("{0}")]
    Args(ArgError),

    /// Loading the configuration failed.
    #[error("{0}")]
    Config(#[from] ghostty_config::config::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// The action exists but depends on functionality that is not ported yet.
    #[error("the `+{0}` action is not yet available in this build")]
    NotYetAvailable(&'static str),

    #[error("{0}")]
    Other(String),
}

impl From<ArgError> for Error {
    fn from(e: ArgError) -> Self {
        match e {
            ArgError::Other(ref n) if n == HELP_ERROR_NAME => Error::ActionHelpRequested,
            e => Error::Args(e),
        }
    }
}

impl From<std::fmt::Error> for Error {
    fn from(_: std::fmt::Error) -> Self {
        Error::Other("FormatError".to_string())
    }
}
