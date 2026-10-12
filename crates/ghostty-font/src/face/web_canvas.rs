//! The browser Canvas face. Port of `font/face/web_canvas.zig`.
//!
//! Glyphs are measured and rasterised with a 2D canvas context through the
//! JS interop layer (`ghostty_os::wasm::js`). The JS-backed [`Face`] only
//! exists on `wasm32`; everything that does not touch JS (metrics math,
//! presentation detection, grapheme-cluster glyph indices, bitmap
//! conversion) lives at module level so it is unit tested natively.
//!
//! Deviations from the Zig file (which is stale against the current Atlas):
//! the canvas bitmap is RGBA, Zig matches on a non-existent `atlas.format ==
//! .rgba`; here a `Bgra` atlas receives the bitmap with R/B swapped (the atlas
//! is BGRA, like `Atlas.Wasm.atlas_debug_canvas` assumes) and a `Grayscale`
//! atlas receives the alpha channel.

use std::collections::HashMap;

use crate::{Presentation, atlas, metrics::Metrics};

/// Graphemes (multi-codepoint clusters) get "glyph indexes" one above the
/// maximum unicode codepoint up to the maximum 32-bit unsigned integer.
pub const GRAPHEME_START: u32 = 0x10FFFF + 1;
pub const GRAPHEME_END: u32 = u32::MAX;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("out of memory")]
    OutOfMemory,
    #[error("unknown grapheme cluster")]
    UnknownGraphemeCluster,
    #[error("unsupported atlas format")]
    UnsupportedAtlasFormat,
    #[error("invalid codepoint")]
    InvalidCodepoint,
    #[error("canvas container not found")]
    CanvasContainerNotFound,
    #[error(transparent)]
    Atlas(#[from] atlas::Error),
    #[cfg(target_arch = "wasm32")]
    #[error("javascript error: {0}")]
    Js(#[from] ghostty_os::wasm::js::Error),
}

/// The map that stores multi-codepoint grapheme clusters that are rendered.
#[derive(Debug)]
pub struct GraphemeTable {
    to_glyph: HashMap<Vec<u8>, u32>,
    from_glyph: HashMap<u32, Vec<u8>>,
    next: u32,
}

impl Default for GraphemeTable {
    fn default() -> Self {
        Self {
            to_glyph: HashMap::new(),
            from_glyph: HashMap::new(),
            next: GRAPHEME_START,
        }
    }
}

impl GraphemeTable {
    /// Returns the glyph index for the given grapheme cluster (UTF-8). The
    /// same cluster will always map to the same glyph index. This does not
    /// render the grapheme, only reserves the index.
    pub fn glyph_index(&mut self, cluster: &[u8]) -> Result<u32, Error> {
        // If we already have this stored then return it
        if let Some(v) = self.to_glyph.get(cluster) {
            return Ok(*v);
        }

        // We don't have it stored. The next will be "0" if we're out of
        // space due to unsigned int wrapping.
        if self.next == 0 {
            return Err(Error::OutOfMemory);
        }

        let index = self.next;
        self.to_glyph.insert(cluster.to_vec(), index);
        self.from_glyph.insert(index, cluster.to_vec());
        self.next = self.next.wrapping_add(1);
        Ok(index)
    }

    /// The UTF-8 bytes of the cluster for a grapheme glyph index.
    pub fn cluster(&self, glyph_index: u32) -> Option<&[u8]> {
        self.from_glyph.get(&glyph_index).map(Vec::as_slice)
    }

    pub fn contains(&self, glyph_index: u32) -> bool {
        self.from_glyph.contains_key(&glyph_index)
    }
}

/// Measurements from `CanvasRenderingContext2D.measureText`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub actual_bounding_box_left: f32,
    pub actual_bounding_box_right: f32,
    pub actual_bounding_box_ascent: f32,
    pub actual_bounding_box_descent: f32,
}

impl TextMetrics {
    /// We prefer the bounding box since it is tighter but certain text such
    /// as emoji do not have a bounding box set so we use the full run width.
    pub fn extent_width(&self) -> f32 {
        if self.actual_bounding_box_right > 0.0 {
            self.actual_bounding_box_right
        } else {
            self.width
        }
    }
}

/// Calculate the face metrics from the measurement of "M" (cell width) and
/// of "M_" (high and low character: ascent + descent is the cell height).
pub fn calc_metrics(m: TextMetrics, m_underscore: TextMetrics) -> Metrics {
    let cell_width = m.extent_width();
    let cell_height =
        m_underscore.actual_bounding_box_ascent + m_underscore.actual_bounding_box_descent;
    let cell_baseline = m_underscore.actual_bounding_box_descent;

    // There isn't a declared underline position for canvas measurements
    // so we just go 1 under the cell height to match freetype logic.
    let underline_position = cell_height - 1.0;
    let underline_thickness = 1.0f32;

    Metrics {
        cell_width: cell_width as u32,
        cell_height: cell_height as u32,
        cell_baseline: cell_baseline as u32,
        underline_position: underline_position as u32,
        underline_thickness: underline_thickness as u32,
        strikethrough_position: underline_position as u32,
        strikethrough_thickness: underline_thickness as u32,
        // The canvas gives no face box; the Zig code leaves these unset (it
        // predates the required fields), so use the cell size.
        face_width: f64::from(cell_width),
        face_height: f64::from(cell_height),
        ..Metrics::init()
    }
}

/// Determines the presentation of a rendered RGBA bitmap by literally
/// inspecting the image data for any color.
pub fn bitmap_presentation(bitmap: &[u8]) -> Presentation {
    let (pixels, _) = bitmap.as_chunks::<4>();
    if pixels.iter().any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0) {
        Presentation::Emoji
    } else {
        Presentation::Text
    }
}

/// The pixel size of a rendered glyph, from its measurement. Returns
/// `(width, height, broken_bbox)`. Width and height both get "+ 1" so that
/// there is a 1px border around the glyph to avoid clipping in the atlas.
pub fn glyph_size(m: TextMetrics) -> (u32, u32, bool) {
    let width = m.extent_width().ceil() as u32 + 1;

    // On Firefox on Linux, the bounding box is broken in some cases for
    // ideographic glyphs (such as emoji). We detect this and behave
    // differently.
    let asc = m.actual_bounding_box_ascent;
    let desc = m.actual_bounding_box_descent;
    let broken_bbox = asc + desc < 0.001;
    let height = if broken_bbox {
        width
    } else {
        (asc + desc).ceil() as u32 + 1
    };
    (width, height, broken_bbox)
}

/// Converts an RGBA canvas bitmap to the byte layout of an atlas format.
pub fn bitmap_for_atlas(rgba: &[u8], format: atlas::Format) -> Result<Vec<u8>, Error> {
    debug_assert_eq!(rgba.len() % 4, 0);
    match format {
        // Swap R and B (the atlas is BGRA).
        atlas::Format::Bgra => {
            let mut out = rgba.to_vec();
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            Ok(out)
        }
        // Convert down to A8
        atlas::Format::Grayscale => Ok(rgba.as_chunks::<4>().0.iter().map(|px| px[3]).collect()),
        atlas::Format::Bgr => Err(Error::UnsupportedAtlasFormat),
    }
}

/// Reserve space in the atlas for a rendered bitmap and write it.
pub fn put_in_atlas(
    atlas: &mut atlas::Atlas,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<crate::Glyph, Error> {
    let formatted = bitmap_for_atlas(rgba, atlas.format)?;
    let region = atlas.reserve(width, height)?;
    if region.width > 0 && region.height > 0 {
        atlas.set(region, &formatted);
    }
    Ok(crate::Glyph {
        width,
        height,
        // TODO: this can't be right (upstream comment)
        offset_x: 0,
        offset_y: 0,
        atlas_x: region.x,
        atlas_y: region.y,
    })
}

#[cfg(target_arch = "wasm32")]
pub use face_js::Face;

#[cfg(target_arch = "wasm32")]
mod face_js {
    use ghostty_os::wasm::js::{self, Object};

    use super::{
        Error,
        GRAPHEME_START,
        GraphemeTable,
        TextMetrics,
        bitmap_presentation,
        calc_metrics,
        glyph_size,
        put_in_atlas,
    };
    use crate::{Glyph, Presentation, atlas::Atlas, face::DesiredSize, metrics::Metrics};

    pub struct Face {
        /// The CSS "font" attribute, excluding size.
        font_str: String,

        /// The size we currently have set.
        pub size: DesiredSize,

        /// The presentation for this font.
        pub presentation: Presentation,

        /// Metrics for this font face. These are useful for renderers.
        pub metrics: Metrics,

        /// The canvas element that we will reuse to render glyphs
        canvas: Object,

        graphemes: GraphemeTable,
    }

    /// An internal (web-canvas-only) format for rendered glyphs since we do
    /// render passes in multiple different situations.
    struct RenderedGlyph {
        width: u32,
        height: u32,
        bitmap: Vec<u8>,
    }

    fn text_metrics(o: &Object) -> Result<TextMetrics, Error> {
        Ok(TextMetrics {
            width: o.get::<f32>("width")?,
            actual_bounding_box_left: o.get::<f32>("actualBoundingBoxLeft")?,
            actual_bounding_box_right: o.get::<f32>("actualBoundingBoxRight")?,
            actual_bounding_box_ascent: o.get::<f32>("actualBoundingBoxAscent")?,
            actual_bounding_box_descent: o.get::<f32>("actualBoundingBoxDescent")?,
        })
    }

    impl Face {
        /// Initialize a web canvas font with a "raw" value. The "raw" value
        /// can be any valid value for a CSS "font" property EXCLUDING the
        /// size. The size is always added via the `size` parameter.
        ///
        /// The presentation is given here directly because the browser gives
        /// us no easy way to determine the presentation we want for this
        /// font. Callers should just tell us what to expect.
        pub fn init_named(
            raw: &str,
            size: DesiredSize,
            presentation: Presentation,
        ) -> Result<Face, Error> {
            // Create our canvas that we're going to continue to reuse.
            let doc = js::global().get::<Object>("document")?;
            let canvas = doc.call::<Object>("createElement", &[&js::string("canvas")])?;

            let mut result = Face {
                font_str: raw.to_owned(),
                size,
                presentation,
                // We're going to calculate these right after initialization.
                metrics: Metrics::init(),
                canvas,
                graphemes: GraphemeTable::default(),
            };
            result.calc_metrics()?;

            log::debug!("face initialized: {raw}");
            Ok(result)
        }

        /// Resize the font in-place. If this succeeds, the caller is
        /// responsible for clearing any glyph caches, font atlas data, etc.
        pub fn set_size(&mut self, size: DesiredSize) -> Result<(), Error> {
            let old = self.size;
            self.size = size;
            if let Err(e) = self.calc_metrics() {
                self.size = old;
                return Err(e);
            }
            Ok(())
        }

        /// Returns the glyph index for the given Unicode code point. For
        /// canvas, we support every glyph and the ID is just the codepoint
        /// since we don't have access to the underlying tables anyways. We
        /// let the browser deal with bad codepoints.
        pub fn glyph_index(&self, cp: u32) -> Option<u32> {
            // If this is a multi-codepoint grapheme then we only check if
            // we actually know about it.
            if cp >= GRAPHEME_START && !self.graphemes.contains(cp) {
                return None;
            }

            // Render the glyph to determine if it is colored or not. We have
            // to do this because the browser will always try to render
            // whatever we give it and we have no API to determine color.
            //
            // We don't want to say yes to the wrong presentation because it
            // will go into the wrong Atlas.
            let p = if cp <= 255 {
                Presentation::Text
            } else {
                // If we are unable to render this glyph then we just say we
                // don't support it.
                self.glyph_presentation(cp).ok()?
            };
            if p != self.presentation {
                return None;
            }

            Some(cp)
        }

        /// This determines the presentation of the glyph by literally
        /// inspecting the image data to look for any color.
        fn glyph_presentation(&self, cp: u32) -> Result<Presentation, Error> {
            let render = self.render_glyph_internal(cp)?;
            Ok(bitmap_presentation(&render.bitmap))
        }

        /// Returns the glyph index for the given grapheme cluster. The same
        /// cluster will always map to the same glyph index. This does not
        /// render the grapheme at this time, only reserves the index.
        pub fn grapheme_glyph_index(&mut self, cluster: &[u8]) -> Result<u32, Error> {
            self.graphemes.glyph_index(cluster)
        }

        /// Render a glyph using the glyph index. The rendered glyph is
        /// stored in the given texture atlas.
        pub fn render_glyph(&self, atlas: &mut Atlas, glyph_index: u32) -> Result<Glyph, Error> {
            let render = self.render_glyph_internal(glyph_index)?;
            put_in_atlas(atlas, render.width, render.height, &render.bitmap)
        }

        /// Calculate the metrics associated with a given face.
        fn calc_metrics(&mut self) -> Result<(), Error> {
            let ctx = self.context()?;

            // Cell width is the width of our M text
            let m = text_metrics(&ctx.call::<Object>("measureText", &[&js::string("M")])?)?;

            // To get the cell height we render a high and low character and
            // get the total of the ascent and descent.
            let hm = text_metrics(&ctx.call::<Object>("measureText", &[&js::string("M_")])?)?;

            self.metrics = calc_metrics(m, hm);
            log::debug!("metrics font={} value={:?}", self.font_str, self.metrics);
            Ok(())
        }

        /// Returns the 2d context configured for drawing.
        fn context(&self) -> Result<Object, Error> {
            // This will return the same context on subsequent calls so it
            // is important to reset it.
            let ctx = self
                .canvas
                .call::<Object>("getContext", &[&js::string("2d")])?;

            // Clear the canvas
            {
                let width = self.canvas.get::<f64>("width")?;
                let height = self.canvas.get::<f64>("height")?;
                ctx.call::<()>("clearRect", &[&0.0_f64, &0.0_f64, &width, &height])?;
            }

            // Set our context font
            let font_val = format!("{}px {}", self.size.points, self.font_str);
            ctx.set("font", js::string(&font_val))?;

            // If the font property didn't change, then the font set didn't
            // work. We do this check because it is very easy to put an
            // invalid font in and this at least makes it show up in the logs.
            let check = ctx.get::<String>("font")?;
            if font_val != check {
                log::warn!(
                    "canvas font didn't set, fonts may be broken, expected={font_val} got={check}"
                );
            }

            Ok(ctx)
        }

        /// Shared logic for rendering a glyph.
        fn render_glyph_internal(&self, glyph_index: u32) -> Result<RenderedGlyph, Error> {
            // Encode our glyph to UTF-8 so we can build a JS string out of it.
            let glyph_str: String = if glyph_index < GRAPHEME_START {
                char::from_u32(glyph_index)
                    .ok_or(Error::InvalidCodepoint)?
                    .to_string()
            } else {
                // We are a multi-codepoint glyph so we have to read the glyph
                // from the map and it is already utf8 encoded.
                let slice = self
                    .graphemes
                    .cluster(glyph_index)
                    .ok_or(Error::UnknownGraphemeCluster)?;
                String::from_utf8_lossy(slice).into_owned()
            };

            // Get our drawing context and the size of the render
            let measure_ctx = self.context()?;
            let m = text_metrics(
                &measure_ctx.call::<Object>("measureText", &[&js::string(&glyph_str)])?,
            )?;
            let (width, height, broken_bbox) = glyph_size(m);

            // Resize canvas to match the glyph size exactly
            {
                self.canvas.set("width", width)?;
                self.canvas.set("height", height)?;

                let style = self.canvas.get::<Object>("style")?;
                style.set("width", js::string(&format!("{width}px")))?;
                style.set("height", js::string(&format!("{height}px")))?;
            }

            // Reload our context since we resized the canvas
            let ctx = self.context()?;

            // For the broken bounding box case we render ideographic
            // baselines so that we just render the glyph fully in the box
            // with no offsets.
            if broken_bbox {
                ctx.set("textBaseline", js::string("ideographic"))?;
            }

            // Draw background
            ctx.set("fillStyle", js::string("transparent"))?;
            ctx.call::<()>("fillRect", &[&0.0_f64, &0.0_f64, &width, &height])?;

            // Draw glyph
            ctx.set("fillStyle", js::string("black"))?;
            ctx.call::<()>(
                "fillText",
                &[
                    &js::string(&glyph_str),
                    &(m.actual_bounding_box_left + 1.0),
                    &if broken_bbox {
                        height as f32
                    } else {
                        m.actual_bounding_box_ascent + 1.0
                    },
                ],
            )?;

            // Read the image data and get it into a Vec<u8> on our side
            let data =
                ctx.call::<Object>("getImageData", &[&0.0_f64, &0.0_f64, &width, &height])?;
            let src_array = data.get::<Object>("data")?;

            // Allocate our local memory to copy the data to.
            let len = src_array.get::<u32>("length")? as usize;
            let mut bitmap = vec![0u8; len];

            // Create our target Uint8Array that we can use to copy from src.
            let mem_array = {
                // Get our runtime memory
                let mem = js::runtime().get::<Object>("memory")?;
                let buf = mem.get::<Object>("buffer")?;

                // Construct our array to peer into our memory
                let uint8_array = js::global().get::<Object>("Uint8Array")?;
                uint8_array.new_instance(&[&buf, &(bitmap.as_mut_ptr() as usize)])?
            };

            // Copy
            mem_array.call::<()>("set", &[&src_array])?;

            Ok(RenderedGlyph {
                width,
                height,
                bitmap,
            })
        }

        /// Appends our canvas to the element with id `face-canvas` (the
        /// debugging hook behind the `face_debug_canvas` export).
        pub fn debug_canvas(&self) -> Result<(), Error> {
            let doc = js::global().get::<Object>("document")?;
            let elem = doc
                .call::<Option<Object>>("getElementById", &[&js::string("face-canvas")])?
                .ok_or(Error::CanvasContainerNotFound)?;
            elem.call::<()>("append", &[&self.canvas])?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{Atlas, Format};

    #[test]
    fn grapheme_indexes_are_stable_and_sequential() {
        let mut t = GraphemeTable::default();
        let a = t.glyph_index("👍🏽".as_bytes()).unwrap();
        let b = t.glyph_index("e\u{301}".as_bytes()).unwrap();
        assert_eq!(a, GRAPHEME_START);
        assert_eq!(b, GRAPHEME_START + 1);
        assert_eq!(t.glyph_index("👍🏽".as_bytes()).unwrap(), a);
        assert_eq!(t.cluster(b), Some("e\u{301}".as_bytes()));
        assert!(t.contains(a));
        assert!(!t.contains(GRAPHEME_START + 2));
    }

    #[test]
    fn grapheme_table_exhaustion_is_oom() {
        let mut t = GraphemeTable {
            next: 0,
            ..GraphemeTable::default()
        };
        assert!(matches!(t.glyph_index(b"ab"), Err(Error::OutOfMemory)));
    }

    #[test]
    fn metrics_prefer_bounding_box() {
        let m = TextMetrics {
            width: 10.0,
            actual_bounding_box_right: 8.7,
            ..TextMetrics::default()
        };
        let hm = TextMetrics {
            actual_bounding_box_ascent: 12.4,
            actual_bounding_box_descent: 3.9,
            ..TextMetrics::default()
        };
        let r = calc_metrics(m, hm);
        assert_eq!(r.cell_width, 8);
        assert_eq!(r.cell_height, 16);
        assert_eq!(r.cell_baseline, 3);
        assert_eq!(r.underline_position, 15);
        assert_eq!(r.underline_thickness, 1);
        assert_eq!(r.strikethrough_position, 15);
        assert_eq!(r.strikethrough_thickness, 1);
    }

    #[test]
    fn metrics_fall_back_to_run_width() {
        let m = TextMetrics {
            width: 9.5,
            ..TextMetrics::default()
        };
        assert_eq!(calc_metrics(m, TextMetrics::default()).cell_width, 9);
    }

    #[test]
    fn presentation_detects_color() {
        assert_eq!(
            bitmap_presentation(&[0, 0, 0, 255, 0, 0, 0, 10]),
            Presentation::Text
        );
        assert_eq!(
            bitmap_presentation(&[0, 0, 0, 255, 0, 5, 0, 255]),
            Presentation::Emoji
        );
        assert_eq!(bitmap_presentation(&[]), Presentation::Text);
    }

    #[test]
    fn glyph_size_has_one_pixel_border() {
        let m = TextMetrics {
            width: 7.0,
            actual_bounding_box_right: 6.2,
            actual_bounding_box_ascent: 9.1,
            actual_bounding_box_descent: 2.0,
            ..TextMetrics::default()
        };
        assert_eq!(glyph_size(m), (8, 13, false));
    }

    #[test]
    fn glyph_size_broken_bbox_is_square() {
        let m = TextMetrics {
            width: 14.0,
            ..TextMetrics::default()
        };
        assert_eq!(glyph_size(m), (15, 15, true));
    }

    #[test]
    fn bitmap_conversion() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            bitmap_for_atlas(&rgba, Format::Bgra).unwrap(),
            [3, 2, 1, 4, 7, 6, 5, 8]
        );
        assert_eq!(bitmap_for_atlas(&rgba, Format::Grayscale).unwrap(), [4, 8]);
        assert!(matches!(
            bitmap_for_atlas(&rgba, Format::Bgr),
            Err(Error::UnsupportedAtlasFormat)
        ));
    }

    #[test]
    fn put_in_atlas_writes_alpha() {
        let mut atlas = Atlas::new(32, Format::Grayscale);
        let rgba = [0, 0, 0, 200, 0, 0, 0, 100, 0, 0, 0, 50, 0, 0, 0, 25];
        let g = put_in_atlas(&mut atlas, 2, 2, &rgba).unwrap();
        assert_eq!((g.width, g.height), (2, 2));
        let row = |y: u32| {
            let o = ((g.atlas_y + y) * atlas.size + g.atlas_x) as usize;
            atlas.data[o..o + 2].to_vec()
        };
        assert_eq!(row(0), [200, 100]);
        assert_eq!(row(1), [50, 25]);
    }
}
