//! A deferred face represents a single font face with all the information
//! necessary to load it, but defers loading the full face until it is
//! needed.
//!
//! Port of `font/DeferredFace.zig` (`freetype` backend: no discovery, so
//! there is no deferred data and every face must already be loaded).

use crate::{
    Face,
    Library,
    Presentation,
    face::{self, Options},
};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Face(#[from] face::FaceError),
}

/// With no discovery mechanism there is no data (all fields are `void` in
/// Zig for the `freetype` backend).
#[derive(Default)]
pub struct DeferredFace {}

impl DeferredFace {
    /// Returns the family name of the font.
    pub fn family_name(&self) -> Result<String, LoadError> {
        Ok(String::new())
    }

    /// Returns the name of this face.
    pub fn name(&self) -> Result<String, LoadError> {
        Ok(String::new())
    }

    /// Load the deferred font face.
    pub fn load(&mut self, _lib: &Library, _opts: Options) -> Result<Face, LoadError> {
        // Unreachable because we must be already loaded or have the
        // proper configuration for one of the other deferred mechanisms.
        unreachable!("deferred face without a discovery mechanism");
    }

    /// Returns true if this face can satisfy the given codepoint and
    /// presentation.
    pub fn has_codepoint(&self, _cp: u32, _p: Option<Presentation>) -> bool {
        // This is unreachable because discovery mechanisms terminate, and
        // if we're not using a discovery mechanism, the face MUST be loaded.
        unreachable!("deferred face without a discovery mechanism");
    }
}
