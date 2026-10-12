//! API-compatible stand-ins for OS facilities that do not exist on wasm32,
//! so crates that are also built for wasm (`ghostty-config`) compile
//! unchanged. In Zig these are simply never analyzed on wasm
//! (`os/passwd.zig` even `@compileError`s); here each reports "unavailable"
//! the way the Zig callers' error paths expect.

use std::io;

/// `homedir.zig` is unavailable: nothing can be expanded.
pub mod homedir {
    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum ExpandError {
        #[error("home detection failed")]
        HomeDetectionFailed,
        #[error("buffer too small")]
        BufferTooSmall,
    }

    /// Paths without a leading `~/` pass through; others fail.
    pub fn expand_home(path: &str) -> Result<String, ExpandError> {
        if path.starts_with("~/") {
            Err(ExpandError::HomeDetectionFailed)
        } else {
            Ok(path.to_owned())
        }
    }
}

pub mod passwd {
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Entry {
        pub shell: Option<String>,
        pub home: Option<String>,
        pub name: Option<String>,
    }

    /// Zig: `@compileError("passwd is not available for wasm")`.
    pub fn get() -> std::io::Result<Entry> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "passwd is not available for wasm",
        ))
    }
}

pub mod xdg {
    use std::ffi::CStr;

    #[derive(Debug, Clone, Copy, Default)]
    pub struct Options<'a> {
        pub subdir: Option<&'a str>,
        pub home: Option<&'a str>,
    }

    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum Error {
        #[error("xdg directories are not available on wasm")]
        Unavailable,
    }

    pub fn config(_: &Options<'_>) -> Result<String, Error> {
        Err(Error::Unavailable)
    }

    pub fn cache(_: &Options<'_>) -> Result<String, Error> {
        Err(Error::Unavailable)
    }

    pub fn state(_: &Options<'_>) -> Result<String, Error> {
        Err(Error::Unavailable)
    }

    /// No `xdg-terminal-exec` mode on wasm.
    pub fn parse_terminal_exec<'a>(_: &'a [&'a CStr]) -> Option<&'a [&'a CStr]> {
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcesDir {
    pub app_path: Option<String>,
    pub host_path: Option<String>,
}

impl ResourcesDir {
    pub fn app(&self) -> Option<&str> {
        self.app_path.as_deref()
    }

    pub fn host(&self) -> Option<&str> {
        self.host_path.as_deref()
    }
}

pub fn resources_dir() -> io::Result<ResourcesDir> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "resources dir is not available on wasm",
    ))
}

pub fn is_flatpak() -> bool {
    false
}
