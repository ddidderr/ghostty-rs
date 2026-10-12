//! Face represents a single font face. A single font face has a single set
//! of properties associated with it such as style, weight, etc.
//!
//! A Face isn't typically meant to be used directly. It is usually used
//! via a Family in order to store it in an Atlas.
//!
//! Port of `font/face/freetype.zig`.

use std::{
    ffi::CString,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use super::{DesiredSize, FontData, FreetypeLoadFlags, Options, RenderOptions, Variation};
use crate::{
    atlas::{self, Atlas},
    glyph::Glyph,
    library::Library,
    metrics::FaceMetrics,
};

pub(crate) mod ft;
mod resize;

pub use ft::HbFont;

#[derive(Debug, thiserror::Error)]
pub enum FaceError {
    #[error("freetype error {0}")]
    Freetype(i32),
    #[error("harfbuzz font creation failed")]
    HarfbuzzFailed,
    #[error("invalid font path")]
    InvalidPath,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Atlas(#[from] atlas::Error),
    #[error("wrong atlas")]
    WrongAtlas,
    #[error("freetype error {0}")]
    Freetype(i32),
    #[error("bitmap handling error")]
    BitmapHandlingError,
    #[error("glyph resize failed")]
    GlyphResizeFailed,
    #[error("unsupported glyph format")]
    UnsupportedGlyphFormat,
    /// Zig panics ("unsupported pixel mode") in this situation.
    #[error("unsupported pixel mode {0}")]
    UnsupportedPixelMode(u32),
}

/// Set to true to apply a synthetic italic to the face.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synthetic {
    pub italic: bool,
    pub bold: bool,
}

/// Convert 26.6 fixed point to f64 (`F26Dot6.to(f64)`).
fn f26dot6_to_f64(v: i64) -> f64 {
    // The Zig code bit-casts the (truncated) i32 value.
    f64::from(v as i32) / 64.0
}

/// `F26Dot6.from(f64)` as the raw 26.6 integer.
fn f64_to_f26dot6(v: f64) -> i32 {
    (v * 64.0).round() as i32
}

fn lock(m: &Mutex<()>) -> MutexGuard<'_, ()> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Face {
    /// Freetype load flags for this font face.
    pub load_flags: FreetypeLoadFlags,

    /// Set quirks.disableDefaultFontFeatures
    pub quirks_disable_default_font_features: bool,

    /// Set to true to apply a synthetic italic to the face.
    pub synthetic: Synthetic,

    /// The current size this font is set to.
    pub size: DesiredSize,

    // NOTE: field order matters for drop order. The harfbuzz font holds its
    // own FreeType reference and the font data must outlive every face
    // reference.
    /// Our font face.
    #[allow(clippy::struct_field_names)]
    face: ft::FtFace,

    /// Harfbuzz font corresponding to this face.
    hb_font: HbFont,

    /// This mutex MUST be held while doing anything with the
    /// glyph slot on the freetype face, because this struct
    /// may be shared across multiple surfaces.
    ///
    /// This means that anywhere where `load_glyph` is called, this
    /// mutex must be held. (Deviation: faces that share an `FT_Face`
    /// through `synthetic_bold`/`synthetic_italic` also share this mutex.)
    ft_mutex: Arc<Mutex<()>>,

    /// Our Library
    lib: Library,

    /// Keeps in-memory font data alive (FreeType does not copy it).
    _data: Option<FontData>,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("size", &self.size)
            .field("synthetic", &self.synthetic)
            .finish_non_exhaustive()
    }
}

// NOTE: `Face` is `Send + Sync` automatically: the unsafe impls live on the
// FFI handle wrappers in `ft.rs`. Mutable FreeType state (size, glyph slot)
// is only touched through `&mut self` or while holding `ft_mutex`.

impl Face {
    /// Initialize a new font face from a file.
    pub fn init_file(
        lib: &Library,
        path: &str,
        index: i32,
        opts: Options,
    ) -> Result<Face, FaceError> {
        let cpath = CString::new(path).map_err(|_| FaceError::InvalidPath)?;
        let face = {
            let _g = lib.lock();
            ft::FtFace::new_file(&lib.inner.lib, &cpath, index).map_err(FaceError::Freetype)?
        };
        Face::init_face(lib, face, None, opts)
    }

    /// Initialize a new font face with the given source in-memory.
    pub fn init(lib: &Library, source: FontData, opts: Options) -> Result<Face, FaceError> {
        let face = {
            let _g = lib.lock();
            ft::FtFace::new_memory(&lib.inner.lib, source.as_slice(), 0)
                .map_err(FaceError::Freetype)?
        };
        Face::init_face(lib, face, Some(source), opts)
    }

    fn init_face(
        lib: &Library,
        face: ft::FtFace,
        data: Option<FontData>,
        opts: Options,
    ) -> Result<Face, FaceError> {
        Face::init_face_with_mutex(lib, face, data, opts, Arc::new(Mutex::new(())))
    }

    fn init_face_with_mutex(
        lib: &Library,
        face: ft::FtFace,
        data: Option<FontData>,
        opts: Options,
        ft_mutex: Arc<Mutex<()>>,
    ) -> Result<Face, FaceError> {
        face.select_unicode_charmap().map_err(FaceError::Freetype)?;
        set_size_(&face, opts.size)?;

        let hb_font = face.create_hb_font().ok_or(FaceError::HarfbuzzFailed)?;

        let mut result = Face {
            load_flags: opts.freetype_load_flags,
            quirks_disable_default_font_features: false,
            synthetic: Synthetic::default(),
            size: opts.size,
            face,
            hb_font,
            ft_mutex,
            lib: lib.clone(),
            _data: data,
        };
        result.quirks_disable_default_font_features = quirks_disable_default_font_features(&result);

        // We output information about available variation axes, if they exist.
        if log::log_enabled!(log::Level::Debug) && result.face.has_multiple_masters() {
            log::debug!("variation axes font={}", result.name());
            if let Ok(axes) = result.face.mm_axes(&result.lib.inner.lib) {
                for axis in axes {
                    let id = super::VariationId(axis.tag);
                    log::debug!(
                        "variation axis: id={} min={} max={} def={}",
                        String::from_utf8_lossy(&id.str()),
                        axis.minimum >> 16,
                        axis.maximum >> 16,
                        axis.def >> 16,
                    );
                }
            }
        }

        Ok(result)
    }

    fn lock_ft(&self) -> MutexGuard<'_, ()> {
        lock(&self.ft_mutex)
    }

    /// Returns the font name (family entry of the SFNT name table), or an
    /// empty string if there is none.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    pub fn name(&self) -> String {
        let count = self.face.sfnt_name_count();

        // We look for the font family entry.
        for i in 0..count {
            let Some(entry) = self.face.sfnt_name(i) else {
                continue;
            };
            if entry.name_id == ft::TT_NAME_ID_FONT_FAMILY {
                let string = &entry.string;
                // There are other encodings that are something other than UTF-8
                // but this is one we've seen "in the wild" so far.
                if entry.platform_id == ft::TT_PLATFORM_MICROSOFT
                    && entry.encoding_id == ft::TT_MS_ID_UNICODE_CS
                    && string.len() % 2 == 0
                    && string.len() <= 1024
                {
                    // UTF-16 big endian.
                    let units: Vec<u16> = string
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    if let Ok(s) = String::from_utf16(&units) {
                        return s;
                    }
                }
                return String::from_utf8_lossy(string).into_owned();
            }
        }

        String::new()
    }

    /// Return a new face that is the same as this but also has synthetic
    /// bold applied.
    pub fn synthetic_bold(&self, opts: Options) -> Result<Face, FaceError> {
        // Increase face ref count
        let face = self.face.reference().map_err(FaceError::Freetype)?;

        let mut f = Face::init_face_with_mutex(
            &self.lib,
            face,
            self._data.clone(),
            opts,
            Arc::clone(&self.ft_mutex),
        )?;
        f.synthetic = self.synthetic;
        f.synthetic.bold = true;

        Ok(f)
    }

    /// Return a new face that is the same as this but has a transformation
    /// matrix applied to italicize it.
    pub fn synthetic_italic(&self, opts: Options) -> Result<Face, FaceError> {
        // Increase face ref count
        let face = self.face.reference().map_err(FaceError::Freetype)?;

        let mut f = Face::init_face_with_mutex(
            &self.lib,
            face,
            self._data.clone(),
            opts,
            Arc::clone(&self.ft_mutex),
        )?;
        f.synthetic = self.synthetic;
        f.synthetic.italic = true;

        Ok(f)
    }

    /// Resize the font in-place. If this succeeds, the caller is responsible
    /// for clearing any glyph caches, font atlas data, etc.
    pub fn set_size(&mut self, opts: Options) -> Result<(), FaceError> {
        let _g = lock(&self.ft_mutex);
        set_size_(&self.face, opts.size)?;
        self.size = opts.size;
        Ok(())
    }

    /// Set the variation axes for this font. This will modify this font
    /// in-place.
    pub fn set_variations(&mut self, vs: &[Variation], _opts: Options) -> Result<(), FaceError> {
        // If this font doesn't support variations, we can't do anything.
        if !self.face.has_multiple_masters() || vs.is_empty() {
            return Ok(());
        }

        // Freetype requires that we send ALL coordinates in at once so the
        // first thing we have to do is get all the vars and put them into
        // an array.
        let axes = self
            .face
            .mm_axes(&self.lib.inner.lib)
            .map_err(FaceError::Freetype)?;

        // To avoid allocations, we cap the number of variation axes we can
        // support. This is arbitrary but Firefox caps this at 16 so I
        // feel like that's probably safe... and we do double cause its
        // cheap.
        let n = axes.len().min(32);
        let mut coords = self
            .face
            .var_design_coordinates(n)
            .map_err(FaceError::Freetype)?;

        // Now we go through each axis and see if its set. This is slow
        // but there usually aren't many axes and usually not many set
        // variations, either.
        for (i, axis) in axes.iter().take(n).enumerate() {
            for v in vs {
                if axis.tag == v.id.0 {
                    coords[i] = (v.value * 65536.0) as i64;
                    break;
                }
            }
        }

        // Set them!
        let _g = lock(&self.ft_mutex);
        self.face
            .set_var_design_coordinates(&coords)
            .map_err(FaceError::Freetype)
    }

    /// Returns the glyph index for the given Unicode code point. If this
    /// face doesn't support this glyph, null is returned.
    pub fn glyph_index(&self, cp: u32) -> Option<u32> {
        self.face.char_index(cp)
    }

    /// Returns true if this font is colored. This can be used by callers to
    /// determine what kind of atlas to pass in.
    pub fn has_color(&self) -> bool {
        self.face.has_color()
    }

    /// Returns true if the given glyph ID is colorized.
    pub fn is_color_glyph(&self, glyph_id: u32) -> bool {
        let _g = self.lock_ft();

        // Load the glyph and see what pixel mode it renders with.
        // All modes other than BGRA are non-color.
        // If the glyph fails to load, just return false.
        //
        // NO_SVG set to true because we don't currently support rendering
        // SVG glyphs under FreeType, since that requires bundling another
        // dependency to handle rendering the SVG.
        let mut flags = ft::LOAD_RENDER | ft::FT_LOAD_NO_SVG;
        if self.face.has_color() {
            flags |= ft::LOAD_COLOR;
        }
        if self.face.load_glyph(glyph_id, flags).is_err() {
            return false;
        }

        self.face.slot_info().pixel_mode == ft::PIXEL_MODE_BGRA
    }

    /// Set the load flags to use when loading a glyph for measurement or
    /// rendering.
    fn glyph_load_flags(&self, constrained: bool) -> i32 {
        // Hinting should only be enabled if the configured load flags specify
        // it and the provided constraint doesn't actually do anything, since
        // if it does, then it'll mess up the hinting anyway when it moves or
        // resizes the glyph.
        let do_hinting = self.load_flags.hinting && !constrained;

        // If our glyph has color, we want to render the color
        let mut flags = 0;
        if self.face.has_color() {
            flags |= ft::LOAD_COLOR;
        }
        // We don't render, because we'll invoke the render
        // manually after applying constraints further down.

        // use options from config
        if !do_hinting {
            flags |= ft::LOAD_NO_HINTING;
        }
        if self.load_flags.force_autohint {
            flags |= ft::LOAD_FORCE_AUTOHINT;
        }
        if !self.load_flags.autohint {
            flags |= ft::LOAD_NO_AUTOHINT;
        }

        // If we're gonna be rendering this glyph in monochrome,
        // then we should use the monochrome hinter as well, or
        // else it won't look very good at all.
        //
        // Otherwise if the user asked for light hinting we
        // use that, otherwise we just use the normal target.
        flags |= if self.load_flags.monochrome {
            ft::TARGET_MONO
        } else if self.load_flags.light {
            ft::TARGET_LIGHT
        } else {
            ft::TARGET_NORMAL
        };

        // NO_SVG set to true because we don't currently support rendering
        // SVG glyphs under FreeType, since that requires bundling another
        // dependency to handle rendering the SVG.
        flags | ft::FT_LOAD_NO_SVG
    }

    /// Get a rect that represents the position and size of the loaded glyph.
    fn get_glyph_size(&self) -> super::GlyphSize {
        let slot = self.face.slot_info();

        // If we're dealing with an outline glyph then we get the
        // outline's bounding box instead of using the built-in
        // metrics, since that's more precise and allows better
        // cell-fitting.
        if slot.format == ft::GLYPH_FORMAT_OUTLINE {
            // Get the glyph's bounding box before we transform it at all.
            // We use this rather than the metrics, since it's more precise.
            let (x_min, y_min, x_max, y_max) = self.face.slot_outline_bbox();

            return super::GlyphSize {
                x: f26dot6_to_f64(x_min),
                y: f26dot6_to_f64(y_min),
                width: f26dot6_to_f64(x_max - x_min),
                height: f26dot6_to_f64(y_max - y_min),
            };
        }

        super::GlyphSize {
            x: f26dot6_to_f64(slot.hori_bearing_x),
            y: f26dot6_to_f64(slot.hori_bearing_y - slot.metrics_height),
            width: f26dot6_to_f64(slot.metrics_width),
            height: f26dot6_to_f64(slot.metrics_height),
        }
    }

    /// Render a glyph using the glyph index. The rendered glyph is stored in the
    /// given texture atlas.
    pub fn render_glyph(
        &self,
        atlas: &mut Atlas,
        glyph_index: u32,
        opts: RenderOptions,
    ) -> Result<Glyph, RenderError> {
        let _g = self.lock_ft();

        // Load the glyph.
        self.face
            .load_glyph(
                glyph_index,
                self.glyph_load_flags(opts.constraint.does_anything()),
            )
            .map_err(RenderError::Freetype)?;
        let slot = self.face.slot_info();

        // For synthetic bold, we embolden the glyph.
        if self.synthetic.bold {
            // We need to scale the embolden amount based on the font size.
            // This is a heuristic I found worked well across a variety of
            // founts: 1 pixel per 64 units of height.
            let font_height = self.face.size_metrics().height as f64;
            let ratio: f64 = 64.0 / 2048.0;
            let amount = (font_height * ratio).ceil();
            self.face.slot_embolden(amount as i64);
        }

        // We get a rect that represents the position
        // and size of the glyph before constraints.
        let rect = self.get_glyph_size();

        // If our glyph is smaller than a quarter pixel in either axis
        // then it has no outlines or they're too small to render.
        //
        // In this case we just return 0-sized glyph struct.
        if rect.width < 0.25 || rect.height < 0.25 {
            return Ok(Glyph::default());
        }

        let metrics = opts.grid_metrics;
        let cell_width = f64::from(metrics.cell_width);
        let cell_height = f64::from(metrics.cell_height);

        // Next we apply any constraints to get the final size of the glyph.
        let constraint = opts.constraint;

        // We need to add the baseline position before passing to the constrain
        // function since it operates on cell-relative positions, not baseline.
        let cell_baseline = f64::from(metrics.cell_baseline);

        let glyph_size = constraint.constrain(
            super::GlyphSize {
                width: rect.width,
                height: rect.height,
                x: rect.x,
                y: rect.y + cell_baseline,
            },
            &metrics,
            opts.constraint_width,
        );

        let mut width = glyph_size.width;
        let mut height = glyph_size.height;
        let mut x = glyph_size.x;
        let mut y = glyph_size.y;

        // We center all glyphs within the pixel-rounded and adjusted
        // cell width if it's larger than the face width, so that they
        // aren't weirdly off to the left.
        //
        // We don't do this if the glyph has a stretch constraint,
        // since in that case the position was already calculated with the
        // new cell width in mind.
        if constraint.size != super::ConstraintSize::Stretch && metrics.face_width < cell_width {
            // We add half the difference to re-center.
            //
            // NOTE: We round this to a whole-pixel amount because under
            //       FreeType, the outlines will be hinted, which isn't
            //       the case under CoreText. If we move the outlines by
            //       a non-whole-pixel amount, it completely ruins the
            //       hinting.
            x += ((cell_width - metrics.face_width) / 2.0).round();
        }

        // If this is a bitmap glyph, it will always render as full pixels,
        // not fractional pixels, so we need to quantize its position and
        // size accordingly to align to full pixels so we get good results.
        if slot.format == ft::GLYPH_FORMAT_BITMAP {
            width = cell_width - (cell_width - width - x).round() - x.round();
            height = cell_height - (cell_height - height - y).round() - y.round();
            x = x.round();
            y = y.round();
        }

        // Now we can render the glyph.
        let mut bitmap: ft::Bitmap;
        let bitmap_left: i32;
        let bitmap_top: i32;
        match slot.format {
            ft::GLYPH_FORMAT_OUTLINE => {
                // Manually adjust the glyph outline with this transform.
                //
                // This offers better precision than using the freetype transform
                // matrix, since that has 16.16 coefficients, and also I was having
                // weird issues that I can only assume where due to freetype doing
                // some bad caching or something when I did this using the matrix.
                let scale_x = width / rect.width;
                let scale_y = height / rect.height;
                let skew: f64 = if self.synthetic.italic {
                    // We skew by 12 degrees to synthesize italics.
                    12.0_f64.to_radians().tan()
                } else {
                    0.0
                };

                self.face.slot_map_outline_points(|ox, oy| {
                    // Convert to f64 for processing
                    let mut px = f26dot6_to_f64(ox);
                    let mut py = f26dot6_to_f64(oy);

                    // Subtract original bearings
                    px -= rect.x;
                    py -= rect.y;

                    // Scale
                    px *= scale_x;
                    py *= scale_y;

                    // Add new bearings
                    px += x;
                    py += y - cell_baseline;

                    // Skew
                    px += py * skew;

                    // Convert back and store
                    (f64_to_f26dot6(px), f64_to_f26dot6(py))
                });

                self.face
                    .slot_render(self.load_flags.monochrome)
                    .map_err(RenderError::Freetype)?;

                // Copy the glyph's bitmap, making sure
                // that it's 8bpp and densely packed.
                bitmap = self
                    .face
                    .slot_bitmap_converted(&self.lib.inner.lib)
                    .map_err(|_| RenderError::BitmapHandlingError)?;
                let after = self.face.slot_info();
                bitmap_left = after.bitmap_left;
                bitmap_top = after.bitmap_top;
            }

            ft::GLYPH_FORMAT_BITMAP => {
                // If our glyph has a non-color bitmap, we need
                // to convert it to dense 8bpp so that the scale
                // operation works correctly.
                let glyph_bitmap = match slot.pixel_mode {
                    ft::PIXEL_MODE_BGRA | ft::PIXEL_MODE_GRAY => self.face.slot_bitmap(),
                    _ => self
                        .face
                        .slot_bitmap_converted(&self.lib.inner.lib)
                        .map_err(|_| RenderError::BitmapHandlingError)?,
                };

                // The resize below interprets pixels using the atlas depth, so
                // a mismatch would read garbage.
                match glyph_bitmap.pixel_mode {
                    ft::PIXEL_MODE_GRAY if atlas.format != atlas::Format::Grayscale => {
                        return Err(RenderError::WrongAtlas);
                    }
                    ft::PIXEL_MODE_BGRA if atlas.format != atlas::Format::Bgra => {
                        return Err(RenderError::WrongAtlas);
                    }
                    _ => {}
                }

                // Round our target width and height
                // as the size for our scaled bitmap.
                let w = width.round().max(0.0) as u32;
                let h = height.round().max(0.0) as u32;
                let depth = u32::from(atlas.format.depth());
                let pitch = w * depth;

                // Resize
                let buf = resize::resize_uint8(
                    &glyph_bitmap.buffer,
                    glyph_bitmap.width as usize,
                    glyph_bitmap.rows as usize,
                    glyph_bitmap.pitch.unsigned_abs() as usize,
                    w as usize,
                    h as usize,
                    depth as usize,
                )
                // This should never fail because this is a
                // fairly straightforward in-memory operation...
                .ok_or(RenderError::GlyphResizeFailed)?;

                bitmap = ft::Bitmap {
                    buffer: buf,
                    width: w,
                    rows: h,
                    pitch: pitch as i32,
                    pixel_mode: glyph_bitmap.pixel_mode,
                    num_grays: glyph_bitmap.num_grays,
                };

                // Update the bearings to account for the new positioning.
                bitmap_top = (y - cell_baseline + height).floor() as i32;
                bitmap_left = x.floor() as i32;
            }

            f => {
                // Glyph formats are tags, so we can
                // output a semi-readable error here.
                log::error!(
                    "Can't render glyph with unsupported glyph format \"{}\"",
                    String::from_utf8_lossy(&f.to_be_bytes())
                );
                return Err(RenderError::UnsupportedGlyphFormat);
            }
        }

        // If this is a color glyph but we're trying to render it to the
        // grayscale atlas, or vice versa, then we throw and error. Maybe
        // in the future we could convert, but for now it should be fine.
        match bitmap.pixel_mode {
            ft::PIXEL_MODE_GRAY => {
                if atlas.format != atlas::Format::Grayscale {
                    return Err(RenderError::WrongAtlas);
                }
            }
            ft::PIXEL_MODE_BGRA => {
                if atlas.format != atlas::Format::Bgra {
                    return Err(RenderError::WrongAtlas);
                }
            }
            mode => {
                log::warn!("glyph={glyph_index} pixel mode={mode}");
                return Err(RenderError::UnsupportedPixelMode(mode));
            }
        }

        // Our whole-pixel bearings for the final glyph.
        // The fractional portion will be included in the rasterized position.
        //
        // For the Y position, FreeType's `bitmap_top` is the distance from the
        // baseline to the top of the glyph, but we need the distance from the
        // bottom of the cell to the bottom of the glyph, so first we add the
        // baseline to get the distance from the bottom of the cell to the top
        // of the glyph, then we subtract the height of the glyph to get the
        // bottom.
        let px_x: i32 = bitmap_left;
        let px_y: i32 = bitmap_top + metrics.cell_baseline as i32 - bitmap.rows as i32;

        let px_width = bitmap.width;
        let px_height = bitmap.rows;
        let len = bitmap.pitch.unsigned_abs() as usize * bitmap.rows as usize;

        // If our bitmap is grayscale, make sure to multiply all pixel
        // values by the right factor to bring `num_grays` up to 256.
        //
        // This is necessary because FT_Bitmap_Convert doesn't do this,
        // it just sets num_grays to the correct number and uses the
        // original smaller pixel values.
        if bitmap.pixel_mode == ft::PIXEL_MODE_GRAY && bitmap.num_grays < 256 {
            let factor = (255 / (i32::from(bitmap.num_grays) - 1)) as u8;
            for p in &mut bitmap.buffer[..len] {
                *p = p.wrapping_mul(factor);
            }
            bitmap.num_grays = 256;
        }

        // Must have non-empty bitmap because we return earlier if zero.
        // We assume the rest of this that it is non-zero so this is important.
        debug_assert!(px_width > 0 && px_height > 0);

        // If this doesn't match then something is wrong.
        debug_assert_eq!(
            px_width * u32::from(atlas.format.depth()),
            bitmap.pitch.unsigned_abs()
        );

        // Allocate our texture atlas region and copy our bitmap in to it.
        let region = atlas.reserve(px_width, px_height)?;
        atlas.set(region, &bitmap.buffer[..len]);

        // This should be the distance from the bottom of
        // the cell to the top of the glyph's bounding box.
        let offset_y: i32 = px_y + px_height as i32;

        // This should be the distance from the left of
        // the cell to the left of the glyph's bounding box.
        let offset_x: i32 = px_x;

        Ok(Glyph {
            width: px_width,
            height: px_height,
            offset_x,
            offset_y,
            atlas_x: region.x,
            atlas_y: region.y,
        })
    }

    /// Convert 16.6 pixel format to pixels based on the scale factor of the
    /// current font size.
    #[allow(dead_code)]
    fn units_to_px_y(&self, units: i32) -> i32 {
        // FT_MulFix(units, y_scale) >> 6
        let scale = self.face.size_metrics().y_scale;
        let prod = i64::from(units) * scale;
        (((prod + 0x8000 + (prod >> 63)) >> 16) >> 6) as i32
    }

    /// Get the `FaceMetrics` for this face.
    pub fn get_metrics(&self) -> FaceMetrics {
        let face = &self.face;

        let size_metrics = face.size_metrics();

        // This code relies on this assumption, and it should always be
        // true since we don't do any non-uniform scaling on the font ever.
        debug_assert_eq!(size_metrics.x_ppem, size_metrics.y_ppem);

        // Read the 'head' table out of the font data.
        let head_units_per_em = face.head_units_per_em();

        // Read the 'post' table out of the font data.
        let post_ = face.post();

        // Read the 'OS/2' table out of the font data.
        let os2_ = face.os2();

        // Read the 'hhea' table out of the font data.
        let hhea_ = face.hhea();

        // Whether the font is in a scalable format. We need to know this
        // because many of the metrics provided by FreeType are invalid for
        // non-scalable fonts.
        let is_scalable = face.is_scalable();

        // We get the UPM from the head table.
        //
        // If we have no head, but it is a scalable face, take the UPM from
        // FreeType's units_per_EM, otherwise we'll assume that UPM == PPEM.
        let units_per_em: u16 = if let Some(u) = head_units_per_em {
            u
        } else if is_scalable {
            face.units_per_em()
        } else {
            size_metrics.y_ppem
        };
        let px_per_em = f64::from(size_metrics.y_ppem);
        let px_per_unit = px_per_em / f64::from(units_per_em);

        let (ascent, descent, line_gap): (f64, f64, f64) = 'vertical_metrics: {
            let Some(hhea) = hhea_ else {
                // If we couldn't get the hhea table, rely on metrics from FreeType.
                let ascender = f26dot6_to_f64(size_metrics.ascender);
                let descender = f26dot6_to_f64(size_metrics.descender);
                let height = f26dot6_to_f64(size_metrics.height);
                break 'vertical_metrics (
                    ascender,
                    descender,
                    // We compute the line gap by adding the (negative) descender
                    // and subtracting the (positive) ascender from the line height
                    // to get the remaining gap size.
                    //
                    // NOTE: This might always be 0... but it doesn't hurt to do.
                    height + descender - ascender,
                );
            };

            let hhea_ascent = f64::from(hhea.ascender);
            let hhea_descent = f64::from(hhea.descender);
            let hhea_line_gap = f64::from(hhea.line_gap);

            // If our font has no OS/2 table, then we just
            // blindly use the metrics from the hhea table.
            let Some(os2) = os2_ else {
                break 'vertical_metrics (
                    hhea_ascent * px_per_unit,
                    hhea_descent * px_per_unit,
                    hhea_line_gap * px_per_unit,
                );
            };

            let os2_ascent = f64::from(os2.s_typo_ascender);
            let os2_descent = f64::from(os2.s_typo_descender);
            let os2_line_gap = f64::from(os2.s_typo_line_gap);

            // If the font says to use typo metrics, trust it.
            // (The USE_TYPO_METRICS bit is bit 7)
            if os2.fs_selection & (1 << 7) != 0 {
                break 'vertical_metrics (
                    os2_ascent * px_per_unit,
                    os2_descent * px_per_unit,
                    os2_line_gap * px_per_unit,
                );
            }

            // Otherwise we prefer the height metrics from 'hhea' if they
            // are available, or else OS/2 sTypo* metrics, and if all else
            // fails then we use OS/2 usWin* metrics.
            //
            // This is not "standard" behavior, but it's our best bet to
            // account for fonts being... just weird. It's pretty much what
            // FreeType does to get its generic ascent and descent metrics.

            if hhea.ascender != 0 || hhea.descender != 0 {
                break 'vertical_metrics (
                    hhea_ascent * px_per_unit,
                    hhea_descent * px_per_unit,
                    hhea_line_gap * px_per_unit,
                );
            }

            if os2_ascent != 0.0 || os2_descent != 0.0 {
                break 'vertical_metrics (
                    os2_ascent * px_per_unit,
                    os2_descent * px_per_unit,
                    os2_line_gap * px_per_unit,
                );
            }

            let win_ascent = f64::from(os2.us_win_ascent);
            let win_descent = f64::from(os2.us_win_descent);
            (
                win_ascent * px_per_unit,
                // usWinDescent is *positive* -> down unlike sTypoDescender
                // and hhea.Descender, so we flip its sign to fix this.
                -win_descent * px_per_unit,
                0.0,
            )
        };

        let (underline_position, underline_thickness): (Option<f64>, Option<f64>) =
            if let Some(post) = post_ {
                // Some fonts have degenerate 'post' tables where the underline
                // thickness (and often position) are 0. We consider them null
                // if this is the case and use our own fallbacks when we calculate.
                let has_broken_underline = post.underline_thickness == 0;

                // If the underline position isn't 0 then we do use it,
                // even if the thickness is't properly specified.
                let pos = if has_broken_underline && post.underline_position == 0 {
                    None
                } else {
                    Some(f64::from(post.underline_position) * px_per_unit)
                };

                let thick = if has_broken_underline {
                    None
                } else {
                    Some(f64::from(post.underline_thickness) * px_per_unit)
                };

                (pos, thick)
            } else {
                (None, None)
            };

        // Similar logic to the underline above.
        let (strikethrough_position, strikethrough_thickness): (Option<f64>, Option<f64>) =
            if let Some(os2) = os2_ {
                let has_broken_strikethrough = os2.y_strikeout_size == 0;

                let pos = if has_broken_strikethrough && os2.y_strikeout_position == 0 {
                    None
                } else {
                    Some(f64::from(os2.y_strikeout_position) * px_per_unit)
                };

                let thick = if has_broken_strikethrough {
                    None
                } else {
                    Some(f64::from(os2.y_strikeout_size) * px_per_unit)
                };

                (pos, thick)
            } else {
                (None, None)
            };

        // Cell width is calculated by calculating the widest width of the
        // visible ASCII characters. Usually 'M' is widest but we just take
        // whatever is widest.
        //
        // ASCII height is calculated as the height of the overall bounding
        // box of the same characters.
        //
        // If we fail to load any visible ASCII we just use max_advance from
        // the metrics provided by FreeType, and set ascii_height to null as
        // it's optional.
        let (cell_width, ascii_height): (f64, Option<f64>) = {
            let _g = self.lock_ft();

            let mut max: f64 = 0.0;
            let mut top: f64 = 0.0;
            let mut bottom: f64 = 0.0;
            for c in b' '..127u8 {
                if let Some(glyph_index) = face.char_index(u32::from(c))
                    && face
                        .load_glyph(glyph_index, self.glyph_load_flags(false))
                        .is_ok()
                {
                    max = f26dot6_to_f64(face.slot_info().advance_x).max(max);
                    let rect = self.get_glyph_size();
                    top = (rect.y + rect.height).max(top);
                    bottom = rect.y.min(bottom);
                }
            }

            // If we couldn't get valid measurements, just use
            // FreeType's max_advance and null, respectively.
            if max == 0.0 {
                max = f26dot6_to_f64(size_metrics.max_advance);
            }
            let estimate = top - bottom;
            let rect_height = if estimate <= 0.0 {
                None
            } else {
                Some(estimate)
            };

            (max, rect_height)
        };

        // We use the cap and ex heights specified by the font if they're
        // available, otherwise we try to measure the `H` and `x` glyphs.
        let (cap_height, ex_height): (Option<f64>, Option<f64>) = 'heights: {
            if let Some(os2) = os2_ {
                // The OS/2 table does not include these metrics in version 1.
                if os2.version >= 2 {
                    break 'heights (
                        Some(f64::from(os2.s_cap_height) * px_per_unit),
                        Some(f64::from(os2.sx_height) * px_per_unit),
                    );
                }
            }

            let measure = |ch: char| -> Option<f64> {
                let _g = self.lock_ft();
                let glyph_index = face.char_index(ch as u32)?;
                face.load_glyph(glyph_index, self.glyph_load_flags(false))
                    .ok()?;
                Some(self.get_glyph_size().height)
            };
            (measure('H'), measure('x'))
        };

        // Measure "水" (CJK water ideograph, U+6C34) for our ic width.
        let ic_width: Option<f64> = 'ic_width: {
            let _g = self.lock_ft();

            let Some(glyph) = face.char_index('水' as u32) else {
                break 'ic_width None;
            };

            if face
                .load_glyph(glyph, self.glyph_load_flags(false))
                .is_err()
            {
                break 'ic_width None;
            }

            // If the advance of the glyph is less than the width of the actual
            // glyph then we just treat it as invalid since it's probably wrong
            // and using it for size normalization will instead make the font
            // way too big.
            //
            // This can sometimes happen if there's a CJK font that has been
            // patched with the nerd fonts patcher and it butchers the advance
            // values so the advance ends up half the width of the actual glyph.
            let ft_glyph_width = self.get_glyph_size().width;
            let advance = f26dot6_to_f64(face.slot_info().advance_x);
            if ft_glyph_width > advance {
                log::warn!(
                    "(getMetrics) Width of glyph '水' for font \"{}\" is greater than its advance ({} > {}), discarding ic_width metric.",
                    self.name(),
                    ft_glyph_width,
                    advance,
                );
                break 'ic_width None;
            }

            Some(advance)
        };

        FaceMetrics {
            px_per_em,

            cell_width,

            ascent,
            descent,
            line_gap,

            underline_position,
            underline_thickness,

            strikethrough_position,
            strikethrough_thickness,

            cap_height,
            ex_height,
            ascii_height,
            ic_width,
        }
    }

    /// Copy the font table data for the given tag.
    pub fn copy_table(&self, tag: &[u8; 4]) -> Result<Option<Vec<u8>>, FaceError> {
        self.face.load_sfnt_table(*tag).map_err(FaceError::Freetype)
    }

    /// The harfbuzz font corresponding to this face (for shapers).
    pub fn hb_font(&self) -> &HbFont {
        &self.hb_font
    }

    /// The library this face was created with.
    pub fn library(&self) -> &Library {
        &self.lib
    }
}

/// If true, the default font features should be disabled for the given face.
///
/// Port of `quirks.disableDefaultFontFeatures`: this used to check specific
/// font names but the logic was integrated directly into the shaping
/// algorithm, so it is always false.
fn quirks_disable_default_font_features(_face: &Face) -> bool {
    false
}

fn set_size_(face: &ft::FtFace, size: DesiredSize) -> Result<(), FaceError> {
    // If we have fixed sizes, we just have to try to pick the one closest
    // to what the user requested. Otherwise, we can choose an arbitrary
    // pixel size.
    if face.is_scalable() {
        let size_26dot6 = (size.points * 64.0).round() as i32;
        face.set_char_size(0, size_26dot6, size.xdpi, size.ydpi)
            .map_err(FaceError::Freetype)
    } else {
        select_size_nearest(face, size.pixels().round() as u32)
    }
}

/// Selects the fixed size in the loaded face that is closest to the
/// requested pixel size.
fn select_size_nearest(face: &ft::FtFace, size: u32) -> Result<(), FaceError> {
    let mut best_i: i32 = 0;
    let mut best_diff: i32 = 0;
    for i in 0..face.num_fixed_sizes() {
        let width = face.fixed_size_width(i);
        let diff = size as i32 - width;
        if i == 0 || diff < best_diff {
            best_diff = diff;
            best_i = i;
        }
    }

    face.select_size(best_i).map_err(FaceError::Freetype)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Metrics, embedded};

    fn opts(points: f32, dpi: u16) -> Options {
        Options::new(DesiredSize {
            points,
            xdpi: dpi,
            ydpi: dpi,
        })
    }

    fn init(lib: &Library, data: &'static [u8], points: f32, dpi: u16) -> Face {
        Face::init(lib, data.into(), opts(points, dpi)).unwrap()
    }

    #[test]
    fn face_name() {
        let lib = Library::init().unwrap();

        {
            let face = init(&lib, embedded::VARIABLE, 14.0, 96);
            assert_eq!("JetBrains Mono", face.name());
        }

        {
            let face = init(&lib, embedded::INCONSOLATA, 14.0, 96);
            assert_eq!("Inconsolata", face.name());
        }
    }

    #[test]
    fn render_ascii_and_resize() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Grayscale);
        let mut ft_font = init(&lib, embedded::INCONSOLATA, 12.0, 96);

        // Generate all visible ASCII
        for i in 32u8..127 {
            ft_font
                .render_glyph(
                    &mut atlas,
                    ft_font.glyph_index(u32::from(i)).unwrap(),
                    RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
                )
                .unwrap();
        }

        // Test resizing
        let g1 = ft_font
            .render_glyph(
                &mut atlas,
                ft_font.glyph_index('A' as u32).unwrap(),
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();
        assert_eq!(11, g1.height);

        ft_font.set_size(opts(24.0, 96)).unwrap();
        let g2 = ft_font
            .render_glyph(
                &mut atlas,
                ft_font.glyph_index('A' as u32).unwrap(),
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();
        assert_eq!(21, g2.height);
    }

    #[test]
    fn color_emoji() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Bgra);
        let ft_font = init(&lib, embedded::EMOJI, 12.0, 96);

        ft_font
            .render_glyph(
                &mut atlas,
                ft_font.glyph_index('🥸' as u32).unwrap(),
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();

        // Make sure this glyph has color
        assert!(ft_font.has_color());
        let glyph_id = ft_font.glyph_index('🥸' as u32).unwrap();
        assert!(ft_font.is_color_glyph(glyph_id));
    }

    #[test]
    fn mono_to_bgra() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Bgra);
        let ft_font = init(&lib, embedded::EMOJI, 12.0, 72);

        // glyph 3 is mono in Noto
        ft_font
            .render_glyph(
                &mut atlas,
                3,
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();
    }

    #[test]
    fn svg_font_table() {
        let lib = Library::init().unwrap();
        let face = init(&lib, embedded::JULIA_MONO, 12.0, 72);

        let table = face.copy_table(b"SVG ").unwrap().unwrap();
        assert_eq!(430, table.len());
    }

    const TERMINUS_I: &str = "\
........\n\
........\n\
...#....\n\
...#....\n\
........\n\
..##....\n\
...#....\n\
...#....\n\
...#....\n\
...#....\n\
...#....\n\
..###...\n\
........\n\
........\n\
........\n\
........";
    // Including the newline
    const TERMINUS_I_PITCH: usize = 9;

    fn check_pattern(atlas: &Atlas, glyph: &Glyph, pattern: &str, pitch: usize) {
        let pat = pattern.as_bytes();
        for y in 0..glyph.height as usize {
            for x in 0..glyph.width as usize {
                let pixel = pat[y * pitch + x];
                assert_eq!(
                    if pixel == b'#' { 255 } else { 0 },
                    atlas.data[(glyph.atlas_y as usize + y) * atlas.size as usize
                        + (glyph.atlas_x as usize + x)],
                    "pixel ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn bitmap_glyph() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Grayscale);

        // Any glyph at 12pt @ 96 DPI is a bitmap
        let ft_font = init(&lib, embedded::TERMINUS_TTF, 12.0, 96);

        // glyph 77 = 'i'
        let glyph = ft_font
            .render_glyph(
                &mut atlas,
                77,
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();

        // should render crisp
        assert_eq!(8, glyph.width);
        assert_eq!(16, glyph.height);
        check_pattern(&atlas, &glyph, TERMINUS_I, TERMINUS_I_PITCH);
    }

    // Expected pixel pattern for Spleen 8x16 'A' (glyph index from char 'A')
    // Derived from BDF BITMAP data: 00,00,7C,C6,C6,C6,FE,C6,C6,C6,C6,C6,00,00,00,00
    const SPLEEN_A: &str = "\
........\n\
........\n\
.#####..\n\
##...##.\n\
##...##.\n\
##...##.\n\
#######.\n\
##...##.\n\
##...##.\n\
##...##.\n\
##...##.\n\
##...##.\n\
........\n\
........\n\
........\n\
........";
    // Including the newline
    const SPLEEN_A_PITCH: usize = 9;
    // Test parameters for bitmap font tests
    const SPLEEN_TEST_POINT_SIZE: f32 = 12.0;
    const SPLEEN_TEST_DPI: u16 = 96;

    fn spleen_test(data: &'static [u8]) {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Grayscale);

        // Spleen 8x16 is a pure bitmap font at 16px height
        let ft_font = init(&lib, data, SPLEEN_TEST_POINT_SIZE, SPLEEN_TEST_DPI);

        // Get glyph index for 'A'
        let glyph_index = ft_font.glyph_index('A' as u32).expect("glyph not found");

        let glyph = ft_font
            .render_glyph(
                &mut atlas,
                glyph_index,
                RenderOptions::new(Metrics::calc(ft_font.get_metrics())),
            )
            .unwrap();

        // Verify dimensions match Spleen 8x16
        assert_eq!(8, glyph.width);
        assert_eq!(16, glyph.height);

        // Verify pixel-perfect rendering
        check_pattern(&atlas, &glyph, SPLEEN_A, SPLEEN_A_PITCH);
    }

    #[test]
    fn bitmap_glyph_bdf() {
        spleen_test(embedded::SPLEEN_BDF);
    }

    #[test]
    fn bitmap_glyph_pcf() {
        spleen_test(embedded::SPLEEN_PCF);
    }

    #[test]
    fn bitmap_glyph_otb() {
        spleen_test(embedded::SPLEEN_OTB);
    }

    // ---- extra tests ----

    #[test]
    fn variation_axes_can_be_set() {
        let lib = Library::init().unwrap();
        let mut face = init(&lib, embedded::VARIABLE, 12.0, 96);
        let o = opts(12.0, 96);
        face.set_variations(
            &[Variation {
                id: super::super::VariationId::init(b"wght"),
                value: 800.0,
            }],
            o,
        )
        .unwrap();
        // A non-variable font ignores variations.
        let mut face = init(&lib, embedded::INCONSOLATA, 12.0, 96);
        face.set_variations(
            &[Variation {
                id: super::super::VariationId::init(b"wght"),
                value: 800.0,
            }],
            o,
        )
        .unwrap();
    }

    #[test]
    fn synthetic_bold_and_italic_render() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Grayscale);
        let face = init(&lib, embedded::INCONSOLATA, 12.0, 96);
        let o = opts(12.0, 96);
        let bold = face.synthetic_bold(o).unwrap();
        let italic = face.synthetic_italic(o).unwrap();
        assert!(bold.synthetic.bold && !bold.synthetic.italic);
        assert!(italic.synthetic.italic && !italic.synthetic.bold);
        let ro = RenderOptions::new(Metrics::calc(face.get_metrics()));
        let gi = face.glyph_index('A' as u32).unwrap();
        let g0 = face.render_glyph(&mut atlas, gi, ro).unwrap();
        let g1 = bold.render_glyph(&mut atlas, gi, ro).unwrap();
        let g2 = italic.render_glyph(&mut atlas, gi, ro).unwrap();
        assert!(g1.width >= g0.width);
        assert!(g2.width >= g0.width);
        // The original face is still usable after the copies are dropped.
        drop(bold);
        drop(italic);
        assert_eq!("Inconsolata", face.name());
    }

    #[test]
    fn wrong_atlas_is_an_error() {
        let lib = Library::init().unwrap();
        let mut atlas = Atlas::new(512, atlas::Format::Bgra);
        let face = init(&lib, embedded::INCONSOLATA, 12.0, 96);
        let r = face.render_glyph(
            &mut atlas,
            face.glyph_index('A' as u32).unwrap(),
            RenderOptions::new(Metrics::calc(face.get_metrics())),
        );
        assert!(matches!(r, Err(RenderError::WrongAtlas)));
    }

    #[test]
    fn metrics_are_sane() {
        let lib = Library::init().unwrap();
        let face = init(&lib, embedded::VARIABLE, 12.0, 96);
        let m = face.get_metrics();
        assert!(m.px_per_em > 0.0);
        assert!(m.cell_width > 0.0);
        assert!(m.ascent > 0.0 && m.descent < 0.0);
        assert!(m.cap_height.is_some() && m.ex_height.is_some());
    }

    #[test]
    fn hb_font_is_available() {
        let lib = Library::init().unwrap();
        let face = init(&lib, embedded::INCONSOLATA, 12.0, 96);
        assert!(!face.hb_font().as_ptr().is_null());
    }

    #[test]
    fn init_file_from_disk() {
        let lib = Library::init().unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/res/Inconsolata-Regular.ttf");
        let face = Face::init_file(&lib, path, 0, opts(12.0, 96)).unwrap();
        assert_eq!("Inconsolata", face.name());
        assert!(Face::init_file(&lib, "/nonexistent/font.ttf", 0, opts(12.0, 96)).is_err());
    }
}
