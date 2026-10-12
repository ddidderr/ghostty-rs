//! A font collection is a list of faces of different styles. The list is
//! ordered by priority (per style). All fonts in a collection share the same
//! size so they can be used interchangeably in cases a glyph is missing in one
//! and present in another.
//!
//! The purpose of a collection is to store a list of fonts by style
//! and priority order. A collection does not handle searching for font
//! callbacks, rasterization, etc. For this, see CodepointResolver.
//!
//! The collection can contain both loaded and deferred faces. Deferred faces
//! typically use less memory while still providing some necessary information
//! such as codepoint support, presentation, etc. This is useful for looking
//! for fallback fonts as efficiently as possible. For example, when the glyph
//! "X" is not found, we can quickly search through deferred fonts rather
//! than loading the font completely.

#![allow(clippy::collapsible_if, clippy::if_not_else, clippy::float_cmp)]

use crate::{
    DeferredFace,
    Face,
    Library,
    Metrics,
    Presentation,
    Style,
    config_types::{FontSyntheticStyle, FreetypeLoadFlags},
    face::{self, DesiredSize},
    metrics::{FaceMetrics, ModifierSet},
};

/// The available faces we have. This shouldn't be modified manually.
/// Instead, use the functions available on Collection.
///
/// (Zig: `StyleArray`, an enum array of segmented lists. Aliases are
/// stored as the [`Index`] of the non-alias entry they point to, so there
/// is no need for pointer stability.)
type StyleArray = [Vec<EntryOrAlias>; 4];

pub struct Collection {
    /// The available faces we have. This shouldn't be modified manually.
    /// Instead, use the functions available on Collection.
    faces: StyleArray,

    /// The metric modifiers to use for this collection.
    ///
    /// Call `Collection::update_metrics` to recompute the
    /// collection's metrics after making changes to these.
    pub metric_modifiers: ModifierSet,

    /// Metrics for this collection. Call `Collection::update_metrics` to
    /// (re)compute these after adding a primary font or making changes to
    /// `metric_modifiers`.
    pub metrics: Option<Metrics>,

    /// The face metrics for the primary face in the collection.
    ///
    /// We keep this around so we don't need to re-compute it when calculating
    /// the scale factor for additional fonts added to the collection.
    pub primary_face_metrics: Option<FaceMetrics>,

    /// The load options for deferred faces in the face list. If this
    /// is not set, then deferred faces will not be loaded. Attempting to
    /// add a deferred face will result in an error.
    pub load_options: Option<LoadOptions>,
}

impl Default for Collection {
    fn default() -> Self {
        Self::new()
    }
}

/// Options for adding a face to the collection.
#[derive(Debug, Clone, Copy)]
pub struct AddOptions {
    /// What style this face is.
    pub style: Style,
    /// What size adjustment to use.
    pub size_adjustment: SizeAdjustment,
    /// Whether this is a fallback face.
    pub fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AddError {
    /// There's no more room in the collection.
    #[error("collection is full")]
    CollectionFull,
    /// The call to `face.set_size` failed.
    #[error("set size failed")]
    SetSizeFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AddDeferredError {
    /// There's no more room in the collection.
    #[error("collection is full")]
    CollectionFull,
    /// `load_options` is None, can't do deferred loading.
    #[error("deferred loading unavailable")]
    DeferredLoadingUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// Index represents a special font (built-in) and these don't
    /// have an associated face. This should be caught upstream and use
    /// alternate logic.
    #[error("special font has no face")]
    SpecialHasNoFace,

    /// Invalid index.
    #[error("index out of bounds")]
    IndexOutOfBounds,
}

/// Errors from [`Collection::get_face`].
#[derive(Debug, thiserror::Error)]
pub enum GetFaceError {
    #[error(transparent)]
    Entry(#[from] EntryError),
    #[error("deferred loading unavailable")]
    DeferredLoadingUnavailable,
    #[error("face load failed: {0}")]
    Load(#[from] crate::deferred_face::LoadError),
    #[error("face error: {0}")]
    Face(#[from] face::FaceError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CompleteError {
    #[error("default face unavailable")]
    DefaultUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UpdateMetricsError {
    #[error("cannot load primary font")]
    CannotLoadPrimaryFont,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetSizeError {
    #[error(transparent)]
    UpdateMetrics(#[from] UpdateMetricsError),
    /// `load_options` is `None`.
    #[error("deferred loading unavailable")]
    DeferredLoadingUnavailable,
    /// The call to `face.set_size` failed.
    #[error("set size failed")]
    SetSizeFailed,
}

#[derive(Debug, thiserror::Error)]
enum SyntheticError {
    #[error("deferred loading unavailable")]
    DeferredLoadingUnavailable,
    #[error(transparent)]
    GetFace(#[from] GetFaceError),
    #[error(transparent)]
    Face(#[from] face::FaceError),
}

impl Collection {
    /// Initialize an empty collection.
    pub fn new() -> Collection {
        Collection {
            faces: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            metric_modifiers: ModifierSet::new(),
            metrics: None,
            primary_face_metrics: None,
            load_options: None,
        }
    }

    /// Number of entries (including aliases) for a style.
    pub fn count(&self, style: Style) -> usize {
        self.faces[style as usize].len()
    }

    /// Add a face to the collection. This face will be added next in priority
    /// if others exist already, i.e. it'll be the _last_ to be searched for a
    /// glyph in that list.
    ///
    /// If no error is encountered then the collection takes ownership of the
    /// face. (On error the face is dropped.)
    ///
    /// When added, the size of the face will be adjusted to match
    /// `load_options`.
    ///
    /// Returns the index for the added face.
    pub fn add(&mut self, face: Face, opts: AddOptions) -> Result<Index, AddError> {
        // We have some special indexes so we must never pass those.
        let idx = self.faces[opts.style as usize].len();
        if idx >= usize::from(Index::SPECIAL_START) - 1 {
            return Err(AddError::CollectionFull);
        }

        let mut owned_face = face;

        // Scale factor to adjust the size of the added face.
        let scale_factor = self.scale_factor(&owned_face.get_metrics(), opts.size_adjustment);

        // If we have load options, we update the size to ensure
        // it's matches and is normalized to the primary if possible.
        if let Some(load_opts) = &self.load_options {
            let mut new_opts = load_opts.clone();
            new_opts.size.points *= scale_factor as f32;
            owned_face
                .set_size(new_opts.face_options())
                .map_err(|_| AddError::SetSizeFailed)?;
        }

        self.faces[opts.style as usize].push(EntryOrAlias::Entry(Entry {
            face: AnyFace::Loaded(owned_face),
            fallback: opts.fallback,
            scale_factor: ScaleFactor::Scale(scale_factor),
        }));

        Ok(Index {
            style: opts.style,
            idx: idx as u16,
        })
    }

    /// Add a deferred face to the collection.
    ///
    /// Returns the index for the added face.
    pub fn add_deferred(
        &mut self,
        face: DeferredFace,
        opts: AddOptions,
    ) -> Result<Index, AddDeferredError> {
        if self.load_options.is_none() {
            return Err(AddDeferredError::DeferredLoadingUnavailable);
        }

        // We have some special indexes so we must never pass those.
        let idx = self.faces[opts.style as usize].len();
        if idx >= usize::from(Index::SPECIAL_START) - 1 {
            return Err(AddDeferredError::CollectionFull);
        }

        self.faces[opts.style as usize].push(EntryOrAlias::Entry(Entry {
            face: AnyFace::Deferred(face),
            fallback: opts.fallback,
            scale_factor: ScaleFactor::Adjustment(opts.size_adjustment),
        }));

        Ok(Index {
            style: opts.style,
            idx: idx as u16,
        })
    }

    /// Return the Face represented by a given Index. The returned reference
    /// is only valid as long as this collection is not modified.
    ///
    /// This will initialize the face if it is deferred and not yet loaded,
    /// which can fail.
    pub fn get_face(&mut self, index: Index) -> Result<&mut Face, GetFaceError> {
        let resolved = self.resolve(index)?;
        self.get_face_from_entry(resolved)
    }

    /// Return the face for an index only if it is already loaded (never
    /// triggers loading, so it only needs a shared reference). Returns `None`
    /// for special, out-of-range or still-deferred entries.
    pub fn get_loaded_face(&self, index: Index) -> Option<&Face> {
        let resolved = self.resolve(index).ok()?;
        match &self.entry_at(resolved).face {
            AnyFace::Loaded(f) => Some(f),
            AnyFace::Deferred(_) => None,
        }
    }

    /// Resolve an index (which might point at an alias) to the index of the
    /// non-alias entry.
    fn resolve(&self, index: Index) -> Result<Index, EntryError> {
        if index.special().is_some() {
            return Err(EntryError::SpecialHasNoFace);
        }
        let list = &self.faces[index.style as usize];
        match list.get(usize::from(index.idx)) {
            None => Err(EntryError::IndexOutOfBounds),
            Some(EntryOrAlias::Entry(_)) => Ok(index),
            Some(EntryOrAlias::Alias(target)) => Ok(*target),
        }
    }

    fn entry_at(&self, resolved: Index) -> &Entry {
        match &self.faces[resolved.style as usize][usize::from(resolved.idx)] {
            EntryOrAlias::Entry(e) => e,
            EntryOrAlias::Alias(_) => unreachable!("alias must point to a non-alias entry"),
        }
    }

    fn entry_at_mut(&mut self, resolved: Index) -> &mut Entry {
        match &mut self.faces[resolved.style as usize][usize::from(resolved.idx)] {
            EntryOrAlias::Entry(e) => e,
            EntryOrAlias::Alias(_) => unreachable!("alias must point to a non-alias entry"),
        }
    }

    /// Get the unaliased entry from an index
    pub fn get_entry(&mut self, index: Index) -> Result<&mut Entry, EntryError> {
        let resolved = self.resolve(index)?;
        Ok(self.entry_at_mut(resolved))
    }

    /// Get the unaliased entry from an index (shared).
    pub fn get_const_entry(&self, index: Index) -> Result<&Entry, EntryError> {
        let resolved = self.resolve(index)?;
        Ok(self.entry_at(resolved))
    }

    /// Get the face from an entry (loading it if deferred).
    ///
    /// This entry must not be an alias.
    fn get_face_from_entry(&mut self, entry: Index) -> Result<&mut Face, GetFaceError> {
        if matches!(self.entry_at(entry).face, AnyFace::Deferred(_)) {
            let mut opts = self
                .load_options
                .clone()
                .ok_or(GetFaceError::DeferredLoadingUnavailable)?;

            // Load the face.
            let mut face = match &mut self.entry_at_mut(entry).face {
                AnyFace::Deferred(d) => d.load(&opts.library, opts.face_options())?,
                AnyFace::Loaded(_) => unreachable!(),
            };

            // Calculate the scale factor for this
            // entry now that we have a loaded face.
            let mut scale = self.entry_at(entry).scale_factor;
            if let ScaleFactor::Adjustment(adj) = scale {
                let factor = self.scale_factor(&face.get_metrics(), adj);
                scale = ScaleFactor::Scale(factor);
                self.entry_at_mut(entry).scale_factor = scale;
            }

            // If our scale factor is something other
            // than 1.0 then we need to resize the face.
            if let ScaleFactor::Scale(s) = scale {
                if s != 1.0 {
                    opts.size.points *= s as f32;
                    face.set_size(opts.face_options())?;
                }
            }

            // Set the loaded face on the entry (dropping the deferred face).
            self.entry_at_mut(entry).face = AnyFace::Loaded(face);
        }

        match &mut self.entry_at_mut(entry).face {
            AnyFace::Loaded(f) => Ok(f),
            AnyFace::Deferred(_) => unreachable!(),
        }
    }

    /// Return the index of the font in this collection that contains
    /// the given codepoint, style, and presentation. If no font is found,
    /// None is returned.
    ///
    /// This does not trigger font loading; deferred fonts can be
    /// searched for codepoints.
    pub fn get_index(&self, cp: u32, style: Style, p_mode: PresentationMode) -> Option<Index> {
        for (i, entry_or_alias) in self.faces[style as usize].iter().enumerate() {
            if self.const_entry(entry_or_alias).has_codepoint(cp, p_mode) {
                return Some(Index {
                    style,
                    idx: i as u16,
                });
            }
        }

        // Not found
        None
    }

    fn const_entry<'a>(&'a self, e: &'a EntryOrAlias) -> &'a Entry {
        match e {
            EntryOrAlias::Entry(e) => e,
            EntryOrAlias::Alias(i) => self.entry_at(*i),
        }
    }

    /// Check if a specific font index has a specific codepoint. This does not
    /// necessarily force the font to load. The presentation value "p" will
    /// verify the Emoji representation matches if it is non-null. If "p" is
    /// null then any presentation will be accepted.
    pub fn has_codepoint(&self, index: Index, cp: u32, p_mode: PresentationMode) -> bool {
        let list = &self.faces[index.style as usize];
        match list.get(usize::from(index.idx)) {
            None => false,
            Some(e) => self.const_entry(e).has_codepoint(cp, p_mode),
        }
    }

    /// Ensure we have an option for all styles in the collection, such
    /// as italic and bold by synthesizing them if necessary from the
    /// first regular face that has text glyphs.
    ///
    /// If there is no regular face that has text glyphs, then this
    /// does nothing.
    pub fn complete_styles(
        &mut self,
        synthetic_config: FontSyntheticStyle,
    ) -> Result<(), CompleteError> {
        // If every style has at least one entry then we're done!
        // This is the most common case.
        if self.faces.iter().all(|l| !l.is_empty()) {
            return Ok(());
        }

        // Find the first regular face that has non-colorized text glyphs.
        // This is the font we want to fallback to. This may not be index zero
        // if a user configures something like an Emoji font first.
        let regular_entry: Index = 'entry: {
            let count = self.faces[Style::Regular as usize].len();
            if count == 0 {
                return Ok(());
            }

            // Find our first regular face that has text glyphs.
            for i in 0..count {
                // Load our face. If we fail to load it, we just skip it and
                // continue on to try the next one.
                let index = Index {
                    style: Style::Regular,
                    idx: i as u16,
                };
                let resolved = self.resolve(index).expect("index in range");
                let face = match self.get_face_from_entry(resolved) {
                    Ok(f) => f,
                    Err(err) => {
                        log::warn!("error loading regular entry={i} err={err}");
                        continue;
                    }
                };

                // We have two conditionals here. The color check is obvious:
                // we want to auto-italicize a normal text font. The second
                // check is less obvious... for mixed color/non-color fonts, we
                // accept the regular font if it has basic ASCII. This may not
                // be strictly correct (especially with international fonts) but
                // it's a reasonable heuristic and the first case will match 99%
                // of the time.
                if !face.has_color() || face.glyph_index('A' as u32).is_some() {
                    break 'entry resolved;
                }
            }

            // No regular text face found. We can't provide any fallback.
            return Err(CompleteError::DefaultUnavailable);
        };

        // If we don't have italic, attempt to create a synthetic italic face.
        // If we can't create a synthetic italic face, we'll just use the regular
        // face for italic.
        let have_italic = !self.faces[Style::Italic as usize].is_empty();
        if !have_italic {
            if !synthetic_config.italic {
                log::info!("italic style not available and synthetic italic disabled");
                self.faces[Style::Italic as usize].push(EntryOrAlias::Alias(regular_entry));
            } else {
                match self.synthetic_italic(regular_entry) {
                    Ok(synthetic) => {
                        log::info!("synthetic italic face created");
                        self.faces[Style::Italic as usize].push(EntryOrAlias::Entry(Entry {
                            face: AnyFace::Loaded(synthetic),
                            fallback: false,
                            scale_factor: ScaleFactor::Scale(1.0),
                        }));
                    }
                    Err(err) => {
                        log::warn!(
                            "failed to create synthetic italic, italic style will not be available err={err}"
                        );
                        self.faces[Style::Italic as usize].push(EntryOrAlias::Alias(regular_entry));
                    }
                }
            }
        }

        // If we don't have bold, use the regular font.
        let have_bold = !self.faces[Style::Bold as usize].is_empty();
        if !have_bold {
            if !synthetic_config.bold {
                log::info!("bold style not available and synthetic bold disabled");
                self.faces[Style::Bold as usize].push(EntryOrAlias::Alias(regular_entry));
            } else {
                match self.synthetic_bold(regular_entry) {
                    Ok(synthetic) => {
                        log::info!("synthetic bold face created");
                        self.faces[Style::Bold as usize].push(EntryOrAlias::Entry(Entry {
                            face: AnyFace::Loaded(synthetic),
                            fallback: false,
                            scale_factor: ScaleFactor::Scale(1.0),
                        }));
                    }
                    Err(err) => {
                        log::warn!(
                            "failed to create synthetic bold, bold style will not be available err={err}"
                        );
                        self.faces[Style::Bold as usize].push(EntryOrAlias::Alias(regular_entry));
                    }
                }
            }
        }

        // If we don't have bold italic, we attempt to synthesize a bold variant
        // of the italic font. If we can't do that, we'll use the italic font.
        if self.faces[Style::BoldItalic as usize].is_empty() {
            self.complete_bold_italic(synthetic_config, regular_entry, have_bold);
        }

        Ok(())
    }

    fn complete_bold_italic(
        &mut self,
        synthetic_config: FontSyntheticStyle,
        regular_entry: Index,
        have_bold: bool,
    ) {
        let bi = Style::BoldItalic as usize;
        if !synthetic_config.bold_italic {
            log::info!("bold italic style not available and synthetic bold italic disabled");
            self.faces[bi].push(EntryOrAlias::Alias(regular_entry));
            return;
        }

        // Prefer to synthesize on top of the face we already had. If we
        // have bold then we try to synthesize italic on top of bold.
        if have_bold {
            let base_entry = self
                .resolve(Index {
                    style: Style::Bold,
                    idx: 0,
                })
                .expect("bold entry exists");
            if let Ok(synthetic) = self.synthetic_italic(base_entry) {
                log::info!("synthetic bold italic face created from bold");
                self.faces[bi].push(EntryOrAlias::Entry(Entry {
                    face: AnyFace::Loaded(synthetic),
                    fallback: false,
                    scale_factor: ScaleFactor::Scale(1.0),
                }));
                return;
            }

            // If synthesizing italic failed, then we try to synthesize
            // bold on whatever italic font we have.
        }

        let base_entry = self
            .resolve(Index {
                style: Style::Italic,
                idx: 0,
            })
            .expect("italic entry exists");
        if let Ok(synthetic) = self.synthetic_bold(base_entry) {
            log::info!("synthetic bold italic face created from italic");
            self.faces[bi].push(EntryOrAlias::Entry(Entry {
                face: AnyFace::Loaded(synthetic),
                fallback: false,
                scale_factor: ScaleFactor::Scale(1.0),
            }));
            return;
        }

        log::warn!("bold italic style not available, using italic font");
        self.faces[bi].push(EntryOrAlias::Alias(base_entry));
    }

    /// Create a synthetic bold font face from the given entry and return it.
    fn synthetic_bold(&mut self, entry: Index) -> Result<Face, SyntheticError> {
        // We require loading options to create a synthetic bold face.
        let opts = self
            .load_options
            .clone()
            .ok_or(SyntheticError::DeferredLoadingUnavailable)?;

        // Try to bold it.
        let regular = self.get_face_from_entry(entry)?;

        // NOTE: upstream computes a `face_opts` with the size inherited from
        // the regular face but then (accidentally) passes `opts.faceOptions()`
        // instead; we mirror that behaviour.
        let face = regular.synthetic_bold(opts.face_options())?;

        log::info!("font synthetic bold created family={}", face.name());

        Ok(face)
    }

    /// Create a synthetic italic font face from the given entry and return it.
    fn synthetic_italic(&mut self, entry: Index) -> Result<Face, SyntheticError> {
        // We require loading options to create a synthetic italic face.
        let opts = self
            .load_options
            .clone()
            .ok_or(SyntheticError::DeferredLoadingUnavailable)?;

        // Try to italicize it.
        let regular = self.get_face_from_entry(entry)?;

        // See the NOTE in `synthetic_bold`.
        let face = regular.synthetic_italic(opts.face_options())?;

        log::info!("font synthetic italic created family={}", face.name());

        Ok(face)
    }

    /// Update the size of all faces in the collection. This will
    /// also update the size in the load options for future deferred
    /// face loading.
    ///
    /// This requires load options to be set.
    pub fn set_size(&mut self, size: DesiredSize) -> Result<(), SetSizeError> {
        // Get our options so we can modify the size.
        let opts = {
            let opts = self
                .load_options
                .as_mut()
                .ok_or(SetSizeError::DeferredLoadingUnavailable)?;
            opts.size = size;
            opts.clone()
        };

        let primary_entry = self
            .resolve(Index {
                style: Style::Regular,
                idx: 0,
            })
            .ok();

        // Resize all our faces that are loaded
        for style in Style::ALL {
            for i in 0..self.faces[style as usize].len() {
                // Resize all faces. We skip entries that are aliases, since
                // the underlying face will have a non-alias entry somewhere.
                if matches!(self.faces[style as usize][i], EntryOrAlias::Alias(_)) {
                    continue;
                }
                let this = Index {
                    style,
                    idx: i as u16,
                };

                let entry = self.entry_at_mut(this);
                let scale = entry.scale_factor;
                if let AnyFace::Loaded(face) = &mut entry.face {
                    // If this isn't our primary face, we scale
                    // the size appropriately before setting it.
                    //
                    // If we don't have a primary face we also don't.
                    let mut new_opts = opts.clone();
                    if primary_entry.is_some() && Some(this) != primary_entry {
                        if let ScaleFactor::Scale(s) = scale {
                            new_opts.size.points *= s as f32;
                        }
                    }
                    face.set_size(new_opts.face_options())
                        .map_err(|_| SetSizeError::SetSizeFailed)?;
                }
            }
        }

        Ok(self.update_metrics()?)
    }

    /// Calculate a factor by which to scale the provided face to match
    /// it with the primary face, depending on the specified adjustment.
    ///
    /// If this encounters any problems loading the primary face or its
    /// metrics then it just returns `1.0`.
    ///
    /// This functions very much like the `font-size-adjust` CSS property.
    /// ref: https://developer.mozilla.org/en-US/docs/Web/CSS/font-size-adjust
    fn scale_factor(&mut self, face_metrics: &FaceMetrics, adjustment: SizeAdjustment) -> f64 {
        // If there's no adjustment, the scale is 1.0
        if adjustment == SizeAdjustment::None {
            return 1.0;
        }

        // If we haven't calculated our primary face metrics yet, do so now.
        if self.primary_face_metrics.is_none() {
            // If we can't load the primary face, just use 1.0 as the scale factor.
            let Ok(primary_face) = self.get_face(Index::default()) else {
                return 1.0;
            };
            self.primary_face_metrics = Some(primary_face.get_metrics());
        }

        let primary_metrics = self.primary_face_metrics.expect("set above");

        // We normalize the metrics values which are expressed in px to instead
        // be in ems, so that it doesn't matter what size the faces actually are.
        let primary_scale = 1.0 / primary_metrics.px_per_em;
        let face_scale = 1.0 / face_metrics.px_per_em;

        // We get the target metrics from the primary face and this face depending
        // on the specified `adjustment`. If this face doesn't explicitly define a
        // metric, or if the value it defines is invalid, we fall through to other
        // options in the order below.
        //
        // In order to make sure the value is valid, we compare it with the result
        // of the estimator function, which rules out both null and invalid values.
        let mut adj = adjustment;
        let (primary_metric, face_metric) = loop {
            match adj {
                SizeAdjustment::IcWidth => {
                    if face_metrics.ic_width != Some(face_metrics.ic_width()) {
                        adj = SizeAdjustment::ExHeight;
                        continue;
                    }
                    break (
                        primary_metrics.ic_width() * primary_scale,
                        face_metrics.ic_width() * face_scale,
                    );
                }
                SizeAdjustment::ExHeight => {
                    if face_metrics.ex_height != Some(face_metrics.ex_height()) {
                        adj = SizeAdjustment::CapHeight;
                        continue;
                    }
                    break (
                        primary_metrics.ex_height() * primary_scale,
                        face_metrics.ex_height() * face_scale,
                    );
                }
                SizeAdjustment::CapHeight => {
                    if face_metrics.cap_height != Some(face_metrics.cap_height()) {
                        adj = SizeAdjustment::LineHeight;
                        continue;
                    }
                    break (
                        primary_metrics.cap_height() * primary_scale,
                        face_metrics.cap_height() * face_scale,
                    );
                }
                SizeAdjustment::LineHeight => {
                    break (
                        primary_metrics.line_height() * primary_scale,
                        face_metrics.line_height() * face_scale,
                    );
                }
                SizeAdjustment::None => unreachable!(),
            }
        };

        primary_metric / face_metric
    }

    /// Update the cell metrics for this collection, based on
    /// the primary font and the modifiers in `metric_modifiers`.
    ///
    /// This requires a primary font (index `0`) to be present.
    pub fn update_metrics(&mut self) -> Result<(), UpdateMetricsError> {
        let primary_face = self
            .get_face(Index::default())
            .map_err(|_| UpdateMetricsError::CannotLoadPrimaryFont)?;

        let fm = primary_face.get_metrics();
        self.primary_face_metrics = Some(fm);

        let mut metrics = Metrics::calc(fm);

        metrics.apply(&self.metric_modifiers);

        self.metrics = Some(metrics);
        Ok(())
    }
}

/// Options for adjusting the size of a face relative to the primary face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeAdjustment {
    /// Don't adjust the size for this face, use the original point size.
    None,
    /// Match ideograph character width with the primary face.
    IcWidth,
    /// Match ex height with the primary face.
    ExHeight,
    /// Match cap height with the primary face.
    CapHeight,
    /// Match line height with the primary face.
    LineHeight,
}

/// Load options are used to configure all the details a Collection
/// needs to load deferred faces.
#[derive(Clone)]
pub struct LoadOptions {
    /// The library to use for loading faces. This is not owned by
    /// the collection and can be used by multiple collections.
    pub library: Library,

    /// The desired font size for all loaded faces.
    pub size: DesiredSize,

    /// Freetype Load Flags to use when loading glyphs. This is a list of
    /// bitfield constants that controls operations to perform during glyph
    /// loading. Only a subset is exposed for configuration, for the whole set
    /// of flags see `pkg.freetype.face.LoadFlags`.
    pub freetype_load_flags: FreetypeLoadFlags,
}

impl LoadOptions {
    /// Options with the default size (12 points) and load flags.
    pub fn new(library: Library) -> LoadOptions {
        LoadOptions {
            library,
            size: DesiredSize::new(12.0),
            freetype_load_flags: face::FREETYPE_LOAD_FLAGS_DEFAULT,
        }
    }

    /// The options to use for loading faces.
    pub fn face_options(&self) -> face::Options {
        face::Options {
            size: self.size,
            freetype_load_flags: self.freetype_load_flags,
        }
    }
}

pub enum AnyFace {
    /// Not yet loaded.
    Deferred(DeferredFace),

    /// Loaded.
    Loaded(Face),
}

/// Factor to multiply the collection size by for a face, or else the size
/// adjustment that should be used to calculate it once the face is loaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScaleFactor {
    Adjustment(SizeAdjustment),
    Scale(f64),
}

/// A entry in a collection can be deferred or loaded. A deferred face
/// is not yet fully loaded and only represents the font descriptor
/// and usually uses less resources. A loaded face is fully parsed,
/// ready to rasterize, and usually uses more resources than a
/// deferred version.
///
/// A face can also be a "fallback" variant that is still either
/// deferred or loaded. Today, there is only one difference between
/// fallback and non-fallback (or "explicit") faces: the handling
/// of emoji presentation.
///
/// For explicit faces, when an explicit emoji presentation is
/// not requested, we will use any glyph for that codepoint found
/// even if the font presentation does not match the UCD
/// (Unicode Character Database) value. When an explicit presentation
/// is requested (via either VS15/V16), that is always honored.
/// The reason we do this is because we assume that if a user
/// explicitly chosen a font face (hence it is "explicit" and
/// not "fallback"), they want to use any glyphs possible within that
/// font face. Fallback fonts on the other hand are picked as a
/// last resort, so we should prefer exactness if possible.
pub struct Entry {
    pub face: AnyFace,

    /// Whether this face is a fallback face, see
    /// main doc comment on Entry for more info.
    pub fallback: bool,

    /// Factor to multiply the collection size by for this face, or
    /// else the size adjustment that should be used to calculate
    /// once the face is loaded.
    ///
    /// This is computed when the face is loaded, based on a scale
    /// factor computed for an adjustment from the primary face to
    /// this one, which allows fallback fonts to be harmonized with
    /// the primary font by matching one of the metrics between them.
    pub scale_factor: ScaleFactor,
}

impl Entry {
    /// If this face is loaded, then this returns the `Face`,
    /// otherwise returns None.
    pub fn get_loaded(&mut self) -> Option<&mut Face> {
        match &mut self.face {
            AnyFace::Deferred(_) => None,
            AnyFace::Loaded(face) => Some(face),
        }
    }

    /// True if the entry is deferred.
    pub fn is_deferred(&self) -> bool {
        matches!(self.face, AnyFace::Deferred(_))
    }

    /// True if this face satisfies the given codepoint and presentation.
    pub fn has_codepoint(&self, cp: u32, p_mode: PresentationMode) -> bool {
        let mut mode = p_mode;
        loop {
            match mode {
                PresentationMode::Default(p) => {
                    mode = if self.fallback {
                        // Fallback fonts require explicit presentation matching.
                        PresentationMode::Explicit(p)
                    } else {
                        // Non-fallback fonts do not.
                        PresentationMode::Any
                    };
                }
                PresentationMode::Explicit(p) => {
                    return match &self.face {
                        AnyFace::Deferred(v) => v.has_codepoint(cp, Some(p)),
                        AnyFace::Loaded(face) => {
                            let Some(index) = face.glyph_index(cp) else {
                                return false;
                            };
                            match p {
                                Presentation::Text => !face.is_color_glyph(index),
                                Presentation::Emoji => face.is_color_glyph(index),
                            }
                        }
                    };
                }
                PresentationMode::Any => {
                    return match &self.face {
                        AnyFace::Deferred(v) => v.has_codepoint(cp, None),
                        AnyFace::Loaded(face) => face.glyph_index(cp).is_some(),
                    };
                }
            }
        }
    }
}

pub enum EntryOrAlias {
    Entry(Entry),

    /// An alias to another entry. This is used to share the same face,
    /// avoid memory duplication. An alias must point to a non-alias entry.
    Alias(Index),
}

/// The requested presentation for a codepoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationMode {
    /// The codepoint has an explicit presentation that is required,
    /// i.e. VS15/V16.
    Explicit(Presentation),

    /// The codepoint has no explicit presentation and we should use
    /// the presentation from the UCD.
    Default(Presentation),

    /// The codepoint can be any presentation.
    Any,
}

/// The special-case fonts that we support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Special {
    /// Sprite drawing, this is rendered JIT using 2D graphics APIs.
    Sprite = Index::SPECIAL_START,
}

/// This represents a specific font in the collection.
///
/// The Zig version is a `packed struct(u16)` with a 3 bit style and 13 bit
/// index (up to 8192 fonts per style). Here the two parts are separate
/// fields, with [`Index::int`] producing the packed representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Index {
    pub style: Style,
    pub idx: u16,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            style: Style::Regular,
            idx: 0,
        }
    }
}

impl Index {
    /// The number of bits we use for the index.
    pub const IDX_BITS: u32 = 13;

    /// We start all special fonts at this index so they can be detected
    /// (`maxInt(IndexInt)`).
    pub const SPECIAL_START: u16 = (1 << Self::IDX_BITS) - 1;

    /// Initialize a special font index.
    pub const fn init_special(v: Special) -> Index {
        Index {
            style: Style::Regular,
            idx: v as u16,
        }
    }

    /// Convert to the packed int (style in the low 3 bits).
    pub fn int(self) -> u16 {
        (self.style as u16) | (self.idx << 3)
    }

    /// Returns Some if this is a "special" index which doesn't map to
    /// a real font face. We can still render it but there is no face for
    /// this font.
    pub fn special(self) -> Option<Special> {
        if self.idx < Self::SPECIAL_START {
            return None;
        }
        Some(Special::Sprite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        embedded,
        face::{DesiredSize, Options},
    };

    fn opts_96(points: f32) -> Options {
        Options::new(DesiredSize {
            points,
            xdpi: 96,
            ydpi: 96,
        })
    }

    fn add_opts(style: Style, adj: SizeAdjustment) -> AddOptions {
        AddOptions {
            style,
            fallback: false,
            size_adjustment: adj,
        }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    fn assert_metrics_approx(expected: &Metrics, actual: &Metrics) {
        assert_eq!(expected.cell_width, actual.cell_width);
        assert_eq!(expected.cell_height, actual.cell_height);
        assert_eq!(expected.cell_baseline, actual.cell_baseline);
        assert_eq!(expected.underline_position, actual.underline_position);
        assert_eq!(expected.underline_thickness, actual.underline_thickness);
        assert_eq!(
            expected.strikethrough_position,
            actual.strikethrough_position
        );
        assert_eq!(
            expected.strikethrough_thickness,
            actual.strikethrough_thickness
        );
        assert_eq!(expected.overline_position, actual.overline_position);
        assert_eq!(expected.overline_thickness, actual.overline_thickness);
        assert_eq!(expected.box_thickness, actual.box_thickness);
        assert_eq!(expected.cursor_height, actual.cursor_height);
        assert!(
            approx(expected.icon_height, actual.icon_height),
            "icon_height"
        );
        assert!(
            approx(expected.icon_height_single, actual.icon_height_single),
            "icon_height_single"
        );
        assert!(approx(expected.face_width, actual.face_width), "face_width");
        assert!(
            approx(expected.face_height, actual.face_height),
            "face_height"
        );
        assert!(approx(expected.face_y, actual.face_y), "face_y");
    }

    #[test]
    fn index_layout() {
        // We never want to take up more than a u16 since font indexes are
        // everywhere. The maximum number of fonts for a style is 13 bits.
        assert_eq!(2, std::mem::size_of::<u16>());
        assert_eq!(13, Index::IDX_BITS);
        assert_eq!(8191, Index::SPECIAL_START);
        let sprite = Index::init_special(Special::Sprite);
        assert_eq!(Some(Special::Sprite), sprite.special());
        assert_eq!(None, Index::default().special());
        assert_eq!(Index::SPECIAL_START << 3, sprite.int());
    }

    #[test]
    fn init() {
        let c = Collection::new();
        drop(c);
    }

    #[test]
    fn add_full() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();

        for _ in 0..(usize::from(Index::SPECIAL_START) - 1) {
            c.add(
                Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
                add_opts(Style::Regular, SizeAdjustment::None),
            )
            .unwrap();
        }

        let face = Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap();
        assert_eq!(
            Err(AddError::CollectionFull),
            c.add(face, add_opts(Style::Regular, SizeAdjustment::None))
        );
    }

    #[test]
    fn add_deferred_without_loading_options() {
        let mut c = Collection::new();
        // The deferred face can be anything since it should never be accessed.
        let deferred = DeferredFace::default();
        assert_eq!(
            Err(AddDeferredError::DeferredLoadingUnavailable),
            c.add_deferred(deferred, add_opts(Style::Regular, SizeAdjustment::None))
        );
    }

    #[test]
    fn get_face() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        let idx = c
            .add(
                Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
                add_opts(Style::Regular, SizeAdjustment::None),
            )
            .unwrap();

        let p1 = std::ptr::from_ref::<Face>(c.get_face(idx).unwrap());
        let p2 = std::ptr::from_ref::<Face>(c.get_face(idx).unwrap());
        assert_eq!(p1, p2);
    }

    #[test]
    fn get_index() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.add(
            Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        // Should find all visible ASCII
        for i in 32u32..127 {
            assert!(
                c.get_index(i, Style::Regular, PresentationMode::Any)
                    .is_some()
            );
        }

        // Should not find emoji
        assert!(
            c.get_index('🥸' as u32, Style::Regular, PresentationMode::Any)
                .is_none()
        );
    }

    #[test]
    fn complete_styles() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        c.add(
            Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        let a = 'A' as u32;
        assert!(c.get_index(a, Style::Bold, PresentationMode::Any).is_none());
        assert!(
            c.get_index(a, Style::Italic, PresentationMode::Any)
                .is_none()
        );
        assert!(
            c.get_index(a, Style::BoldItalic, PresentationMode::Any)
                .is_none()
        );
        c.complete_styles(FontSyntheticStyle::default()).unwrap();
        assert!(c.get_index(a, Style::Bold, PresentationMode::Any).is_some());
        assert!(
            c.get_index(a, Style::Italic, PresentationMode::Any)
                .is_some()
        );
        assert!(
            c.get_index(a, Style::BoldItalic, PresentationMode::Any)
                .is_some()
        );
    }

    #[test]
    fn complete_styles_synthetic_disabled_uses_aliases() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));
        c.add(
            Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        c.complete_styles(FontSyntheticStyle {
            bold: false,
            italic: false,
            bold_italic: false,
        })
        .unwrap();
        for style in [Style::Bold, Style::Italic, Style::BoldItalic] {
            assert_eq!(1, c.count(style));
            let idx = c
                .get_index('A' as u32, style, PresentationMode::Any)
                .unwrap();
            // The alias resolves to the regular entry's face.
            let p1 = std::ptr::from_ref::<Face>(c.get_face(idx).unwrap());
            let p2 = std::ptr::from_ref::<Face>(c.get_face(Index::default()).unwrap());
            assert_eq!(p1, p2);
        }
    }

    #[test]
    fn set_size() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        c.add(
            Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        assert_eq!(12.0, c.load_options.as_ref().unwrap().size.points);
        c.set_size(DesiredSize::new(24.0)).unwrap();
        assert_eq!(24.0, c.load_options.as_ref().unwrap().size.points);
    }

    #[test]
    fn has_codepoint() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        let idx = c
            .add(
                Face::init(&lib, embedded::REGULAR.into(), opts_96(12.0)).unwrap(),
                add_opts(Style::Regular, SizeAdjustment::None),
            )
            .unwrap();

        assert!(c.has_codepoint(idx, 'A' as u32, PresentationMode::Any));
        assert!(!c.has_codepoint(idx, '🥸' as u32, PresentationMode::Any));
    }

    #[cfg(not(font_coretext))]
    #[test]
    fn has_codepoint_emoji_default_graphical() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        c.load_options = Some(LoadOptions::new(lib.clone()));

        let idx = c
            .add(
                Face::init(&lib, embedded::EMOJI.into(), opts_96(12.0)).unwrap(),
                add_opts(Style::Regular, SizeAdjustment::None),
            )
            .unwrap();

        assert!(!c.has_codepoint(idx, 'A' as u32, PresentationMode::Any));
        assert!(c.has_codepoint(idx, '🥸' as u32, PresentationMode::Any));
        // TODO(fontmem): test explicit/implicit
    }

    #[test]
    fn metrics() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        let size = DesiredSize {
            points: 12.0,
            xdpi: 96,
            ydpi: 96,
        };
        let mut lo = LoadOptions::new(lib.clone());
        lo.size = size;
        c.load_options = Some(lo);

        c.add(
            Face::init(&lib, embedded::INCONSOLATA.into(), Options::new(size)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        c.update_metrics().unwrap();

        let base = Metrics {
            cell_width: 8,
            // The cell height is 17 px because the calculation is
            //
            //  ascender - descender + gap
            //
            // which, for inconsolata is
            //
            //  859 - -190 + 0
            //
            // font units, at 1000 units per em that works out to 1.049 em,
            // and 1em should be the point size * dpi scale, so 12 * (96/72)
            // which is 16, and 16 * 1.049 = 16.784, which finally is rounded
            // to 17.
            //
            // The icon height is (2 * cap_height + face_height) / 3
            // = (2 * 623 + 1049) / 3 = 765, and 16 * 0.765 = 12.24.
            cell_height: 17,
            cell_baseline: 3,
            underline_position: 17,
            underline_thickness: 1,
            strikethrough_position: 10,
            strikethrough_thickness: 1,
            overline_position: 0,
            overline_thickness: 1,
            box_thickness: 1,
            cursor_height: 17,
            icon_height: 16.784,
            icon_height_single: 12.24,
            face_width: 8.0,
            face_height: 16.784,
            face_y: -0.04,
            cursor_thickness: 1,
        };
        assert_metrics_approx(&base, &c.metrics.unwrap());

        // Resize should change metrics
        c.set_size(DesiredSize {
            points: 24.0,
            xdpi: 96,
            ydpi: 96,
        })
        .unwrap();
        let big = Metrics {
            cell_width: 16,
            cell_height: 34,
            cell_baseline: 6,
            underline_position: 34,
            underline_thickness: 2,
            strikethrough_position: 19,
            strikethrough_thickness: 2,
            overline_position: 0,
            overline_thickness: 2,
            box_thickness: 2,
            cursor_height: 34,
            icon_height: 33.568,
            icon_height_single: 24.48,
            face_width: 16.0,
            face_height: 33.568,
            face_y: -0.08,
            cursor_thickness: 1,
        };
        assert_metrics_approx(&big, &c.metrics.unwrap());
    }

    // TODO: Also test CJK fallback sizing, we don't currently have a CJK test font.
    #[test]
    fn adjusted_sizes() {
        let lib = Library::init().unwrap();
        let mut c = Collection::new();
        let size = DesiredSize {
            points: 12.0,
            xdpi: 96,
            ydpi: 96,
        };
        let mut lo = LoadOptions::new(lib.clone());
        lo.size = size;
        c.load_options = Some(lo);

        // Add our primary face.
        c.add(
            Face::init(&lib, embedded::INCONSOLATA.into(), Options::new(size)).unwrap(),
            add_opts(Style::Regular, SizeAdjustment::None),
        )
        .unwrap();

        c.update_metrics().unwrap();

        for metric in [SizeAdjustment::ExHeight, SizeAdjustment::CapHeight] {
            let pick = |m: &FaceMetrics| match metric {
                SizeAdjustment::ExHeight => m.ex_height.unwrap(),
                SizeAdjustment::CapHeight => m.cap_height.unwrap(),
                _ => unreachable!(),
            };

            // Add the fallback face with the chosen adjustment metric.
            let fallback_idx = c
                .add(
                    Face::init(&lib, embedded::MONASPACE_NEON.into(), Options::new(size)).unwrap(),
                    add_opts(Style::Regular, metric),
                )
                .unwrap();

            // The chosen metric should match.
            {
                let pm = c.get_face(Index::default()).unwrap().get_metrics();
                let fm = c.get_face(fallback_idx).unwrap().get_metrics();
                // We accept anything within half a pixel.
                assert!((pick(&pm) - pick(&fm)).abs() <= 0.5);
            }

            // Resize should keep that relationship.
            c.set_size(DesiredSize {
                points: 37.0,
                xdpi: 96,
                ydpi: 96,
            })
            .unwrap();
            {
                let pm = c.get_face(Index::default()).unwrap().get_metrics();
                let fm = c.get_face(fallback_idx).unwrap().get_metrics();
                assert!((pick(&pm) - pick(&fm)).abs() <= 0.5);
            }

            // Reset size for the next iteration
            c.set_size(size).unwrap();
        }

        {
            // A reference metric of "none" should leave the size unchanged.
            let fallback_idx = c
                .add(
                    Face::init(&lib, embedded::MONASPACE_NEON.into(), Options::new(size)).unwrap(),
                    add_opts(Style::Regular, SizeAdjustment::None),
                )
                .unwrap();

            let primary_points = c.get_face(Index::default()).unwrap().size.points;
            assert_eq!(
                primary_points,
                c.get_face(fallback_idx).unwrap().size.points
            );

            // Resize should keep that.
            c.set_size(DesiredSize {
                points: 37.0,
                xdpi: 96,
                ydpi: 96,
            })
            .unwrap();

            let primary_points = c.get_face(Index::default()).unwrap().size.points;
            assert_eq!(
                primary_points,
                c.get_face(fallback_idx).unwrap().size.points
            );

            // Reset collection size
            c.set_size(size).unwrap();
        }

        // Add the symbol face.
        let symbol_idx = c
            .add(
                Face::init(&lib, embedded::SYMBOLS_NERD_FONT.into(), Options::new(size)).unwrap(),
                add_opts(Style::Regular, SizeAdjustment::ExHeight),
            )
            .unwrap();

        // Test fallback to line_height() (ex_height and cap_height not defined
        // in symbols font).
        {
            let pm = c.get_face(Index::default()).unwrap().get_metrics();
            let sm = c.get_face(symbol_idx).unwrap().get_metrics();
            // We accept anything within half a pixel.
            assert!((pm.line_height() - sm.line_height()).abs() <= 0.5);
        }
    }
}
