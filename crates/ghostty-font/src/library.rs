//! A library represents the shared state that the underlying font
//! library implementation(s) require per-process. Port of `font/library.zig`.
//!
//! * FreeType backends: the `FT_Library` (plus a face-creation mutex).
//! * Backends without a library (CoreText, Canvas): an empty library
//!   (Zig: `NoopLibrary`).

use std::sync::{Arc, Mutex};

#[cfg(font_freetype)]
use crate::face::freetype::ft;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[cfg(font_freetype)]
    #[error("freetype error {0}")]
    Freetype(i32),
}

/// Shared font library state. Cheap to clone.
#[derive(Clone)]
pub struct Library {
    #[cfg_attr(not(font_freetype), allow(dead_code))]
    pub(crate) inner: Arc<LibraryInner>,
}

pub(crate) struct LibraryInner {
    #[cfg(font_freetype)]
    pub(crate) lib: ft::Lib,

    /// Mutex to be held any time the library is
    /// being used to create or destroy a face.
    pub(crate) mutex: Mutex<()>,
}

impl Library {
    #[cfg(font_freetype)]
    pub fn init() -> Result<Library, LibraryError> {
        let lib = ft::Lib::new().map_err(LibraryError::Freetype)?;
        Ok(Library {
            inner: Arc::new(LibraryInner {
                lib,
                mutex: Mutex::new(()),
            }),
        })
    }

    /// The CoreText backend has no per-process library state.
    #[cfg(not(font_freetype))]
    #[allow(clippy::unnecessary_wraps)]
    pub fn init() -> Result<Library, LibraryError> {
        Ok(Library {
            inner: Arc::new(LibraryInner {
                mutex: Mutex::new(()),
            }),
        })
    }

    /// Lock the library mutex (poisoning is ignored: the guarded state is
    /// only the font library's own).
    #[cfg_attr(not(font_freetype), allow(dead_code))]
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.inner
            .mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Library")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_clone() {
        let lib = Library::init().unwrap();
        let lib2 = lib.clone();
        drop(lib);
        drop(lib2.lock());
    }
}
