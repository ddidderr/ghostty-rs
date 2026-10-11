//! This structure represents the state required to render a terminal
//! grid using the font subsystem. It is "shared" because it is able to
//! be shared across multiple surfaces.
//!
//! It is desirable for the grid state to be shared because the font
//! configuration for a set of surfaces is almost always the same and
//! font data is relatively memory intensive. Further, the font subsystem
//! should be read-heavy compared to write-heavy, so it handles concurrent
//! reads well. Going even further, the font subsystem should be very rarely
//! read at all since it should only be necessary when the grid actively
//! changes.
//!
//! SharedGrid does NOT support resizing, font-family changes, font removals
//! in collections, etc. Because the Grid is shared this would cause a
//! major disruption in the rendering of multiple surfaces (i.e. increasing
//! the font size in one would increase it in all). In many cases this isn't
//! desirable so to implement configuration changes the grid should be
//! reinitialized and all surfaces should switch over to using that one.

#![allow(clippy::collapsible_if, clippy::if_not_else, clippy::float_cmp)]

use std::collections::HashMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::atlas::{self, Atlas};
use crate::codepoint_resolver::{CodepointResolver, RenderGlyphError};
use crate::collection::{self, Collection, GetFaceError, Index, PresentationMode};
use crate::face::{Constraint, ConstraintSize, RenderOptions};
use crate::{Glyph, Metrics, Presentation, SpriteFace, Style};

/// The size of a grid cell in pixels.
// TODO(integration): replace with `renderer::CellSize` once the renderer
// crate exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(transparent)]
    UpdateMetrics(#[from] collection::UpdateMetricsError),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    GetFace(#[from] GetFaceError),
    #[error(transparent)]
    Render(#[from] RenderGlyphError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Render {
    pub glyph: Glyph,
    pub presentation: Presentation,
}

/// The glyph cache key. Like upstream (`GlyphKey.Packed`), only the render
/// option fields that affect the identity of the render are part of the key:
/// the grid metrics and the constraint are NOT (the constraint is a function
/// of the codepoint/font, and the metrics are fixed for the grid's lifetime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    index: Index,
    glyph: u32,
    cell_width: u8,
    thicken: bool,
    thicken_strength: u8,
    constraint_width: u8,
}

impl GlyphKey {
    fn new(index: Index, glyph: u32, opts: &RenderOptions) -> GlyphKey {
        GlyphKey {
            index,
            glyph,
            cell_width: opts.cell_width.unwrap_or(0) & 0b11,
            thicken: opts.thicken,
            thicken_strength: opts.thicken_strength,
            constraint_width: opts.constraint_width & 0b11,
        }
    }
}

/// The lock-protected state of the grid. Callers that need direct access
/// (for example the renderer uploading the atlases) take the guard from
/// [`SharedGrid::read`] / [`SharedGrid::write`].
pub struct State {
    /// Cache for codepoints to font indexes in a group.
    pub codepoints: HashMap<(Style, u32, Option<Presentation>), Option<Index>>,

    /// Cache for glyph renders into the atlas.
    glyphs: HashMap<GlyphKey, Render>,

    /// The texture atlas to store renders in. The Glyph data in the glyphs
    /// cache is dependent on the atlas matching.
    pub atlas_grayscale: Atlas,
    pub atlas_color: Atlas,

    /// The underlying resolver for font data, fallbacks, etc. The shared
    /// grid takes ownership of the resolver.
    pub resolver: CodepointResolver,

    /// The currently active grid metrics dictating the layout of the grid.
    /// This is calculated based on the resolver and current fonts.
    pub metrics: Metrics,
}

pub struct SharedGrid {
    /// The RwLock used to protect the shared grid. Callers are expected to use
    /// this (through `read`/`write`) if they need to i.e. access the atlas
    /// directly. Because callers can use this lock directly, maintainers need
    /// to be extra careful to review call sites to ensure they are using the
    /// lock correctly.
    lock: RwLock<State>,
}

impl SharedGrid {
    /// Initialize the grid.
    ///
    /// The resolver must have a collection that supports deferred loading
    /// (`collection.load_options != None`). This is because we need the load
    /// options data to determine grid metrics and setup our sprite font.
    ///
    /// SharedGrid always configures the sprite font. This struct is expected
    /// to be used with a terminal grid and therefore the sprite font is
    /// always necessary for correct rendering.
    pub fn new(resolver: CodepointResolver) -> Result<SharedGrid, InitError> {
        // We need to support loading options since we use the size data
        assert!(resolver.collection.load_options.is_some());

        let mut state = State {
            // We set an initial capacity that can fit a good number of
            // characters. This number was picked empirically based on my own
            // terminal usage.
            codepoints: HashMap::with_capacity(128),
            glyphs: HashMap::with_capacity(128),
            atlas_grayscale: Atlas::new(512, atlas::Format::Grayscale),
            atlas_color: Atlas::new(512, atlas::Format::Bgra),
            resolver,
            // Loaded below
            metrics: Metrics::calc(crate::metrics::FaceMetrics::default()),
        };

        // Initialize our metrics.
        state.reload_metrics()?;

        Ok(SharedGrid {
            lock: RwLock::new(state),
        })
    }

    /// Take the shared (read) lock.
    pub fn read(&self) -> RwLockReadGuard<'_, State> {
        self.lock
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Take the exclusive (write) lock.
    pub fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.lock
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the grid cell size.
    pub fn cell_size(&self) -> CellSize {
        let st = self.read();
        CellSize {
            width: st.metrics.cell_width,
            height: st.metrics.cell_height,
        }
    }

    /// Returns a copy of the current grid metrics.
    pub fn metrics(&self) -> Metrics {
        self.read().metrics
    }

    /// Get the font index for a given codepoint. This is cached.
    ///
    /// This always forces loading any deferred fonts since we assume that if
    /// you're looking up an index that the caller plans to use the font. By
    /// loading the font in this function we can ensure thread-safety on the
    /// load without complicating future calls.
    pub fn get_index(
        &self,
        cp: u32,
        style: Style,
        p: Option<Presentation>,
    ) -> Result<Option<Index>, Error> {
        let key = (style, cp, p);

        // Fast path: the cache has the value. This is almost always true and
        // only requires a read lock.
        {
            let st = self.read();
            if let Some(v) = st.codepoints.get(&key) {
                return Ok(*v);
            }
        }

        // Slow path: we need to search this codepoint
        let mut guard = self.write();
        let st = &mut *guard;

        // Try to get it, if it is now in the cache another thread beat us to it.
        if let Some(v) = st.codepoints.get(&key) {
            return Ok(*v);
        }

        // Load a value and cache it. This even caches negative matches.
        let value = st.resolver.get_index(cp, style, p);
        st.codepoints.insert(key, value);

        if let Some(idx) = value {
            // If the font is a sprite font then we don't need to preload
            // because get_face doesn't work with special fonts.
            if idx.special().is_none() {
                // Load the face in case its deferred. If this fails then we
                // would've failed to load it in the future anyways so we want
                // to undo all the caching we did.
                if let Err(err) = st.resolver.collection.get_face(idx) {
                    st.codepoints.remove(&key);
                    return Err(err.into());
                }
            }
        }

        Ok(value)
    }

    /// Returns true if the given font index has the codepoint and presentation.
    pub fn has_codepoint(&self, idx: Index, cp: u32, p: Option<Presentation>) -> bool {
        let st = self.read();
        st.resolver.collection.has_codepoint(
            idx,
            cp,
            match p {
                Some(v) => PresentationMode::Explicit(v),
                None => PresentationMode::Any,
            },
        )
    }

    /// Render a codepoint. This uses the first font index that has the codepoint
    /// and matches the presentation requested. If the codepoint cannot be found
    /// in any font, an null render is returned.
    pub fn render_codepoint(
        &self,
        cp: u32,
        style: Style,
        p: Option<Presentation>,
        opts: RenderOptions,
    ) -> Result<Option<Render>, Error> {
        // Note: we could optimize the below to use way less locking, but
        // at the time of writing this codepath is only called for preedit
        // text which is relatively rare and almost non-existent in multiple
        // surfaces at the same time.

        // Get the font that has the codepoint
        let Some(index) = self.get_index(cp, style, p)? else {
            return Ok(None);
        };

        // Get the glyph for the font
        let glyph_index = {
            let mut guard = self.write();
            // (The face was preloaded by `get_index`; `get_face` is a no-op
            // lookup then. We need `&mut` for the possible lazy load.)
            let face = guard.resolver.collection.get_face(index)?;
            match face.glyph_index(cp) {
                Some(g) => g,
                None => return Ok(None),
            }
        };

        // Render
        Ok(Some(self.render_glyph(index, glyph_index, opts)?))
    }

    /// Render a glyph index. This automatically determines the correct texture
    /// atlas to use and caches the result.
    pub fn render_glyph(
        &self,
        index: Index,
        glyph_index: u32,
        opts: RenderOptions,
    ) -> Result<Render, Error> {
        let key = GlyphKey::new(index, glyph_index, &opts);

        // Fast path: the cache has the value. This is almost always true and
        // only requires a read lock.
        {
            let st = self.read();
            if let Some(v) = st.glyphs.get(&key) {
                return Ok(*v);
            }
        }

        // Slow path: we need to search this codepoint
        let mut guard = self.write();
        let st = &mut *guard;

        if let Some(v) = st.glyphs.get(&key) {
            return Ok(*v);
        }

        // Get the presentation to determine what atlas to use. (Nothing is
        // inserted into the cache until the render succeeds, which gives the
        // same roll-back behaviour as upstream's errdefer.)
        let p = st.resolver.get_presentation(index, glyph_index)?;
        let atlas: &mut Atlas = match p {
            Presentation::Text => &mut st.atlas_grayscale,
            Presentation::Emoji => &mut st.atlas_color,
        };

        let mut render_opts = opts;

        // Always use these constraints for emoji.
        if p == Presentation::Emoji {
            render_opts.constraint = Constraint {
                // Scale emoji to be as large as possible
                // while preserving their aspect ratio.
                size: ConstraintSize::Cover,

                // Center the emoji in its cells.
                align_horizontal: crate::face::Align::Center,
                align_vertical: crate::face::Align::Center,

                // Add a small bit of padding so the emoji
                // doesn't quite touch the edges of the cells.
                pad_left: 0.025,
                pad_right: 0.025,

                ..Constraint::NONE
            };
        }

        // Render into the atlas
        let glyph = match st
            .resolver
            .render_glyph(atlas, index, glyph_index, render_opts)
        {
            Ok(g) => g,
            // If the atlas is full, we resize it
            Err(err) if err.is_atlas_full() => {
                atlas.grow(atlas.size * 2);
                st.resolver
                    .render_glyph(atlas, index, glyph_index, render_opts)?
            }
            Err(err) => return Err(err.into()),
        };

        // Cache and return
        let render = Render {
            glyph,
            presentation: p,
        };
        st.glyphs.insert(key, render);
        Ok(render)
    }
}

impl State {
    fn reload_metrics(&mut self) -> Result<(), collection::UpdateMetricsError> {
        let collection: &mut Collection = &mut self.resolver.collection;
        collection.update_metrics()?;

        self.metrics = collection.metrics.expect("set by update_metrics");

        // Setup our sprite font.
        self.resolver.sprite = Some(SpriteFace::new(self.metrics));
        Ok(())
    }

    /// Number of cached glyph renders (test/diagnostic helper).
    pub fn glyph_cache_len(&self) -> usize {
        self.glyphs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::{AddOptions, LoadOptions, SizeAdjustment};
    use crate::face::{DesiredSize, Options};
    use crate::{Face, Library, embedded};

    fn test_grid(lib: &Library) -> SharedGrid {
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        c.add(
            Face::init(
                lib,
                embedded::REGULAR.into(),
                Options::new(DesiredSize {
                    points: 12.0,
                    xdpi: 96,
                    ydpi: 96,
                }),
            )
            .unwrap(),
            AddOptions {
                style: Style::Regular,
                fallback: false,
                size_adjustment: SizeAdjustment::None,
            },
        )
        .unwrap();

        SharedGrid::new(CodepointResolver::new(c)).unwrap()
    }

    #[test]
    fn get_index() {
        let lib = Library::init().unwrap();
        let grid = test_grid(&lib);

        // Visible ASCII.
        for i in 32u32..127 {
            let idx = grid.get_index(i, Style::Regular, None).unwrap().unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(0, idx.idx);
            assert!(grid.has_codepoint(idx, i, None));
        }

        // Do it again without a resolver set to ensure we only hit the cache
        // (upstream sets the resolver to `undefined`; we swap in an empty one).
        let old_resolver = std::mem::replace(
            &mut grid.write().resolver,
            CodepointResolver::new(Collection::new()),
        );
        for i in 32u32..127 {
            let idx = grid.get_index(i, Style::Regular, None).unwrap().unwrap();
            assert_eq!(Style::Regular, idx.style);
            assert_eq!(0, idx.idx);
        }
        grid.write().resolver = old_resolver;
    }

    #[test]
    fn render_glyph_error_after_cache_insert_rolls_back_cache_entry() {
        // Upstream uses a tripwire to fail `renderGlyph` after the cache entry
        // was inserted. Here a failure is provoked with an invalid index
        // (`get_presentation` fails); the cache must stay clean.
        let lib = Library::init().unwrap();
        let grid = test_grid(&lib);

        let idx = grid
            .get_index('A' as u32, Style::Regular, None)
            .unwrap()
            .unwrap();
        let glyph_index = {
            let st = grid.read();
            st.resolver
                .collection
                .get_loaded_face(idx)
                .unwrap()
                .glyph_index('A' as u32)
                .unwrap()
        };

        let render_opts = RenderOptions::new(grid.metrics());
        assert_eq!(0, grid.read().glyph_cache_len());

        let bad = Index {
            style: Style::Regular,
            idx: 100,
        };
        assert!(grid.render_glyph(bad, glyph_index, render_opts).is_err());
        assert_eq!(0, grid.read().glyph_cache_len());

        // And a good render is cached.
        let r = grid.render_glyph(idx, glyph_index, render_opts).unwrap();
        assert_eq!(Presentation::Text, r.presentation);
        assert_eq!(1, grid.read().glyph_cache_len());
        assert_eq!(r, grid.render_glyph(idx, glyph_index, render_opts).unwrap());
        assert_eq!(1, grid.read().glyph_cache_len());
    }

    #[test]
    fn render_codepoint_and_cell_size() {
        let lib = Library::init().unwrap();
        let grid = test_grid(&lib);
        let size = grid.cell_size();
        assert!(size.width > 0 && size.height > 0);

        let r = grid
            .render_codepoint(
                'A' as u32,
                Style::Regular,
                None,
                RenderOptions::new(grid.metrics()),
            )
            .unwrap()
            .unwrap();
        assert!(r.glyph.width > 0);
        // Not in any font and no discovery.
        assert!(
            grid.render_codepoint(
                0x10FFFE,
                Style::Regular,
                None,
                RenderOptions::new(grid.metrics())
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn atlas_grows_when_full() {
        let lib = Library::init().unwrap();
        let grid = test_grid(&lib);
        let metrics = grid.metrics();
        // Render sprite glyphs (box drawing) until the 512 atlas must grow.
        for cp in 0x2500u32..0x2580 {
            let idx = grid.get_index(cp, Style::Regular, None).unwrap().unwrap();
            grid.render_glyph(idx, cp, RenderOptions::new(metrics))
                .unwrap();
        }
        assert!(grid.read().atlas_grayscale.size >= 512);
    }

    #[test]
    fn init_error_without_primary_face() {
        // Analogue of upstream "init error": failure in `init` must surface
        // as an error (no primary font to compute metrics from).
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib));
        assert!(SharedGrid::new(CodepointResolver::new(c)).is_err());
    }
}
