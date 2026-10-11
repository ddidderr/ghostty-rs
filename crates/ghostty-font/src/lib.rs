//! Font engine: metrics, atlas, sprite font, faces (FreeType), discovery
//! (fontconfig), collections, codepoint resolution, shaping (HarfBuzz) and
//! the shared glyph grid. Port of `ZIG/src/font/` for the Linux backend
//! (`fontconfig_freetype`).

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod atlas;
pub mod backend;
pub mod codepoint_map;
pub mod codepoint_resolver;
pub mod collection;
pub mod config_types;
pub mod deferred_face;
pub mod discovery;
pub mod embedded;
pub mod face;
pub mod fontconfig;
pub mod glyph;
pub mod library;
pub mod metrics;
pub mod nerd_font_attributes;
pub mod opentype;
pub mod shape;
pub mod shaper;
pub mod shared_grid;
pub mod shared_grid_set;
pub mod sprite;

pub use atlas::Atlas;
pub use backend::{Backend, OPTIONS as options};

pub use codepoint_map::CodepointMap;
pub use codepoint_resolver::CodepointResolver;
pub use collection::Collection;
pub use deferred_face::DeferredFace;
pub use discovery::{Descriptor, Discover};
pub use face::Face;
pub use glyph::Glyph;
pub use library::Library;
pub use metrics::Metrics;

pub use shared_grid::SharedGrid;
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
pub const SPRITE_INDEX: collection::Index =
    collection::Index::init_special(collection::Special::Sprite);

/// The default font size adjustment we use when loading fallback fonts.
///
/// TODO: Add user configuration for this instead of hard-coding it.
pub const DEFAULT_FALLBACK_ADJUSTMENT: collection::SizeAdjustment =
    collection::SizeAdjustment::IcWidth;
