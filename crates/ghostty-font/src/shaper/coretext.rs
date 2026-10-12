//! CoreText shaper (Apple targets). Port of `font/shaper/coretext.zig`.
//!
//! CoreText shaping differs in subtle ways from HarfBuzz so it may result
//! in inconsistent rendering across platforms. But it also fixes many
//! issues (some macOS specific):
//!
//!   - Theta hat offset is incorrect in HarfBuzz but correct by default
//!     on macOS applications using CoreText. (See:
//!     <https://github.com/harfbuzz/harfbuzz/discussions/4525>)
//!
//!   - Hyphens (U+2010) can be synthesized by CoreText but not by HarfBuzz.
//!     See: <https://github.com/mitchellh/ghostty/issues/1643>
//!
//! The API mirrors [`crate::shaper::harfbuzz::Shaper`]: the run iterator
//! drives `prepare` / `add_codepoint` / `finalize` through
//! [`RunIteratorHook`](crate::shaper::run::RunIteratorHook) and the caller
//! shapes with [`Shaper::shape_run`]. The cluster/ligature heuristics live
//! in [`super::coretext_cells`] (unit tested on every platform).
#![allow(unsafe_code)]

use std::ptr::{self, NonNull};

use objc2_core_foundation::{
    CFArray,
    CFAttributedString,
    CFDictionary,
    CFNumber,
    CFRange,
    CFRetained,
    CFString,
    CFType,
    CGPoint,
    CGSize,
};
use objc2_core_text::{
    CTFont,
    CTFontDescriptor,
    CTLine,
    CTRun,
    CTRunStatus,
    CTTypesetter,
    kCTFontAttributeName,
    kCTFontFeatureSettingsAttribute,
    kCTFontOpenTypeFeatureTag,
    kCTFontOpenTypeFeatureValue,
    kCTTypesetterOptionForcedEmbeddingLevel,
};

use super::{
    cf_release::{Pooled, ReleaseThread, pooled},
    coretext_cells::{self as cells, CtRun, RunGlyph, RunState},
};
use crate::{
    coretext_util::CfBox,
    face::Face,
    shape::{self, Cell, DEFAULT_FEATURES, Feature, FeatureList, TextRun},
    shared_grid::SharedGrid,
};

#[derive(Debug, thiserror::Error)]
pub enum ShapeError {
    #[error("coretext object creation failed")]
    CoreTextFailed,
}

/// Errors from [`Shaper::shape_run`].
#[derive(Debug, thiserror::Error)]
pub enum ShapeRunError {
    #[error(transparent)]
    GetFace(#[from] crate::collection::GetFaceError),
    #[error(transparent)]
    Shape(#[from] ShapeError),
}

/// Create a CoreFoundation Dictionary suitable for settings the font
/// features of a CoreText font (`makeFeaturesDict`).
fn make_features_dict(feats: &[Feature]) -> Result<CfBox<CFDictionary>, ShapeError> {
    let mut items: Vec<CFRetained<CFDictionary<CFString, CFType>>> = Vec::new();

    for feat in feats {
        // Keys can only be ASCII.
        if !feat.tag.is_ascii() {
            return Err(ShapeError::CoreTextFailed);
        }
        let key = CFString::from_str(
            std::str::from_utf8(&feat.tag).map_err(|_| ShapeError::CoreTextFailed)?,
        );
        let value = CFNumber::new_i32(feat.value as i32);

        // SAFETY: valid static keys.
        let (k_tag, k_val) = unsafe { (kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue) };
        let dict = CFDictionary::<CFString, CFType>::from_slices(
            &[k_tag, k_val],
            &[key.as_ref(), value.as_ref()],
        );
        items.push(dict);
    }

    let item_refs: Vec<&CFDictionary<CFString, CFType>> = items.iter().map(|d| &**d).collect();
    let list = CFArray::from_objects(&item_refs);

    // SAFETY: valid static key.
    let k = unsafe { kCTFontFeatureSettingsAttribute };
    let dict = CFDictionary::<CFString, CFType>::from_slices(&[k], &[list.as_ref()]);
    // SAFETY: the attribute dictionary is used as an untyped CFDictionary.
    Ok(CfBox::new(unsafe {
        CFRetained::cast_unchecked::<CFDictionary>(dict)
    }))
}

/// Shaper that uses CoreText.
pub struct Shaper {
    /// The string used for shaping the current run.
    run_state: RunState,

    /// CoreFoundation Dictionary which represents our font feature settings.
    features: CfBox<CFDictionary>,
    /// A version of the features dictionary with the default features excluded.
    features_no_default: CfBox<CFDictionary>,

    /// The shared memory used for shaping results.
    cell_buf: Vec<Cell>,

    /// Cached attributes dict for creating CTTypesetter objects.
    /// The values in this never change so we can avoid overhead
    /// by just creating it once and saving it for reuse.
    typesetter_attr_dict: CfBox<CFDictionary>,

    /// List where we cache fonts, so we don't have to remake them for
    /// every single shaping operation.
    ///
    /// Fonts are cached as attribute dictionaries to be applied directly to
    /// attributed strings.
    cached_fonts: Vec<Option<CfBox<CFDictionary>>>,

    /// The grid that our cached fonts correspond to.
    /// If the grid changes then we need to reset our cache.
    cached_font_grid: usize,

    /// The list of CoreFoundation objects to release on the dedicated
    /// release thread. This is built up over the course of shaping and
    /// sent to the release thread when `end_frame` is called.
    cf_release_pool: Vec<Pooled>,

    /// Dedicated thread for releasing CoreFoundation objects. Some objects,
    /// such as those produced by CoreText, have excessively slow release
    /// callback logic.
    cf_release_thread: Option<ReleaseThread>,
}

impl Shaper {
    /// The cell buffer holds the shaped results.
    pub fn new(opts: &shape::Options) -> Result<Shaper, ShapeError> {
        let mut feature_list = FeatureList::default();
        for feature_str in &opts.features {
            feature_list.append_from_bytes(&feature_str.to_bytes());
        }

        // We need to construct two attrs dictionaries for font features;
        // one without the default features included, and one with them.
        let feats = &feature_list.features;
        let mut feats_df: Vec<Feature> = DEFAULT_FEATURES.to_vec();
        feats_df.extend_from_slice(feats);

        let features = make_features_dict(&feats_df)?;
        let features_no_default = make_features_dict(feats)?;

        // For now we only support LTR text. If we shape RTL text then
        // rendering will be very wrong so we need to explicitly force
        // LTR no matter what.
        //
        // See: https://github.com/mitchellh/ghostty/issues/1737
        // See: https://github.com/mitchellh/ghostty/issues/1442
        //
        // We used to do this by setting the writing direction attribute
        // on the attributed string we used, but it seems like that will
        // still allow some weird results, for example a single space at
        // the end of a line composed of RTL characters will be cause it
        // to output a run containing just that space, BEFORE it outputs
        // the rest of the line as a separate run, very weirdly with the
        // "right to left" flag set in the single space run's run status...
        //
        // So instead what we do is use a CTTypesetter to create our line,
        // using the kCTTypesetterOptionForcedEmbeddingLevel attribute to
        // force CoreText not to try doing any sort of BiDi, instead just
        // treat all text as embedding level 0 (left to right).
        let typesetter_attr_dict = {
            let num = CFNumber::new_i32(0);
            // SAFETY: valid static key.
            let k = unsafe { kCTTypesetterOptionForcedEmbeddingLevel };
            let d = CFDictionary::<CFString, CFType>::from_slices(&[k], &[num.as_ref()]);
            // SAFETY: used as an untyped CFDictionary.
            CfBox::new(unsafe { CFRetained::cast_unchecked::<CFDictionary>(d) })
        };

        Ok(Shaper {
            run_state: RunState::default(),
            features,
            features_no_default,
            cell_buf: Vec::new(),
            typesetter_attr_dict,
            cached_fonts: Vec::new(),
            cached_font_grid: 0,
            cf_release_pool: Vec::new(),
            cf_release_thread: ReleaseThread::new(),
        })
    }

    /// Send the release pool to the release thread.
    pub fn end_frame(&mut self) {
        if self.cf_release_pool.is_empty() {
            return;
        }

        // Get all the items in the pool as an owned vec so we can
        // send it to the dedicated release thread.
        let items = std::mem::take(&mut self.cf_release_pool);
        match &self.cf_release_thread {
            Some(t) => t.release(items),
            // No thread: release inline.
            None => drop(items),
        }
    }

    // ---- RunIterator hooks ----

    pub fn prepare(&mut self) {
        self.run_state.reset();
    }

    pub fn add_codepoint(&mut self, cp: u32, cluster: u32) {
        self.run_state.add_codepoint(cp, cluster);
    }

    #[allow(clippy::unused_self)]
    pub fn finalize(&mut self) {}

    /// Convenience: shape `text` as a single run where every item is a
    /// `(codepoint, cluster)` pair.
    pub fn shape_text(
        &mut self,
        face: Option<&Face>,
        text: &[(u32, u32)],
    ) -> Result<&[Cell], ShapeError> {
        self.prepare();
        for &(cp, cluster) in text {
            self.add_codepoint(cp, cluster);
        }
        self.finalize();
        self.shape(face)
    }

    /// Shape the given text run (`shape(run)` in Zig). The run must be the
    /// immediately previous run that was iterated since it shares state with
    /// the shaper.
    ///
    /// Note that this will accumulate garbage in the release pool. The
    /// caller must ensure you're properly calling `end_frame` to release
    /// all the objects.
    pub fn shape_run(
        &mut self,
        grid: &SharedGrid,
        run: &TextRun,
    ) -> Result<&[Cell], ShapeRunError> {
        // Special fonts aren't shaped and their codepoint == glyph so we
        // can just return the codepoints as-is.
        if run.font_index.special().is_some() {
            cells::special_cells(&self.run_state.codepoints, &mut self.cell_buf);
            return Ok(&self.cell_buf);
        }

        let attr_dict = self.get_font(grid, run.font_index)?;
        Ok(self.shape_with_attrs(&attr_dict)?)
    }

    /// Shape the current run (as built through the hooks) with the given
    /// face (uncached), or pass `None` for a special-case font (sprite).
    pub fn shape(&mut self, face: Option<&Face>) -> Result<&[Cell], ShapeError> {
        let Some(face) = face else {
            cells::special_cells(&self.run_state.codepoints, &mut self.cell_buf);
            return Ok(&self.cell_buf);
        };
        let attr_dict = self.make_attr_dict(face);
        self.shape_with_attrs(&attr_dict)
    }

    fn shape_with_attrs(&mut self, attr_dict: &CFDictionary) -> Result<&[Cell], ShapeError> {
        let state = &self.run_state;

        // Make room for the attributed string, CTTypesetter, and CTLine.
        self.cf_release_pool.reserve(4);

        // SAFETY: `unichars` is a valid buffer of `len` UTF-16 units.
        let str = unsafe {
            CFString::with_characters(None, state.unichars.as_ptr(), state.unichars.len() as isize)
        }
        .ok_or(ShapeError::CoreTextFailed)?;

        // Create an attributed string from our string
        // SAFETY: valid string and attribute dictionary.
        let attr_str = unsafe { CFAttributedString::new(None, Some(&str), Some(attr_dict)) }
            .ok_or(ShapeError::CoreTextFailed)?;

        // Create a typesetter from the attributed string and the cached
        // attr dict. (See comment in new for more info on the attr dict.)
        // SAFETY: valid string and options dictionary.
        let typesetter = unsafe {
            CTTypesetter::with_attributed_string_and_options(
                &attr_str,
                Some(&self.typesetter_attr_dict),
            )
        }
        .ok_or(ShapeError::CoreTextFailed)?;

        // Create a line from the typesetter
        // SAFETY: a zero length range means the whole string.
        let line: CFRetained<CTLine> = unsafe {
            typesetter.line(CFRange {
                location: 0,
                length: 0,
            })
        };

        // CoreText, despite our insistence with an enforced embedding level,
        // may sometimes output runs that are non-monotonic.
        //
        // CoreText may generate multiple runs even though our input to
        // CoreText is already split into runs by our own run iterator.
        // The runs as far as I can tell are always sequential to each
        // other so we can iterate over them and just append to our
        // cell buffer.
        // SAFETY: plain CoreText call.
        let glyph_runs = unsafe { line.glyph_runs() };
        // SAFETY: the glyph runs array contains CTRun objects.
        let glyph_runs: &CFArray<CTRun> = unsafe { glyph_runs.cast_unchecked() };
        let mut runs: Vec<CtRun> = Vec::with_capacity(glyph_runs.len());
        for i in 0..glyph_runs.len() {
            let Some(ctrun) = glyph_runs.get(i) else {
                continue;
            };
            runs.push(read_run(&ctrun));
        }

        cells::cells_from_runs(&self.run_state.codepoints, &runs, &mut self.cell_buf);

        // The objects are released on the release thread (see `end_frame`).
        self.cf_release_pool.push(pooled(str));
        self.cf_release_pool.push(pooled(attr_str));
        self.cf_release_pool.push(pooled(typesetter));
        self.cf_release_pool.push(pooled(line));

        Ok(&self.cell_buf)
    }

    /// Build an attr dict (kCTFontAttributeName -> font with our features)
    /// for the given face.
    fn make_attr_dict(&mut self, face: &Face) -> CfBox<CFDictionary> {
        let attrs = if face.quirks_disable_default_font_features {
            &self.features_no_default
        } else {
            &self.features
        };

        // SAFETY: valid attributes dictionary.
        let desc = unsafe { CTFontDescriptor::with_attributes(attrs) };
        // SAFETY: valid descriptor, null matrix is allowed.
        let copied: CFRetained<CTFont> = unsafe {
            face.ct_font()
                .copy_with_attributes(0.0, ptr::null(), Some(&desc))
        };

        // Get our font and use that get the attributes to set for the
        // attributed string so the whole string uses the same font.
        // SAFETY: valid static key.
        let k = unsafe { kCTFontAttributeName };
        let d = CFDictionary::<CFString, CFType>::from_slices(&[k], &[copied.as_ref()]);

        self.cf_release_pool.push(pooled(desc));
        self.cf_release_pool.push(pooled(copied));
        // SAFETY: used as an untyped CFDictionary.
        CfBox::new(unsafe { CFRetained::cast_unchecked::<CFDictionary>(d) })
    }

    /// Get an attr dict for a font from a specific index.
    /// These items are cached.
    fn get_font(
        &mut self,
        grid: &SharedGrid,
        index: crate::collection::Index,
    ) -> Result<CfBox<CFDictionary>, ShapeRunError> {
        // If this grid doesn't match the one we've cached fonts for,
        // then we reset the cache list since it's no longer valid.
        // We use an integer rather than keeping the pointer because we
        // don't want anyone to inadvertently use the pointer.
        let grid_id = ptr::from_ref(grid) as usize;
        if grid_id != self.cached_font_grid {
            if self.cached_font_grid > 0 {
                // Put all the currently cached fonts in to
                // the release pool before clearing the list.
                for ft in self.cached_fonts.drain(..).flatten() {
                    self.cf_release_pool.push(pooled(ft.0));
                }
            }

            self.cached_font_grid = grid_id;
        }

        let index_int = index.int() as usize;

        // The cached fonts are indexed directly by the font index, since
        // this number is usually low. Therefore, we set any index we haven't
        // seen to null.
        if self.cached_fonts.len() <= index_int {
            self.cached_fonts.resize_with(index_int + 1, || None);
        }

        // If we have it, return the cached attr dict.
        if let Some(cached) = &self.cached_fonts[index_int] {
            return Ok(cached.clone());
        }

        // The CoreText shaper relies on CoreText and CoreText claims
        // that CTFonts are threadsafe. See:
        // https://developer.apple.com/documentation/coretext/
        //
        // Quote:
        // All individual functions in Core Text are thread-safe. Font
        // objects (CTFont, CTFontDescriptor, and associated objects) can
        // be used simultaneously by multiple operations, work queues, or
        // threads. However, the layout objects (CTTypesetter, CTFramesetter,
        // CTRun, CTLine, CTFrame, and associated objects) should be used in
        // a single operation, work queue, or thread.
        //
        // Because of this, we only hold the grid lock to grab the face and
        // set it up, then release it.
        let attr_dict = {
            let mut guard = grid.write();
            let face = guard.resolver.collection.get_face(index)?;
            self.make_attr_dict(face)
        };

        self.cached_fonts[index_int] = Some(attr_dict.clone());
        Ok(attr_dict)
    }
}

impl Drop for Shaper {
    fn drop(&mut self) {
        // Release whatever is left in the pool on the release thread
        // (it is joined when the thread handle drops).
        let items = std::mem::take(&mut self.cf_release_pool);
        if !items.is_empty() {
            log::debug!("releasing {} remaining CF objects", items.len());
        }
        let cached = std::mem::take(&mut self.cached_fonts);
        let mut all = items;
        all.extend(cached.into_iter().flatten().map(|d| pooled(d.0)));
        if let Some(t) = &self.cf_release_thread {
            t.release(all);
        }
    }
}

/// Copy a CTRun's glyphs, advances, positions and string indices
/// (`getGlyphsPtr() orelse getGlyphs(alloc)` etc.).
fn read_run(ctrun: &CTRun) -> CtRun {
    // SAFETY: plain CoreText getters on a valid run.
    let (status, n) = unsafe { (ctrun.status(), ctrun.glyph_count() as usize) };
    let non_ltr = status.intersects(CTRunStatus::NonMonotonic | CTRunStatus::RightToLeft);
    if n == 0 {
        return CtRun {
            non_ltr,
            glyphs: Vec::new(),
        };
    }

    let full = CFRange {
        location: 0,
        length: 0,
    };
    let mut glyphs = vec![0u16; n];
    let mut advances = vec![CGSize::new(0.0, 0.0); n];
    let mut positions = vec![CGPoint::new(0.0, 0.0); n];
    let mut indices = vec![0isize; n];
    // SAFETY: each buffer holds `n` elements, which is the glyph count of the
    // run; a zero-length range copies everything.
    unsafe {
        ctrun.glyphs(full, NonNull::new_unchecked(glyphs.as_mut_ptr()));
        ctrun.advances(full, NonNull::new_unchecked(advances.as_mut_ptr()));
        ctrun.positions(full, NonNull::new_unchecked(positions.as_mut_ptr()));
        ctrun.string_indices(full, NonNull::new_unchecked(indices.as_mut_ptr()));
    }

    CtRun {
        non_ltr,
        glyphs: (0..n)
            .map(|i| RunGlyph {
                glyph: glyphs[i],
                advance_width: advances[i].width,
                x: positions[i].x,
                y: positions[i].y,
                index: indices[i] as usize,
            })
            .collect(),
    }
}
