//! CoreText font face (macOS / iOS). Port of `font/face/coretext.zig`.
//!
//! Compile-checked on Linux with `cargo check --target aarch64-apple-darwin`;
//! it cannot be run here. The platform-independent math (metrics priority,
//! glyph bearings/canvas size, synthetic bold width) lives in
//! [`super::coretext_math`] and is unit tested everywhere.
#![allow(unsafe_code)]

use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFData, CFNumber, CFRetained, CFString, CGAffineTransform, CGPoint};
use objc2_core_graphics::{
    CGBitmapContextCreate,
    CGColorSpace,
    CGContext,
    CGImageAlphaInfo,
    CGImageByteOrderInfo,
    CGTextDrawingMode,
    kCGColorSpaceDisplayP3,
    kCGColorSpaceLinearGray,
};
use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits};

use super::{
    DesiredSize,
    FontData,
    Options,
    RenderOptions,
    Variation,
    coretext_math::{self as math, CtMetrics, Measurements, Rect, synthetic_bold_line_width},
};
use crate::{
    atlas::{self, Atlas},
    coretext_util::{self as util, CfBox},
    glyph::Glyph,
    library::Library,
    metrics::FaceMetrics,
    opentype,
};

#[derive(Debug, thiserror::Error)]
pub enum FaceError {
    /// CoreText could not create a font from the given data.
    #[error("font initialization failed")]
    FontInitFailure,
    /// The `SVG ` table is malformed.
    #[error("invalid SVG table")]
    InvalidSvgTable,
    /// The face has no backing file (used by the `coretext_freetype` backend).
    #[error("font has no file")]
    FontHasNoFile,
    /// The font file path could not be decoded.
    #[error("font path can't be decoded")]
    FontPathCantDecode,
    /// Creating a font variation failed.
    #[error("variation failed")]
    VariationFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Atlas(#[from] atlas::Error),
    /// The atlas format does not match the glyph's color depth.
    #[error("invalid atlas format")]
    InvalidAtlasFormat,
    /// A CoreGraphics color space or bitmap context could not be created.
    #[error("coregraphics resource creation failed")]
    ContextFailed,
    /// The glyph index doesn't fit a CoreText `CGGlyph` (16 bits).
    #[error("invalid glyph index")]
    InvalidGlyph,
}

/// A harfbuzz font owned by a [`Face`] (only used by the
/// `coretext_harfbuzz` backend).
#[cfg(font_harfbuzz)]
pub struct HbFont(NonNull<harfbuzz_sys::hb_font_t>);

// SAFETY: hb_font_t is thread-safe for shaping when not mutated; Ghostty uses
// it behind the collection lock exactly like upstream.
#[cfg(font_harfbuzz)]
unsafe impl Send for HbFont {}
// SAFETY: see above.
#[cfg(font_harfbuzz)]
unsafe impl Sync for HbFont {}

#[cfg(font_harfbuzz)]
impl HbFont {
    /// The raw `hb_font_t` pointer (valid while the `Face` lives).
    pub fn as_ptr(&self) -> *mut harfbuzz_sys::hb_font_t {
        self.0.as_ptr()
    }
}

#[cfg(font_harfbuzz)]
impl Drop for HbFont {
    fn drop(&mut self) {
        // SAFETY: we own one reference to the hb font.
        unsafe { harfbuzz_sys::hb_font_destroy(self.0.as_ptr()) };
    }
}

pub struct Face {
    /// Our font face
    font: CfBox<CTFont>,

    /// Harfbuzz font corresponding to this face. We only use this
    /// if we're using Harfbuzz.
    #[cfg(font_harfbuzz)]
    hb_font: HbFont,

    /// Set quirks.disableDefaultFontFeatures
    pub quirks_disable_default_font_features: bool,

    /// True if this font face should be rasterized with a synthetic bold
    /// effect. This is used for fonts that don't have a bold variant.
    pub synthetic_bold: Option<f64>,

    /// If the face can possibly be colored, then this is the state
    /// used to check for color information. This is None if the font
    /// can't possibly be colored (i.e. doesn't have SVG, sbix, etc
    /// tables).
    color: Option<ColorState>,

    /// The current size this font is set to.
    pub size: DesiredSize,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("size", &self.size)
            .field("synthetic_bold", &self.synthetic_bold)
            .finish_non_exhaustive()
    }
}

/// The matrix applied to a regular font to auto-italicize it.
fn italic_skew() -> CGAffineTransform {
    CGAffineTransform {
        a: 1.0,
        b: 0.0,
        c: math::ITALIC_SKEW_C, // approx. tan(15)
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    }
}

impl Face {
    /// Initialize a CoreText-based font from a TTF/TTC in memory.
    pub fn init(_lib: &Library, source: FontData, opts: Options) -> Result<Face, FaceError> {
        let data = match source {
            FontData::Static(s) => CFData::from_static_bytes(s),
            FontData::Shared(s) => CFData::from_bytes(&s),
        };

        // SAFETY: `data` is a valid CFData.
        let desc = unsafe { objc2_core_text::CTFontManagerCreateFontDescriptorFromData(&data) }
            .ok_or(FaceError::FontInitFailure)?;

        // SAFETY: valid descriptor, null matrix is allowed.
        let ct_font = unsafe { CTFont::with_font_descriptor(&desc, 12.0, ptr::null()) };

        Self::init_font_copy(&ct_font, opts)
    }

    /// Initialize a CoreText-based face from another initialized font face
    /// but with a new size. This is often how CoreText fonts are initialized
    /// because the font is loaded at a default size during discovery, and then
    /// adjusted to the final size for final load.
    pub fn init_font_copy(base: &CTFont, opts: Options) -> Result<Face, FaceError> {
        // Create a copy. The copyWithAttributes docs say the size is in points,
        // but we need to scale the points by the DPI and to do that we use our
        // function called "pixels".
        // SAFETY: null matrix and no descriptor are allowed.
        let ct_font =
            unsafe { base.copy_with_attributes(f64::from(opts.size.pixels()), ptr::null(), None) };

        Self::init_font(ct_font, opts)
    }

    /// Initialize a face with a CTFont. This takes ownership of the CTFont.
    pub fn init_font(ct_font: CFRetained<CTFont>, opts: Options) -> Result<Face, FaceError> {
        // SAFETY: plain CoreText call.
        let traits = unsafe { ct_font.symbolic_traits() };

        #[cfg(font_harfbuzz)]
        let hb_font = {
            // SAFETY: valid CTFont; hb takes its own reference.
            let p = unsafe { harfbuzz_sys::coretext::hb_coretext_font_create(&ct_font) };
            let hb = NonNull::new(p)
                .map(HbFont)
                .ok_or(FaceError::FontInitFailure)?;
            // 26.6 fixed point scale
            let scale = (f64::from(opts.size.pixels()) * 64.0).round() as i32;
            // SAFETY: valid hb font.
            unsafe { harfbuzz_sys::hb_font_set_scale(hb.as_ptr(), scale, scale) };
            hb
        };

        let color = if traits.contains(CTFontSymbolicTraits::TraitColorGlyphs) {
            Some(ColorState::new(&ct_font)?)
        } else {
            None
        };

        let result = Face {
            font: CfBox::new(ct_font),
            #[cfg(font_harfbuzz)]
            hb_font,
            quirks_disable_default_font_features: false,
            synthetic_bold: None,
            color,
            size: opts.size,
        };

        // In debug mode, we output information about available variation axes,
        // if they exist.
        if cfg!(debug_assertions)
            && let Some(axes) = util::variation_axes(&result.font)
        {
            log::debug!("variation axes font={}", result.name());
            for axis in axes {
                log::debug!(
                    "variation axis: name={} id={} min={} max={} def={}",
                    axis.name,
                    String::from_utf8_lossy(&axis.id.to_be_bytes()),
                    axis.min,
                    axis.max,
                    axis.default,
                );
            }
        }

        Ok(result)
    }

    /// Return a new face that is the same as this but has a transformation
    /// matrix applied to italicize it.
    pub fn synthetic_italic(&self, opts: Options) -> Result<Face, FaceError> {
        let skew = italic_skew();
        // SAFETY: `skew` outlives the call.
        let ct_font = unsafe { self.font.copy_with_attributes(0.0, &raw const skew, None) };
        Self::init_font(ct_font, opts)
    }

    /// Return a new face that is the same as this but applies a synthetic
    /// bold effect to it. This is useful for fonts that don't have a bold
    /// variant.
    pub fn synthetic_bold(&self, opts: Options) -> Result<Face, FaceError> {
        // SAFETY: null matrix and no descriptor are allowed.
        let ct_font = unsafe { self.font.copy_with_attributes(0.0, ptr::null(), None) };
        let mut face = Self::init_font(ct_font, opts)?;
        face.synthetic_bold = Some(synthetic_bold_line_width(f64::from(opts.size.points)));
        Ok(face)
    }

    /// Returns the font name (the family name).
    pub fn name(&self) -> String {
        // SAFETY: plain CoreText call.
        util::cf_to_string(&*unsafe { self.font.family_name() })
    }

    /// Resize the font in-place. If this succeeds, the caller is responsible
    /// for clearing any glyph caches, font atlas data, etc.
    pub fn set_size(&mut self, opts: Options) -> Result<(), FaceError> {
        // We just create a copy and replace ourself
        let face = Self::init_font_copy(&self.font, opts)?;
        *self = face;
        Ok(())
    }

    /// Set the variation axes for this font. This will modify this font
    /// in-place.
    pub fn set_variations(&mut self, vs: &[Variation], opts: Options) -> Result<(), FaceError> {
        // If we have no variations, we don't need to do anything.
        if vs.is_empty() {
            return Ok(());
        }

        // Create a new font descriptor with all the variations set.
        // SAFETY: plain CoreText call.
        let mut desc = unsafe { self.font.font_descriptor() };
        for v in vs {
            let id = CFNumber::new_i32(v.id.0 as i32);
            // SAFETY: valid number and descriptor.
            desc = unsafe { desc.copy_with_variation(&id, v.value) };
        }

        // Put our current size in the opts so that we don't change size.
        let mut new_opts = opts;
        new_opts.size = self.size;

        // Initialize a font based on these attributes.
        // SAFETY: null matrix is allowed.
        let ct_font = unsafe {
            self.font
                .copy_with_attributes(0.0, ptr::null(), Some(&desc))
        };
        let face = Self::init_font(ct_font, new_opts)?;
        *self = face;
        Ok(())
    }

    /// Returns true if the face has any glyphs that are colorized.
    /// To determine if an individual glyph is colorized you must use
    /// `is_color_glyph`.
    pub fn has_color(&self) -> bool {
        self.color.is_some()
    }

    /// Returns true if the given glyph ID is colorized.
    pub fn is_color_glyph(&self, glyph_id: u32) -> bool {
        match &self.color {
            Some(c) => c.is_color_glyph(glyph_id),
            None => false,
        }
    }

    /// Returns the glyph index for the given Unicode code point. If this
    /// face doesn't support this glyph, None is returned.
    pub fn glyph_index(&self, cp: u32) -> Option<u32> {
        let (glyph, ok) = util::glyphs_for_codepoint(&self.font, cp);
        if !ok {
            return None;
        }
        Some(u32::from(glyph))
    }

    /// Render a glyph using the glyph index. The rendered glyph is stored in
    /// the given texture atlas.
    pub fn render_glyph(
        &self,
        atlas: &mut Atlas,
        glyph_index: u32,
        opts: RenderOptions,
    ) -> Result<Glyph, RenderError> {
        let mut glyph = [u16::try_from(glyph_index).map_err(|_| RenderError::InvalidGlyph)?];
        // SAFETY: one valid glyph in the buffer.
        let ct_rect = unsafe {
            self.font.bounding_rects_for_glyphs(
                CTFontOrientation::Horizontal,
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                ptr::null_mut(),
                1,
            )
        };
        // This is in a coordinate space with (0.0, 0.0)
        // in the bottom left and +Y pointing up.
        let rect = Rect {
            x: ct_rect.origin.x,
            y: ct_rect.origin.y,
            width: ct_rect.size.width,
            height: ct_rect.size.height,
        };

        // Determine whether this is a color glyph.
        let is_color = self.is_color_glyph(glyph_index);
        // And whether it's (probably) a bitmap (sbix).
        let sbix = is_color && self.color.as_ref().is_some_and(|c| c.sbix);

        let Some(layout) = math::glyph_layout(rect, sbix, self.synthetic_bold, &opts) else {
            return Ok(Glyph {
                width: 0,
                height: 0,
                offset_x: 0,
                offset_y: 0,
                atlas_x: 0,
                atlas_y: 0,
            });
        };

        // Settings that are specific to if we are rendering text or emoji.
        let depth = math::bitmap_depth(is_color);
        let (space, context_opts) = if is_color {
            // SAFETY: valid static CFString constant.
            let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceDisplayP3 }));
            (
                space,
                CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0,
            )
        } else {
            // SAFETY: valid static CFString constant.
            let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceLinearGray }));
            (space, CGImageAlphaInfo::Only.0)
        };
        let space = space.ok_or(RenderError::ContextFailed)?;

        // This is just a safety check.
        if u32::from(atlas.format.depth()) != depth {
            log::warn!(
                "font atlas color depth doesn't equal font color depth atlas={} font={}",
                atlas.format.depth(),
                depth,
            );
            return Err(RenderError::InvalidAtlasFormat);
        }

        // Our buffer for rendering. We could cache this but glyph rasterization
        // usually stabilizes pretty quickly and is very infrequent so I think
        // the allocation overhead is acceptable compared to the cost of
        // caching it forever or having to deal with a cache lifetime.
        let width = layout.px_width as usize;
        let height = layout.px_height as usize;
        let mut buf = vec![0u8; width * height * depth as usize];

        // SAFETY: `buf` is `width * height * depth` bytes, matching the
        // bytes-per-row and dimensions given; it outlives the context (the
        // context is dropped before `buf` at the end of this function).
        let ctx = unsafe {
            CGBitmapContextCreate(
                buf.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * depth as usize,
                Some(&space),
                context_opts,
            )
        }
        .ok_or(RenderError::ContextFailed)?;
        let c = Some(&*ctx);

        // Perform an initial fill. This ensures that we don't have any
        // uninitialized pixels in the bitmap.
        if is_color {
            CGContext::set_rgb_fill_color(c, 0.0, 0.0, 0.0, 0.0);
        } else {
            CGContext::set_gray_fill_color(c, 0.0, 0.0);
        }
        CGContext::fill_rect(
            c,
            objc2_core_foundation::CGRect::new(
                CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(width as f64, height as f64),
            ),
        );

        // "Font smoothing" is what we call "thickening", it's an attempt
        // to compensate for optical thinning of fonts, but at this point
        // it's just something that makes the text look closer to system
        // applications if users want that.
        CGContext::set_allows_font_smoothing(c, true);
        CGContext::set_should_smooth_fonts(c, opts.thicken);

        // Subpixel positioning allows glyphs to be placed at non-integer
        // coordinates. We need this for our alignment.
        CGContext::set_allows_font_subpixel_positioning(c, true);
        CGContext::set_should_subpixel_position_fonts(c, true);

        // We don't want subpixel quantization, since we very carefully
        // manage the position of our glyphs ourselves, and dont want to
        // mess that up.
        CGContext::set_allows_font_subpixel_quantization(c, false);
        CGContext::set_should_subpixel_quantize_fonts(c, false);

        // Anti-aliasing is self explanatory.
        CGContext::set_allows_antialiasing(c, true);
        CGContext::set_should_antialias(c, true);

        // Set our color for drawing
        if is_color {
            CGContext::set_rgb_fill_color(c, 1.0, 1.0, 1.0, 1.0);
            CGContext::set_rgb_stroke_color(c, 1.0, 1.0, 1.0, 1.0);
        } else {
            let strength = f64::from(opts.thicken_strength);
            CGContext::set_gray_fill_color(c, strength / 255.0, 1.0);
            CGContext::set_gray_stroke_color(c, strength / 255.0, 1.0);
        }

        // If we are drawing with synthetic bold then use a fill stroke
        // which strokes the outlines of the glyph making a more bold look.
        if let Some(line_width) = self.synthetic_bold {
            CGContext::set_text_drawing_mode(c, CGTextDrawingMode::FillStroke);
            CGContext::set_line_width(c, line_width);
        }

        // Translate our drawing context so that when we draw our
        // glyph the bottom/left edge is at the correct sub-pixel
        // position. The bottom/left edges are guaranteed to be at
        // exactly [0, 0] relative to this because when we call to
        // `drawGlyphs`, we pass the negated bearings.
        CGContext::translate_ctm(
            c,
            layout.frac_x + f64::from(layout.canvas_padding),
            layout.frac_y + f64::from(layout.canvas_padding),
        );

        // Scale the drawing context so that when we draw
        // our glyph it's stretched to the constrained size.
        CGContext::scale_ctm(c, layout.scale_x, layout.scale_y);

        // Draw our glyph.
        //
        // We offset the position by the negated bearings so that the
        // glyph is drawn at exactly [0, 0], which is then offset to
        // the appropriate fractional position by the translation we
        // did before scaling.
        let mut position = CGPoint::new(-layout.rect_x, -layout.rect_y);
        // SAFETY: one glyph and one position, valid context.
        unsafe {
            self.font.draw_glyphs(
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                NonNull::from(&mut position),
                1,
                &ctx,
            );
        }

        // The context must be released before we read the buffer.
        drop(ctx);

        // Write our rasterized glyph to the atlas.
        let region = atlas.reserve(layout.px_width, layout.px_height)?;
        atlas.set(region, &buf);

        Ok(Glyph {
            width: layout.px_width,
            height: layout.px_height,
            offset_x: layout.offset_x(),
            offset_y: layout.offset_y(),
            atlas_x: region.x,
            atlas_y: region.y,
        })
    }

    /// Get the `FaceMetrics` for this face.
    pub fn get_metrics(&self) -> FaceMetrics {
        let ct_font: &CTFont = &self.font;

        // Read the 'head' table out of the font data.
        //
        // macOS bitmap-only fonts use a 'bhed' tag rather than 'head', but
        // the table format is byte-identical to the 'head' table, so if we
        // can't find 'head' we try 'bhed' instead before failing.
        //
        // ref: https://fontforge.org/docs/techref/bitmaponlysfnt.html
        let head = util::font_table_bytes(ct_font, b"head")
            .or_else(|| util::font_table_bytes(ct_font, b"bhed"))
            .and_then(|d| {
                opentype::Head::init(&d)
                    .inspect_err(|e| log::warn!("error parsing head table: {e}"))
                    .ok()
            });

        // Read the 'post' table out of the font data.
        let post = util::font_table_bytes(ct_font, b"post").and_then(|d| {
            opentype::Post::init(&d)
                .inspect_err(|e| log::warn!("error parsing post table: {e}"))
                .ok()
        });

        // Read the 'OS/2' table out of the font data if it's available.
        let os2 = util::font_table_bytes(ct_font, b"OS/2").and_then(|d| {
            opentype::OS2::init(&d)
                .inspect_err(|e| log::warn!("error parsing OS/2 table: {e}"))
                .ok()
        });

        // Read the 'hhea' table out of the font data.
        let hhea = util::font_table_bytes(ct_font, b"hhea").and_then(|d| {
            opentype::Hhea::init(&d)
                .inspect_err(|e| log::warn!("error parsing hhea table: {e}"))
                .ok()
        });

        // SAFETY: plain CoreText getters.
        let (units_per_em, px_per_em, ct) = unsafe {
            (
                head.map_or_else(
                    || f64::from(ct_font.units_per_em()),
                    |h| f64::from(h.unitsPerEm),
                ),
                ct_font.size(),
                CtMetrics {
                    ascent: ct_font.ascent(),
                    descent: ct_font.descent(),
                    leading: ct_font.leading(),
                    cap_height: ct_font.cap_height(),
                    x_height: ct_font.x_height(),
                },
            )
        };

        // Cell width is calculated by calculating the widest width of the
        // visible ASCII characters. Usually 'M' is widest but we just take
        // whatever is widest.
        //
        // ASCII height is calculated as the height of the overall bounding
        // box of the same characters.
        let (cell_width, ascii_height) = {
            // All the visible ASCII chars
            let mut unichars: Vec<u16> = (32u16..127).collect();
            let n = unichars.len();

            // Get our glyph IDs for the ASCII chars
            let mut glyphs = vec![0u16; n];
            // SAFETY: both buffers have `n` elements.
            unsafe {
                ct_font.glyphs_for_characters(
                    NonNull::new_unchecked(unichars.as_mut_ptr()),
                    NonNull::new_unchecked(glyphs.as_mut_ptr()),
                    n as isize,
                );
            }

            // Get all our advances
            let mut advances = vec![objc2_core_foundation::CGSize::new(0.0, 0.0); n];
            // SAFETY: buffers have `n` elements.
            unsafe {
                ct_font.advances_for_glyphs(
                    CTFontOrientation::Horizontal,
                    NonNull::new_unchecked(glyphs.as_mut_ptr()),
                    advances.as_mut_ptr(),
                    n as isize,
                );
            }

            // Find the maximum advance
            let max = advances.iter().fold(0.0f64, |m, a| a.width.max(m));

            // Get the overall bounding rect for the glyphs
            // SAFETY: `glyphs` has `n` elements.
            let rect = unsafe {
                ct_font.bounding_rects_for_glyphs(
                    CTFontOrientation::Horizontal,
                    NonNull::new_unchecked(glyphs.as_mut_ptr()),
                    ptr::null_mut(),
                    n as isize,
                )
            };

            (max, rect.size.height)
        };

        // Measure "水" (CJK water ideograph, U+6C34) for our ic width.
        let ic_width = self.glyph_index('水' as u32).and_then(|glyph| {
            let mut g = [glyph as u16];
            // SAFETY: one valid glyph.
            let (advance, bounds) = unsafe {
                (
                    ct_font.advances_for_glyphs(
                        CTFontOrientation::Horizontal,
                        NonNull::new_unchecked(g.as_mut_ptr()),
                        ptr::null_mut(),
                        1,
                    ),
                    ct_font.bounding_rects_for_glyphs(
                        CTFontOrientation::Horizontal,
                        NonNull::new_unchecked(g.as_mut_ptr()),
                        ptr::null_mut(),
                        1,
                    ),
                )
            };

            // If the advance of the glyph is less than the width of the actual
            // glyph then we just treat it as invalid since it's probably wrong
            // and using it for size normalization will instead make the font
            // way too big.
            //
            // This can sometimes happen if there's a CJK font that has been
            // patched with the nerd fonts patcher and it butchers the advance
            // values so the advance ends up half the width of the actual glyph.
            if !math::ic_width_is_valid(bounds.size.width, advance) {
                log::warn!(
                    "(getMetrics) Width of glyph '水' for font \"{}\" is greater than its advance ({} > {}), discarding ic_width metric.",
                    self.name(),
                    bounds.size.width,
                    advance,
                );
                return None;
            }

            Some(advance)
        });

        math::face_metrics(
            px_per_em,
            units_per_em,
            hhea.as_ref(),
            os2.as_ref(),
            post.as_ref(),
            &ct,
            Measurements {
                cell_width,
                ascii_height,
                ic_width,
            },
        )
    }

    /// Copy the font table data for the given tag.
    pub fn copy_table(&self, tag: &[u8; 4]) -> Result<Option<Vec<u8>>, FaceError> {
        Ok(util::font_table_bytes(&self.font, tag))
    }

    /// The underlying CTFont (for shapers).
    pub(crate) fn ct_font(&self) -> &CTFont {
        &self.font
    }

    /// The harfbuzz font corresponding to this face (for the HarfBuzz shaper).
    #[cfg(font_harfbuzz)]
    pub fn hb_font(&self) -> &HbFont {
        &self.hb_font
    }

    /// The `FontHasNoFile`-style path of this face's file (for the
    /// `coretext_freetype` backend this is done by the deferred face).
    pub fn font_descriptor(&self) -> CFRetained<objc2_core_text::CTFontDescriptor> {
        // SAFETY: plain CoreText call.
        unsafe { self.font.font_descriptor() }
    }
}

/// The state associated with a font face that may have colorized glyphs.
/// This is used to determine if a specific glyph ID is colorized.
struct ColorState {
    /// True if there is an sbix font table. For now, the mere presence
    /// of an sbix font table causes us to assume the glyph is colored.
    /// We can improve this later.
    sbix: bool,

    /// The SVG font table data (if any), which we can use to determine
    /// if a glyph is present in the SVG table.
    svg: Option<SvgTable>,
}

/// An owned copy of the `SVG ` table and the glyph range it covers.
struct SvgTable {
    data: Vec<u8>,
}

impl ColorState {
    fn new(f: &CTFont) -> Result<ColorState, FaceError> {
        // sbix is true if the table exists in the font data at all.
        // In the future we probably want to actually parse it and
        // check for glyphs.
        let sbix = util::font_table(f, b"sbix").is_some_and(|d| !d.is_empty());

        // Read the SVG table out of the font data.
        let svg = match util::font_table_bytes(f, b"SVG ") {
            None => None,
            Some(data) => {
                // Validate the table up front like Zig.
                opentype::SVG::init(&data).map_err(|_| FaceError::InvalidSvgTable)?;
                Some(SvgTable { data })
            }
        };

        Ok(ColorState { sbix, svg })
    }

    /// Returns true if the given glyph ID is colored.
    fn is_color_glyph(&self, glyph_id: u32) -> bool {
        // Our font system uses 32-bit glyph IDs for special values but
        // actual fonts only contain 16-bit glyph IDs so if we can't cast
        // into it it must be false.
        let Ok(glyph_u16) = u16::try_from(glyph_id) else {
            return false;
        };

        // sbix is always true for now
        if self.sbix {
            return true;
        }

        // if we have svg data, check it
        if let Some(svg) = &self.svg
            && let Ok(table) = opentype::SVG::init(&svg.data)
            && table.has_glyph(glyph_u16)
        {
            return true;
        }

        false
    }
}

/// Create a CTFont by name at the given size (test/dev helper equivalent of
/// `FontDescriptor.createWithNameAndSize` + `Font.createWithFontDescriptor`).
pub fn font_with_name(name: &str, size: f64) -> CFRetained<CTFont> {
    let cf_name = CFString::from_str(name);
    // SAFETY: valid name; null matrix is allowed.
    unsafe { CTFont::with_name(&cf_name, size, ptr::null()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{embedded, metrics::Metrics};

    fn opts(points: f32) -> Options {
        Options::new(DesiredSize::new(points))
    }

    fn render_ascii(face: &Face) {
        let mut atlas = Atlas::new(512, atlas::Format::Grayscale);
        for i in 32u32..127 {
            let g = face.glyph_index(i).unwrap();
            face.render_glyph(
                &mut atlas,
                g,
                RenderOptions::new(Metrics::calc(face.get_metrics())),
            )
            .unwrap();
        }
    }

    #[test]
    fn monaco_ascii() {
        let font = font_with_name("Monaco", 12.0);
        let face = Face::init_font_copy(&font, opts(12.0)).unwrap();
        render_ascii(&face);
    }

    #[test]
    fn name() {
        let font = font_with_name("Menlo", 12.0);
        let face = Face::init_font_copy(&font, opts(12.0)).unwrap();
        assert_eq!(face.name(), "Menlo");
    }

    #[test]
    fn emoji() {
        let font = font_with_name("Apple Color Emoji", 12.0);
        let face = Face::init_font_copy(&font, opts(18.0)).unwrap();
        let id = face.glyph_index('🥸' as u32).unwrap();
        assert!(face.is_color_glyph(id));
    }

    #[test]
    fn in_memory() {
        let lib = Library::init().unwrap();
        let face = Face::init(&lib, FontData::Static(embedded::REGULAR), opts(12.0)).unwrap();
        render_ascii(&face);
    }

    #[test]
    fn variable() {
        let lib = Library::init().unwrap();
        let face = Face::init(&lib, FontData::Static(embedded::VARIABLE), opts(12.0)).unwrap();
        render_ascii(&face);
    }

    #[test]
    fn variable_set_variation() {
        let lib = Library::init().unwrap();
        let mut face = Face::init(&lib, FontData::Static(embedded::VARIABLE), opts(12.0)).unwrap();
        face.set_variations(
            &[Variation {
                id: super::super::VariationId::init(b"wght"),
                value: 400.0,
            }],
            opts(12.0),
        )
        .unwrap();
        render_ascii(&face);
    }

    #[test]
    fn svg_font_table() {
        let lib = Library::init().unwrap();
        let face = Face::init(&lib, FontData::Static(embedded::JULIA_MONO), opts(12.0)).unwrap();
        let table = face.copy_table(b"SVG ").unwrap().unwrap();
        assert_ne!(table.len(), 0);
    }

    #[test]
    fn glyph_index_colored_vs_text() {
        let lib = Library::init().unwrap();
        let face = Face::init(&lib, FontData::Static(embedded::JULIA_MONO), opts(12.0)).unwrap();
        let glyph = face.glyph_index('A' as u32).unwrap();
        assert_eq!(glyph, 4);
        assert!(!face.is_color_glyph(glyph));

        let glyph = face.glyph_index(0xE800).unwrap();
        assert_eq!(glyph, 11482);
        assert!(face.is_color_glyph(glyph));
    }
}
