//! API-compatible stand-ins for OS facilities that do not exist on wasm32,
//! so crates that are also built for wasm (`ghostty-config`) compile
//! unchanged. In Zig these are simply never analyzed on wasm
//! (`os/passwd.zig` even `@compileError`s); here each reports "unavailable"
//! the way the Zig callers' error paths expect.

use std::io;

use crate::osbytes::EncodedOs;

/// `homedir.zig` is unavailable: nothing can be expanded.
pub mod homedir {
    use crate::osbytes::EncodedOs;

    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum ExpandError {
        #[error("home detection failed")]
        HomeDetectionFailed,
        #[error("buffer too small")]
        BufferTooSmall,
    }

    /// Paths without a leading `~/` pass through; others fail.
    pub fn expand_home(path: &EncodedOs) -> Result<EncodedOs, ExpandError> {
        if path.starts_with_plain("~/") {
            Err(ExpandError::HomeDetectionFailed)
        } else {
            Ok(path.clone())
        }
    }
}

pub mod passwd {
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Entry {
        pub shell: Option<crate::osbytes::EncodedOs>,
        pub home: Option<crate::osbytes::EncodedOs>,
        pub name: Option<crate::osbytes::EncodedOs>,
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
        pub home: Option<&'a crate::osbytes::EncodedOs>,
    }

    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum Error {
        #[error("xdg directories are not available on wasm")]
        Unavailable,
    }

    pub fn config(_: &Options<'_>) -> Result<crate::osbytes::EncodedOs, Error> {
        Err(Error::Unavailable)
    }

    pub fn cache(_: &Options<'_>) -> Result<crate::osbytes::EncodedOs, Error> {
        Err(Error::Unavailable)
    }

    pub fn state(_: &Options<'_>) -> Result<crate::osbytes::EncodedOs, Error> {
        Err(Error::Unavailable)
    }

    /// No `xdg-terminal-exec` mode on wasm.
    pub fn parse_terminal_exec<'a>(_: &'a [&'a CStr]) -> Option<&'a [&'a CStr]> {
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcesDir {
    pub app_path: Option<EncodedOs>,
    pub host_path: Option<EncodedOs>,
}

impl ResourcesDir {
    pub fn app(&self) -> Option<&EncodedOs> {
        self.app_path.as_ref()
    }

    pub fn host(&self) -> Option<&EncodedOs> {
        self.host_path.as_ref()
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
