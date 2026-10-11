//! Diagnostic messages produced while parsing configuration / CLI args.

use std::fmt;

/// A diagnostic message from parsing. This is used to provide additional
/// human-friendly warnings and errors about the parsed data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diagnostic {
    pub location: Location,
    pub key: String,
    pub message: String,
}

impl Diagnostic {
    /// Write the full user-friendly diagnostic message to the writer.
    pub fn format(&self, writer: &mut dyn fmt::Write) -> fmt::Result {
        match &self.location {
            Location::None => {}
            Location::Cli(index) => write!(writer, "cli:{index}:")?,
            Location::File { path, line } => write!(writer, "{path}:{line}:")?,
        }

        if !self.key.is_empty() {
            write!(writer, "{}: ", self.key)?;
        } else if self.location != Location::None {
            write!(writer, " ")?;
        }

        write!(writer, "{}", self.message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f)
    }
}

/// The possible locations for a diagnostic message. This is used
/// to provide context for the message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Location {
    #[default]
    None,
    Cli(usize),
    File {
        path: String,
        line: usize,
    },
}

/// The tag of a [`Location`] (Zig: `Location.Key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKey {
    None,
    Cli,
    File,
}

impl Location {
    #[must_use]
    pub fn key(&self) -> LocationKey {
        match self {
            Location::None => LocationKey::None,
            Location::Cli(_) => LocationKey::Cli,
            Location::File { .. } => LocationKey::File,
        }
    }
}

/// A list of diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticList {
    list: Vec<Diagnostic>,
}

impl DiagnosticList {
    pub fn append(&mut self, diag: Diagnostic) {
        self.list.push(diag);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Zig: `empty()`.
    #[must_use]
    pub fn empty(&self) -> bool {
        self.list.is_empty()
    }

    #[must_use]
    pub fn items(&self) -> &[Diagnostic] {
        &self.list
    }

    /// Returns true if there are any diagnostics for the given
    /// location type.
    #[must_use]
    pub fn contains_location(&self, location: LocationKey) -> bool {
        self.list.iter().any(|d| d.location.key() == location)
    }
}
