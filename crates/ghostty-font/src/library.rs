//! A library represents the shared state that the underlying font
//! library implementation(s) require per-process. Port of `font/library.zig`
//! (the `FreetypeLibrary` variant).

use std::sync::{Arc, Mutex};

use crate::face::freetype::ft;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("freetype error {0}")]
    Freetype(i32),
}

/// Shared FreeType library state. Cheap to clone.
#[derive(Clone)]
pub struct Library {
    pub(crate) inner: Arc<LibraryInner>,
}

pub(crate) struct LibraryInner {
    pub(crate) lib: ft::Lib,

    /// Mutex to be held any time the library is
    /// being used to create or destroy a face.
    pub(crate) mutex: Mutex<()>,
}

impl Library {
    pub fn init() -> Result<Library, LibraryError> {
        let lib = ft::Lib::new().map_err(LibraryError::Freetype)?;
        Ok(Library {
            inner: Arc::new(LibraryInner {
                lib,
                mutex: Mutex::new(()),
            }),
        })
    }

    /// Lock the library mutex (poisoning is ignored: the guarded state is
    /// only FreeType's own).
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
