//! A deferred face represents a single font face with all the information
//! necessary to load it, but defers loading the full face until it is
//! needed. Port of `font/DeferredFace.zig` (web_canvas variant, wasm32).

pub use crate::face::web_canvas::Face;
use crate::{
    Presentation,
    face::{DesiredSize, web_canvas::Error},
};

/// WebCanvas specific data.
#[derive(Debug, Clone)]
pub struct WebCanvas {
    /// The CSS "font" attribute, excluding size.
    pub font_str: String,

    /// The presentation for this font.
    pub presentation: Presentation,
}

#[derive(Debug, Default)]
pub struct DeferredFace {
    pub wc: Option<WebCanvas>,
}

impl DeferredFace {
    pub fn from_web_canvas(wc: WebCanvas) -> DeferredFace {
        DeferredFace { wc: Some(wc) }
    }

    /// Returns the family name of the font.
    pub fn family_name(&self) -> String {
        self.wc
            .as_ref()
            .map(|wc| wc.font_str.clone())
            .unwrap_or_default()
    }

    /// Returns the name of this face.
    pub fn name(&self) -> String {
        self.family_name()
    }

    /// Load the deferred font face.
    pub fn load(&mut self, size: DesiredSize) -> Result<Face, Error> {
        let wc = self
            .wc
            .as_ref()
            .expect("deferred face has no web canvas data");
        Face::init_named(&wc.font_str, size, wc.presentation)
    }

    /// Returns true if this face can satisfy the given codepoint and
    /// presentation. Canvas always has the codepoint because we have no way
    /// of really checking and we let the browser handle it.
    pub fn has_codepoint(&self, cp: u32, p: Option<Presentation>) -> bool {
        let Some(wc) = &self.wc else {
            // Unreachable upstream: the face MUST be loaded.
            unreachable!("deferred face without web canvas data");
        };

        // Fast-path if we have a specific presentation and we don't match,
        // then it is definitely not this face.
        if p.is_some_and(|desired| wc.presentation != desired) {
            return false;
        }

        // Slow-path: we initialize the font, render it, and check if it
        // works and the presentation matches.
        match Face::init_named(&wc.font_str, DesiredSize::new(12.0), wc.presentation) {
            Ok(face) => face.glyph_index(cp).is_some(),
            Err(err) => {
                log::warn!(
                    "failed to init face for codepoint check face={} err={err}",
                    wc.font_str
                );
                false
            }
        }
    }
}
