//! Font engine: metrics, atlas, sprite font, faces (FreeType), discovery
//! (fontconfig), collections, codepoint resolution, shaping (HarfBuzz) and
//! the shared glyph grid. Port of `ZIG/src/font/` for the Linux backend
//! (`fontconfig_freetype`).

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod atlas;
pub mod backend;
pub mod codepoint_map;
#[cfg(not(target_arch = "wasm32"))]
pub mod codepoint_resolver;
#[cfg(not(target_arch = "wasm32"))]
pub mod collection;
pub mod config_types;
#[cfg(not(target_arch = "wasm32"))]
pub mod deferred_face;
#[cfg(target_arch = "wasm32")]
#[path = "deferred_face_web_canvas.rs"]
pub mod deferred_face;
#[cfg(not(target_arch = "wasm32"))]
pub mod discovery;
#[cfg(not(target_arch = "wasm32"))]
pub mod embedded;
pub mod face;
#[cfg(not(target_arch = "wasm32"))]
pub mod fontconfig;
pub mod glyph;
#[cfg(not(target_arch = "wasm32"))]
pub mod library;
pub mod metrics;
pub mod nerd_font_attributes;
pub mod opentype;
pub mod shape;
pub mod shaper;
#[cfg(not(target_arch = "wasm32"))]
pub mod shared_grid;
#[cfg(not(target_arch = "wasm32"))]
pub mod shared_grid_set;
pub mod sprite;

pub use atlas::Atlas;
pub use backend::{Backend, OPTIONS as options};
pub use codepoint_map::CodepointMap;
#[cfg(not(target_arch = "wasm32"))]
pub use codepoint_resolver::CodepointResolver;
#[cfg(not(target_arch = "wasm32"))]
pub use collection::Collection;
pub use deferred_face::DeferredFace;
#[cfg(not(target_arch = "wasm32"))]
pub use discovery::{Descriptor, Discover};
pub use face::Face;
pub use glyph::Glyph;
#[cfg(not(target_arch = "wasm32"))]
pub use library::Library;
pub use metrics::Metrics;
#[cfg(not(target_arch = "wasm32"))]
pub use shared_grid::SharedGrid;
#[cfg(not(target_arch = "wasm32"))]
pub use shared_grid_set::SharedGridSet;
pub use sprite::{Face as SpriteFace, Sprite};

/// The styles that a family can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Style {
    Regular = 0,
    Bold = 1,
    Italic = 2,
    BoldItalic = 3,
}

impl Style {
    /// All styles, in discriminant order (Zig: `std.meta.fields(Style)`).
    pub const ALL: [Style; 4] = [
        Style::Regular,
        Style::Bold,
        Style::Italic,
        Style::BoldItalic,
    ];
}

/// The presentation for an emoji.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presentation {
    /// U+FE0E
    Text = 0,
    /// U+FE0F
    Emoji = 1,
}

/// A FontIndex that can be used to use the sprite font directly.
#[cfg(not(target_arch = "wasm32"))]
pub const SPRITE_INDEX: collection::Index =
    collection::Index::init_special(collection::Special::Sprite);

/// The default font size adjustment we use when loading fallback fonts.
///
/// TODO: Add user configuration for this instead of hard-coding it.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_FALLBACK_ADJUSTMENT: collection::SizeAdjustment =
    collection::SizeAdjustment::IcWidth;
