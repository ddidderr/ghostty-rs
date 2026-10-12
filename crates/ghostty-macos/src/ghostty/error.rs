//! `Ghostty.Error` (`Ghostty.Error.swift`): possible errors from internal
//! Ghostty calls.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    ApiFailed,
}

impl fmt::Display for Error {
    /// `localizedStringResource`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiFailed => f.write_str("libghostty API call failed"),
        }
    }
}

impl std::error::Error for Error {}
