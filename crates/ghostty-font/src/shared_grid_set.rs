//! This structure contains a set of SharedGrid structures keyed by
//! unique font configuration. Port of `font/SharedGridSet.zig`.
//!
//! Most terminals (surfaces) will share the same font configuration.
//! This structure allows expensive font information such as
//! the font atlas, glyph cache, font faces, etc. to be shared.
//!
//! This structure is thread-safe when the operations are documented
//! as thread-safe.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex, MutexGuard},
};

use ghostty_config::{
    config::Config,
    font_types::{Variation, VariationId},
    types_a::{
        FontStyle,
        FontSyntheticStyle,
        RepeatableCodepointMap,
        RepeatableFontVariation,
        RepeatableString,
    },
    types_b::FreetypeLoadFlags,
};

use crate::{
    Face,
    Style,
    codepoint_map::CodepointMap,
    codepoint_resolver::{CodepointResolver, StyleStatus},
    collection::{self, AddOptions, Collection, LoadOptions, SizeAdjustment},
    discovery::{Descriptor, Discover, Fontconfig},
    embedded,
    face::{self, DesiredSize, FontData},
    library::{Library, LibraryError},
    metrics::{Key as MetricsKey, Modifier, ModifierSet},
    shared_grid::{self, SharedGrid},
};

/// Errors from [`SharedGridSet::ref_`].
#[derive(Debug, thiserror::Error)]
pub enum RefError {
    #[error(transparent)]
    Discover(#[from] crate::discovery::DiscoverError),
    #[error("font discovery failed: {0}")]
    DiscoverNext(#[from] crate::fontconfig::Error),
    #[error(transparent)]
    Add(#[from] collection::AddError),
    #[error(transparent)]
    AddDeferred(#[from] collection::AddDeferredError),
    #[error(transparent)]
    Complete(#[from] collection::CompleteError),
    #[error(transparent)]
    GetFace(#[from] collection::GetFaceError),
    #[error(transparent)]
    Face(#[from] face::freetype::FaceError),
    #[error(transparent)]
    Grid(#[from] shared_grid::InitError),
    #[error(transparent)]
    Load(#[from] crate::deferred_face::LoadError),
}

pub type InitError = LibraryError;

/// Ref-counted SharedGrid.
struct ReffedGrid {
    grid: Arc<SharedGrid>,
    ref_count: u32,
}

struct Inner {
    /// The map of font configurations to SharedGrid instances. As in
    /// Zig, key equality is equality of the key hashcode.
    map: HashMap<Key, ReffedGrid>,

    /// Font discovery mechanism. Initialized once and kept for the
    /// lifetime of the set because some font discovery mechanisms
    /// (i.e. fontconfig) are unsafe to reinit.
    font_discover: Option<Discover>,
}

pub struct SharedGridSet {
    /// The font library that is used for all font groups.
    font_lib: Library,

    /// Lock to protect multi-threaded access to the map.
    inner: Mutex<Inner>,
}

impl SharedGridSet {
    /// Initialize a new SharedGridSet.
    pub fn new() -> Result<SharedGridSet, InitError> {
        Ok(SharedGridSet {
            font_lib: Library::init()?,
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                font_discover: None,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the number of cached grids.
    pub fn count(&self) -> usize {
        self.lock().map.len()
    }

    /// Initialize a SharedGrid for the given font configuration. If the
    /// SharedGrid is not present it will be initialized with a ref count of
    /// 1. If it is present, the ref count will be incremented.
    ///
    /// (Zig: `ref`; renamed since `ref` is a Rust keyword.) The returned
    /// key must be passed to [`SharedGridSet::deref`] when done. The grid
    /// is shared through an `Arc`; the set drops its own reference when the
    /// ref count reaches zero.
    pub fn ref_(
        &self,
        config: &DerivedConfig,
        font_size: DesiredSize,
    ) -> Result<(Key, Arc<SharedGrid>), RefError> {
        let key = Key::new(config, font_size);

        let mut inner = self.lock();
        if let Some(entry) = inner.map.get_mut(&key) {
            log::debug!("found cached grid for font config");

            // Increment our ref count and return the cache
            entry.ref_count += 1;
            return Ok((key, Arc::clone(&entry.grid)));
        }

        log::debug!("initializing new grid for font config");

        // A new font config, initialize the cache.
        // Build our collection. This is the expensive operation that
        // involves finding fonts, loading them (maybe, some are deferred),
        // etc.
        let c = self.collection(&mut inner, &key, font_size, config)?;

        // Setup our enabled/disabled styles
        let mut styles = StyleStatus::init_fill(true);
        styles.set(Style::Bold, config.font_style_bold != FontStyle::False);
        styles.set(Style::Italic, config.font_style_italic != FontStyle::False);
        styles.set(
            Style::BoldItalic,
            config.font_style_bold_italic != FontStyle::False,
        );

        // Init our resolver which just requires setting fields.
        let mut resolver = CodepointResolver::new(c);
        resolver.styles = styles;
        resolver.discover = Some(Self::discover(&mut inner).clone());
        resolver.codepoint_map = Some(key.codepoint_map.clone());

        let grid = Arc::new(SharedGrid::new(resolver)?);
        inner.map.insert(
            key.clone(),
            ReffedGrid {
                grid: Arc::clone(&grid),
                ref_count: 1,
            },
        );
        Ok((key, grid))
    }

    /// Builds the Collection for the given configuration key and
    /// initial font size.
    fn collection(
        &self,
        inner: &mut Inner,
        key: &Key,
        size: DesiredSize,
        config: &DerivedConfig,
    ) -> Result<Collection, RefError> {
        let load_options = LoadOptions {
            library: self.font_lib.clone(),
            size,
            freetype_load_flags: key.freetype_load_flags,
        };

        let mut c = Collection::new();
        c.load_options = Some(load_options.clone());
        c.metric_modifiers.clone_from(&key.metric_modifiers);

        // Search for fonts
        {
            let disco = Self::discover(inner);
            for style in Style::ALL {
                for desc in key.descriptors_for_style(style) {
                    {
                        let mut disco_it = disco.discover(desc)?;
                        if let Some(face) = disco_it.next()? {
                            log::info!(
                                "font {style:?}: {}",
                                face.name().unwrap_or_else(|_| "<error>".into())
                            );

                            c.add_deferred(
                                face,
                                AddOptions {
                                    style,
                                    fallback: false,
                                    // No size adjustment for primary fonts.
                                    size_adjustment: SizeAdjustment::None,
                                },
                            )?;

                            continue;
                        }
                    }

                    // If there are variation configurations and we didn't find
                    // the font, then we retry the discovery with all stylistic
                    // bits set to false. This is because some fonts may not
                    // set the stylistic bit in their table but still support
                    // axes to mimic the style. At the time of writing, Berkeley
                    // Mono Variable is like this. See #2140.
                    if style != Style::Regular && !desc.variations.is_empty() {
                        let mut copy = desc.clone();
                        copy.bold = false;
                        copy.italic = false;
                        let mut disco_it = disco.discover(&copy)?;
                        if let Some(face) = disco_it.next()? {
                            log::info!(
                                "font {style:?}: {}",
                                face.name().unwrap_or_else(|_| "<error>".into())
                            );

                            c.add_deferred(
                                face,
                                AddOptions {
                                    style,
                                    fallback: false,
                                    // No size adjustment for primary fonts.
                                    size_adjustment: SizeAdjustment::None,
                                },
                            )?;

                            continue;
                        }
                    }

                    log::warn!(
                        "font-family {style:?} not found: {}",
                        desc.family.as_deref().unwrap_or("")
                    );
                }
            }
        }

        // Complete our styles to ensure we have something to satisfy every
        // possible style request. We do this before adding our built-in font
        // because we want to ensure our built-in styles are fallbacks to
        // the configured styles.
        c.complete_styles(config.font_synthetic_style)?;

        let fallback = |style: Style, size_adjustment: SizeAdjustment| AddOptions {
            style,
            fallback: true,
            size_adjustment,
        };
        let face_opts = load_options.face_options();
        let wght_700 = [Variation {
            id: VariationId::init(b"wght"),
            value: 700.0,
        }];

        // Our built-in font will be used as a backup
        c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::VARIABLE),
                face_opts,
            )?,
            fallback(Style::Regular, crate::DEFAULT_FALLBACK_ADJUSTMENT),
        )?;
        let idx = c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::VARIABLE),
                face_opts,
            )?,
            fallback(Style::Bold, crate::DEFAULT_FALLBACK_ADJUSTMENT),
        )?;
        c.get_face(idx)?.set_variations(&wght_700, face_opts)?;
        c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::VARIABLE_ITALIC),
                face_opts,
            )?,
            fallback(Style::Italic, crate::DEFAULT_FALLBACK_ADJUSTMENT),
        )?;
        let idx = c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::VARIABLE_ITALIC),
                face_opts,
            )?,
            fallback(Style::BoldItalic, crate::DEFAULT_FALLBACK_ADJUSTMENT),
        )?;
        c.get_face(idx)?.set_variations(&wght_700, face_opts)?;

        // Nerd-font symbols fallback.
        c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::SYMBOLS_NERD_FONT),
                face_opts,
            )?,
            // No size adjustment for the symbols font.
            fallback(Style::Regular, SizeAdjustment::None),
        )?;

        // Emoji fallback. (Zig skips this on macOS, which is out of scope.)
        c.add(
            Face::init(&self.font_lib, FontData::Static(embedded::EMOJI), face_opts)?,
            // No size adjustment for emojis.
            fallback(Style::Regular, SizeAdjustment::None),
        )?;
        c.add(
            Face::init(
                &self.font_lib,
                FontData::Static(embedded::EMOJI_TEXT),
                face_opts,
            )?,
            fallback(Style::Regular, SizeAdjustment::None),
        )?;

        Ok(c)
    }

    /// Decrement the ref count for the given key. If the ref count is zero,
    /// the grid will be deinitialized and removed from the map.
    pub fn deref(&self, key: &Key) {
        let mut inner = self.lock();
        let Some(entry) = inner.map.get_mut(key) else {
            return;
        };
        assert!(entry.ref_count >= 1);

        // If we have more than one reference, decrement and return.
        if entry.ref_count > 1 {
            entry.ref_count -= 1;
            return;
        }

        // We are at a zero ref count so deinit the group and remove.
        inner.map.remove(key);
    }

    /// Initialize once and return the font discovery mechanism. This remains
    /// initialized throughout the lifetime of the application because some
    /// font discovery mechanisms (i.e. fontconfig) are unsafe to reinit.
    fn discover(inner: &mut Inner) -> &Fontconfig {
        inner.font_discover.get_or_insert_with(Fontconfig::init)
    }
}

/// This is the configuration required to create a key without having
/// to keep the full Ghostty configuration around.
#[derive(Debug, Clone)]
pub struct DerivedConfig {
    pub font_family: RepeatableString,
    pub font_family_bold: RepeatableString,
    pub font_family_italic: RepeatableString,
    pub font_family_bold_italic: RepeatableString,
    pub font_style: FontStyle,
    pub font_style_bold: FontStyle,
    pub font_style_italic: FontStyle,
    pub font_style_bold_italic: FontStyle,
    pub font_variation: RepeatableFontVariation,
    pub font_variation_bold: RepeatableFontVariation,
    pub font_variation_italic: RepeatableFontVariation,
    pub font_variation_bold_italic: RepeatableFontVariation,
    pub font_codepoint_map: RepeatableCodepointMap,
    pub font_synthetic_style: FontSyntheticStyle,
    pub adjust_cell_width: Option<Modifier>,
    pub adjust_cell_height: Option<Modifier>,
    pub adjust_font_baseline: Option<Modifier>,
    pub adjust_underline_position: Option<Modifier>,
    pub adjust_underline_thickness: Option<Modifier>,
    pub adjust_strikethrough_position: Option<Modifier>,
    pub adjust_strikethrough_thickness: Option<Modifier>,
    pub adjust_overline_position: Option<Modifier>,
    pub adjust_overline_thickness: Option<Modifier>,
    pub adjust_cursor_thickness: Option<Modifier>,
    pub adjust_cursor_height: Option<Modifier>,
    pub adjust_box_thickness: Option<Modifier>,
    pub adjust_icon_height: Option<Modifier>,
    pub freetype_load_flags: FreetypeLoadFlags,
}

impl DerivedConfig {
    /// Initialize a DerivedConfig from a config. (Cloning a DerivedConfig
    /// is `Clone::clone`.)
    pub fn new(config: &Config) -> DerivedConfig {
        DerivedConfig {
            font_family: config.font_family.clone(),
            font_family_bold: config.font_family_bold.clone(),
            font_family_italic: config.font_family_italic.clone(),
            font_family_bold_italic: config.font_family_bold_italic.clone(),
            font_style: config.font_style.clone(),
            font_style_bold: config.font_style_bold.clone(),
            font_style_italic: config.font_style_italic.clone(),
            font_style_bold_italic: config.font_style_bold_italic.clone(),
            font_variation: config.font_variation.clone(),
            font_variation_bold: config.font_variation_bold.clone(),
            font_variation_italic: config.font_variation_italic.clone(),
            font_variation_bold_italic: config.font_variation_bold_italic.clone(),
            font_codepoint_map: config.font_codepoint_map.clone(),
            font_synthetic_style: config.font_synthetic_style,
            adjust_cell_width: config.adjust_cell_width,
            adjust_cell_height: config.adjust_cell_height,
            adjust_font_baseline: config.adjust_font_baseline,
            adjust_underline_position: config.adjust_underline_position,
            adjust_underline_thickness: config.adjust_underline_thickness,
            adjust_strikethrough_position: config.adjust_strikethrough_position,
            adjust_strikethrough_thickness: config.adjust_strikethrough_thickness,
            adjust_overline_position: config.adjust_overline_position,
            adjust_overline_thickness: config.adjust_overline_thickness,
            adjust_cursor_thickness: config.adjust_cursor_thickness,
            adjust_cursor_height: config.adjust_cursor_height,
            adjust_box_thickness: config.adjust_box_thickness,
            adjust_icon_height: config.adjust_icon_height,
            freetype_load_flags: config.freetype_load_flags,
        }
    }
}

/// The key used to uniquely identify a font configuration.
#[derive(Debug, Clone)]
pub struct Key {
    /// The descriptors used for all the fonts added to the
    /// initial group, including all styles. This is hashed
    /// in order so the order matters. All users of the struct
    /// should ensure that the order is consistent.
    pub descriptors: Vec<Descriptor>,

    /// These are the offsets into the descriptors array for
    /// each style. For example, bold is from
    /// offsets[Style::Bold as usize - 1] to
    /// offsets[Style::Bold as usize].
    pub style_offsets: [usize; 4],

    /// The codepoint map configuration.
    pub codepoint_map: CodepointMap,

    /// The metric modifier set configuration.
    pub metric_modifiers: ModifierSet,

    /// The configured font size for this key. We don't use this
    /// directly but it is used as part of the hash for the
    /// font grid.
    pub font_size: DesiredSize,

    /// The freetype load flags configuration.
    pub freetype_load_flags: FreetypeLoadFlags,
}

impl Key {
    pub fn new(config: &DerivedConfig, font_size: DesiredSize) -> Key {
        let mut descriptors: Vec<Descriptor> = Vec::new();
        for family in &config.font_family.list {
            descriptors.push(Descriptor {
                family: Some(family.clone()),
                style: config.font_style.name_value().map(str::to_string),
                size: font_size.points,
                variations: config.font_variation.list.clone(),
                ..Descriptor::default()
            });
        }

        // In all the styled cases below, we prefer to specify an exact
        // style via the `font-style` configuration. If a style is not
        // specified, we use the discovery mechanism to search for a
        // style category such as bold, italic, etc. We can't specify both
        // because the latter will restrict the search to only that. If
        // a user says `font-style = italic` for the bold face for example,
        // no results would be found if we restrict to ALSO searching for
        // italic.
        for family in &config.font_family_bold.list {
            let style = config.font_style_bold.name_value().map(str::to_string);
            descriptors.push(Descriptor {
                family: Some(family.clone()),
                bold: style.is_none(),
                style,
                size: font_size.points,
                variations: config.font_variation_bold.list.clone(),
                ..Descriptor::default()
            });
        }
        for family in &config.font_family_italic.list {
            let style = config.font_style_italic.name_value().map(str::to_string);
            descriptors.push(Descriptor {
                family: Some(family.clone()),
                italic: style.is_none(),
                style,
                size: font_size.points,
                variations: config.font_variation_italic.list.clone(),
                ..Descriptor::default()
            });
        }
        for family in &config.font_family_bold_italic.list {
            let style = config
                .font_style_bold_italic
                .name_value()
                .map(str::to_string);
            descriptors.push(Descriptor {
                family: Some(family.clone()),
                bold: style.is_none(),
                italic: style.is_none(),
                style,
                size: font_size.points,
                variations: config.font_variation_bold_italic.list.clone(),
                ..Descriptor::default()
            });
        }

        // Setup the codepoint map
        let codepoint_map = config.font_codepoint_map.map.clone();

        // Metric modifiers
        let mut metric_modifiers = ModifierSet::new();
        for (key, m) in [
            (MetricsKey::CellWidth, config.adjust_cell_width),
            (MetricsKey::CellHeight, config.adjust_cell_height),
            (MetricsKey::CellBaseline, config.adjust_font_baseline),
            (
                MetricsKey::UnderlinePosition,
                config.adjust_underline_position,
            ),
            (
                MetricsKey::UnderlineThickness,
                config.adjust_underline_thickness,
            ),
            (
                MetricsKey::StrikethroughPosition,
                config.adjust_strikethrough_position,
            ),
            (
                MetricsKey::StrikethroughThickness,
                config.adjust_strikethrough_thickness,
            ),
            (
                MetricsKey::OverlinePosition,
                config.adjust_overline_position,
            ),
            (
                MetricsKey::OverlineThickness,
                config.adjust_overline_thickness,
            ),
            (MetricsKey::CursorThickness, config.adjust_cursor_thickness),
            (MetricsKey::CursorHeight, config.adjust_cursor_height),
            (MetricsKey::BoxThickness, config.adjust_box_thickness),
            (MetricsKey::IconHeight, config.adjust_icon_height),
        ] {
            if let Some(m) = m {
                metric_modifiers.insert(key, m);
            }
        }

        let regular_offset = config.font_family.list.len();
        let bold_offset = regular_offset + config.font_family_bold.list.len();
        let italic_offset = bold_offset + config.font_family_italic.list.len();
        let bold_italic_offset = italic_offset + config.font_family_bold_italic.list.len();

        Key {
            descriptors,
            style_offsets: [
                regular_offset,
                bold_offset,
                italic_offset,
                bold_italic_offset,
            ],
            codepoint_map,
            metric_modifiers,
            font_size,
            freetype_load_flags: config.freetype_load_flags,
        }
    }

    /// Get the descriptors for the given font style that can be
    /// used with discovery.
    pub fn descriptors_for_style(&self, style: Style) -> &[Descriptor] {
        let idx = style as usize;
        let start = if idx == 0 {
            0
        } else {
            self.style_offsets[idx - 1]
        };
        let end = self.style_offsets[idx];
        &self.descriptors[start..end]
    }

    /// Hash the key with the given hasher.
    pub fn hash_into<H: Hasher>(&self, hasher: &mut H) {
        self.font_size.points.to_bits().hash(hasher);
        self.font_size.xdpi.hash(hasher);
        self.font_size.ydpi.hash(hasher);
        self.descriptors.len().hash(hasher);
        for d in &self.descriptors {
            d.hash(hasher);
        }
        self.codepoint_map.hashcode().hash(hasher);
        self.metric_modifiers.len().hash(hasher);
        self.freetype_load_flags.hash(hasher);
        if !self.metric_modifiers.is_empty() {
            for key in MetricsKey::ALL {
                if let Some(value) = self.metric_modifiers.get(&key) {
                    key.hash(hasher);
                    // Modifier::hash: tag, then the value (floats are
                    // bit-cast, as in Zig).
                    match *value {
                        Modifier::Percent(v) => {
                            0u8.hash(hasher);
                            v.to_bits().hash(hasher);
                        }
                        Modifier::Absolute(v) => {
                            1u8.hash(hasher);
                            v.hash(hasher);
                        }
                    }
                }
            }
        }
    }

    /// Returns a hash code that can be used to uniquely identify this
    /// key.
    pub fn hashcode(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash_into(&mut hasher);
        hasher.finish()
    }
}

// As in Zig's `Map` context, key equality is hashcode equality.
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.hashcode() == other.hashcode()
    }
}
impl Eq for Key {}
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hashcode().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keycfg() -> DerivedConfig {
        DerivedConfig::new(&Config::default())
    }

    #[test]
    fn key() {
        let keycfg = keycfg();
        let k = Key::new(&keycfg, DesiredSize::new(12.0));
        let k2 = Key::new(&keycfg, DesiredSize::new(12.0));
        assert!(k.hashcode() > 0);
        assert_eq!(k.hashcode(), k2.hashcode());
    }

    #[test]
    fn key_different_font_points() {
        let keycfg = keycfg();
        let k = Key::new(&keycfg, DesiredSize::new(12.0));
        let k2 = Key::new(&keycfg, DesiredSize::new(16.0));
        assert_ne!(k.hashcode(), k2.hashcode());
    }

    #[test]
    fn key_different_font_dpi() {
        let keycfg = keycfg();
        let mut a = DesiredSize::new(12.0);
        a.xdpi = 1;
        let mut b = DesiredSize::new(12.0);
        b.xdpi = 2;
        let k = Key::new(&keycfg, a);
        let k2 = Key::new(&keycfg, b);
        assert_ne!(k.hashcode(), k2.hashcode());
    }

    #[test]
    fn key_descriptors_and_modifiers() {
        let mut cfg = Config::default();
        cfg.font_family.list = vec!["A".into(), "B".into()];
        cfg.font_family_bold.list = vec!["C".into()];
        cfg.adjust_cell_width = Some(Modifier::Percent(1.1));
        let d = DerivedConfig::new(&cfg);
        let k = Key::new(&d, DesiredSize::new(12.0));
        assert_eq!(k.style_offsets, [2, 3, 3, 3]);
        assert_eq!(k.descriptors_for_style(Style::Regular).len(), 2);
        let bold = k.descriptors_for_style(Style::Bold);
        assert_eq!(bold.len(), 1);
        assert!(bold[0].bold);
        assert_eq!(k.metric_modifiers.len(), 1);
        let plain = Key::new(&keycfg(), DesiredSize::new(12.0));
        assert_ne!(k.hashcode(), plain.hashcode());
    }

    #[test]
    fn shared_grid_set() {
        let set = SharedGridSet::new().unwrap();
        let keycfg = keycfg();

        // Get a grid for the given config
        let (key1, grid1) = set.ref_(&keycfg, DesiredSize::new(12.0)).unwrap();
        assert_eq!(1, set.count());

        // Get another
        let (key2, grid2) = set.ref_(&keycfg, DesiredSize::new(12.0)).unwrap();
        assert_eq!(1, set.count());

        // They should be pointer equivalent
        assert!(Arc::ptr_eq(&grid1, &grid2));

        // If I deref grid2 then we should still have a count of 1
        set.deref(&key2);
        assert_eq!(1, set.count());

        // If I deref grid1 then we should have a count of 0
        set.deref(&key1);
        assert_eq!(0, set.count());
    }
}
