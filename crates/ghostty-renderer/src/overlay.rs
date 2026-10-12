//! The debug overlay that can be drawn on top of the terminal during the
//! rendering process (`renderer/Overlay.zig`).
//!
//! Zig draws with z2d on the CPU. Everything drawn here is an axis-aligned
//! rectangle (translucent fill plus a 1px border, no anti-aliasing), so the
//! Rust port rasterizes directly into a premultiplied RGBA8 buffer with
//! source-over compositing, which is what z2d does for these operations.
//!
//! Initialize the overlay, apply features with [`Overlay::apply_features`],
//! then get the resulting image with [`Overlay::pending_image`] to upload to
//! the GPU. This works in concert with [`crate::image::State`].

use ghostty_terminal::{
    page::{SemanticContent, SemanticPrompt},
    render::RenderState,
};

use crate::{
    image::{Pending, PixelFormat},
    size::{CellSize, Size},
};

/// The colors we use for overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// light blue
    Hyperlink,
    /// orange/gold
    SemanticPrompt,
    /// cyan
    SemanticInput,
}

/// A premultiplied RGBA pixel.
type Pixel = [u8; 4];

impl Color {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Color::Hyperlink => [180, 180, 255],
            Color::SemanticPrompt => [255, 200, 64],
            Color::SemanticInput => [64, 200, 255],
        }
    }

    /// The fill color for rectangles.
    pub fn rect_fill(self) -> Pixel {
        self.alpha_pixel(96)
    }

    /// The border color for rectangles.
    pub fn rect_border(self) -> Pixel {
        self.alpha_pixel(200)
    }

    fn alpha_pixel(self, alpha: u8) -> Pixel {
        let [r, g, b] = self.rgb();
        let m = |c: u8| ((u32::from(c) * u32::from(alpha) + 127) / 255) as u8;
        [m(r), m(g), m(b), alpha]
    }
}

/// The set of available features and their configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    HighlightHyperlinks,
    SemanticPrompts,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The terminal dimensions are invalid to support an overlay.
    /// Either too small or too big.
    #[error("invalid overlay dimensions")]
    InvalidDimensions,
}

/// The overlay surface plus the cell size so we can map grid coordinates to
/// pixels.
#[derive(Debug)]
pub struct Overlay {
    width: u32,
    height: u32,
    /// Premultiplied RGBA, row-major.
    buf: Vec<u8>,

    /// Cell size information so we can map grid coordinates to pixels.
    pub cell_size: CellSize,
}

impl Overlay {
    /// Initialize a new, blank overlay.
    pub fn new(sz: &Size) -> Result<Overlay, InitError> {
        // Our surface does NOT need to take into account padding because
        // we render the overlay using the image subsystem and shaders which
        // already take that into account.
        let term_size = sz.terminal();
        if term_size.width == 0
            || term_size.height == 0
            || i32::try_from(term_size.width).is_err()
            || i32::try_from(term_size.height).is_err()
        {
            return Err(InitError::InvalidDimensions);
        }
        let len = (term_size.width as usize)
            .checked_mul(term_size.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(InitError::InvalidDimensions)?;
        Ok(Overlay {
            width: term_size.width,
            height: term_size.height,
            buf: vec![0; len],
            cell_size: sz.cell,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The raw premultiplied RGBA pixels.
    pub fn data(&self) -> &[u8] {
        &self.buf
    }

    /// Returns a pending image that can be used to copy, convert, upload, etc.
    pub fn pending_image(&self) -> Pending {
        Pending {
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Rgba,
            data: self.buf.clone(),
        }
    }

    /// Clear the overlay.
    pub fn reset(&mut self) {
        self.buf.fill(0);
    }

    /// Apply the given features to this overlay. This will draw on top of
    /// any pre-existing content in the overlay.
    pub fn apply_features(&mut self, state: &RenderState, features: &[Feature]) {
        for f in features {
            match f {
                Feature::HighlightHyperlinks => self.highlight_hyperlinks(state),
                Feature::SemanticPrompts => self.highlight_semantic_prompts(state),
            }
        }
    }

    /// Add rectangles around contiguous hyperlinks in the render state.
    ///
    /// Note: this currently doesn't take into account unique hyperlink IDs
    /// because the render state doesn't contain this. This will be added
    /// later.
    fn highlight_hyperlinks(&mut self, state: &RenderState) {
        let border_color = Color::Hyperlink.rect_border();
        let fill_color = Color::Hyperlink.rect_fill();

        for (y, row) in state.row_data.iter().enumerate() {
            if !row.raw.hyperlink() {
                continue;
            }

            let raw_cells = &row.cells.raw;
            let mut x = 0;
            while x < raw_cells.len() {
                // Skip cells without hyperlinks
                if !raw_cells[x].hyperlink() {
                    x += 1;
                    continue;
                }

                // Found start of a hyperlink run
                let start_x = x;

                // Find end of contiguous hyperlink cells
                while x < raw_cells.len() && raw_cells[x].hyperlink() {
                    x += 1;
                }
                let end_x = x;

                if let Err(err) = self.highlight_grid_rect(
                    start_x,
                    y,
                    end_x - start_x,
                    1,
                    border_color,
                    fill_color,
                ) {
                    log::warn!("Error drawing hyperlink border: {err}");
                }
            }
        }
    }

    fn highlight_semantic_prompts(&mut self, state: &RenderState) {
        // Highlight the row-level semantic prompt bars. The prompts are easy
        // because they're part of the row metadata.
        {
            let prompt_border = Color::SemanticPrompt.rect_border();
            let prompt_fill = Color::SemanticPrompt.rect_fill();

            let rows = &state.row_data;
            let mut y = 0;
            while y < rows.len() {
                // If its not a semantic prompt row, skip it.
                if rows[y].raw.semantic_prompt() == SemanticPrompt::None {
                    y += 1;
                    continue;
                }

                // Find the full length of the semantic prompt row by
                // connecting all continuations.
                let start_y = y;
                y += 1;
                while y < rows.len()
                    && rows[y].raw.semantic_prompt() == SemanticPrompt::PromptContinuation
                {
                    y += 1;
                }
                let end_y = y; // Exclusive

                let bar_width = 5.min(self.cell_size.width as usize);
                if let Err(err) = self.highlight_pixel_rect(
                    0,
                    start_y,
                    bar_width,
                    end_y - start_y,
                    prompt_border,
                    prompt_fill,
                ) {
                    log::warn!("Error drawing semantic prompt bar: {err}");
                }
            }
        }

        // Highlight contiguous semantic cells within rows.
        for (y, row) in state.row_data.iter().enumerate() {
            let raw_cells = &row.cells.raw;

            let mut x = 0;
            while x < raw_cells.len() {
                let content = raw_cells[x].semantic_content();
                let start_x = x;

                // We skip output because its just the rest of the non-prompt
                // parts and it makes the overlay too noisy.
                if content == SemanticContent::Output {
                    x += 1;
                    continue;
                }

                // Find the end of this content.
                x += 1;
                while x < raw_cells.len() && raw_cells[x].semantic_content() == content {
                    x += 1;
                }

                let color = match content {
                    SemanticContent::Prompt => Color::SemanticPrompt,
                    SemanticContent::Input => Color::SemanticInput,
                    SemanticContent::Output => unreachable!(),
                };

                if let Err(err) = self.highlight_grid_rect(
                    start_x,
                    y,
                    x - start_x,
                    1,
                    color.rect_border(),
                    color.rect_fill(),
                ) {
                    log::warn!("Error drawing semantic content highlight: {err}");
                }
            }
        }
    }

    /// Creates a rectangle for highlighting a grid region. x/y/width/height
    /// are all in grid cells.
    fn highlight_grid_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        border_color: Pixel,
        fill_color: Pixel,
    ) -> Result<(), Overflow> {
        // All math below uses checked arithmetic to avoid overflows. The
        // inputs aren't trusted and the path this is in isn't hot enough
        // to warrant unsafe optimizations.
        let cw = self.cell_size.width as usize;
        let px_width = checked_i32(width.checked_mul(cw).ok_or(Overflow)?)?;
        let px_height = checked_i32(
            height
                .checked_mul(self.cell_size.height as usize)
                .ok_or(Overflow)?,
        )?;
        let start_x = checked_i32(x.checked_mul(cw).ok_or(Overflow)?)?;
        let start_y = checked_i32(
            y.checked_mul(self.cell_size.height as usize)
                .ok_or(Overflow)?,
        )?;
        self.rect(
            start_x,
            start_y,
            start_x.checked_add(px_width).ok_or(Overflow)?,
            start_y.checked_add(px_height).ok_or(Overflow)?,
            border_color,
            fill_color,
        );
        Ok(())
    }

    /// Creates a rectangle for highlighting a region. x/y are grid cells and
    /// width/height are pixels (width) and grid cells (height).
    fn highlight_pixel_rect(
        &mut self,
        x: usize,
        y: usize,
        width_px: usize,
        height: usize,
        border_color: Pixel,
        fill_color: Pixel,
    ) -> Result<(), Overflow> {
        let px_width = checked_i32(width_px)?;
        let px_height = checked_i32(
            height
                .checked_mul(self.cell_size.height as usize)
                .ok_or(Overflow)?,
        )?;
        let start_x = checked_i32(
            x.checked_mul(self.cell_size.width as usize)
                .ok_or(Overflow)?,
        )?;
        let start_y = checked_i32(
            y.checked_mul(self.cell_size.height as usize)
                .ok_or(Overflow)?,
        )?;
        self.rect(
            start_x,
            start_y,
            start_x.checked_add(px_width).ok_or(Overflow)?,
            start_y.checked_add(px_height).ok_or(Overflow)?,
            border_color,
            fill_color,
        );
        Ok(())
    }

    /// Fill the half-open rectangle `[x0, x1) x [y0, y1)` and then stroke a
    /// one pixel hairline along its outline (the closed path through the
    /// four corners, so the right/bottom edge lies on `x1`/`y1`).
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, border: Pixel, fill: Pixel) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, fill);
            }
        }
        for x in x0..=x1 {
            self.blend(x, y0, border);
            self.blend(x, y1, border);
        }
        for y in y0..=y1 {
            self.blend(x0, y, border);
            self.blend(x1, y, border);
        }
    }

    /// Source-over blend of a premultiplied pixel, clipped to the surface.
    fn blend(&mut self, x: i32, y: i32, src: Pixel) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let inv = 255 - u32::from(src[3]);
        for (d, s) in self.buf[i..i + 4].iter_mut().zip(src) {
            let out = u32::from(s) + (u32::from(*d) * inv + 127) / 255;
            *d = out.min(255) as u8;
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("overflow")]
struct Overflow;

fn checked_i32(v: usize) -> Result<i32, Overflow> {
    i32::try_from(v).map_err(|_| Overflow)
}

#[cfg(test)]
mod tests {
    use ghostty_terminal::terminal::{Options, Terminal};

    use super::*;
    use crate::size::{Padding, ScreenSize};

    fn size(w: u32, h: u32) -> Size {
        Size {
            screen: ScreenSize {
                width: w,
                height: h,
            },
            cell: CellSize {
                width: 10,
                height: 20,
            },
            padding: Padding::default(),
        }
    }

    #[test]
    fn invalid_dimensions() {
        assert!(Overlay::new(&size(0, 10)).is_err());
        assert!(Overlay::new(&size(10, 0)).is_err());
        assert!(Overlay::new(&size(40, 40)).is_ok());
    }

    #[test]
    fn colors_are_premultiplied() {
        assert_eq!(Color::Hyperlink.rect_fill()[3], 96);
        assert_eq!(Color::Hyperlink.rect_border()[3], 200);
        let [r, _, _, a] = Color::SemanticPrompt.rect_fill();
        assert_eq!(a, 96);
        assert_eq!(r, ((255u32 * 96 + 127) / 255) as u8);
    }

    #[test]
    fn grid_rect_fill_and_reset() {
        let mut o = Overlay::new(&size(100, 100)).unwrap();
        o.highlight_grid_rect(1, 1, 2, 1, [0, 0, 100, 200], [0, 0, 50, 96])
            .unwrap();
        // Interior pixel is the fill.
        let i = ((30 * 100) + 15) * 4;
        assert_eq!(&o.data()[i..i + 4], &[0, 0, 50, 96]);
        // The top-left corner pixel is the border (blended over fill).
        let i = ((20 * 100) + 10) * 4;
        assert!(o.data()[i + 3] >= 200);
        // Outside is untouched.
        assert_eq!(&o.data()[0..4], &[0, 0, 0, 0]);
        o.reset();
        assert!(o.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn highlights_semantic_prompt_rows() {
        let mut t = Terminal::new(Options::new(10, 3)).unwrap();
        {
            let mut s = t.vt_stream();
            // OSC 133 A: start of prompt.
            s.next_slice(b"\x1b]133;A\x07$ ").unwrap();
        }
        let mut state = RenderState::new();
        state.update(&mut t).unwrap();
        let mut o = Overlay::new(&size(100, 60)).unwrap();
        o.apply_features(&state, &[Feature::SemanticPrompts]);
        assert!(o.data().iter().any(|&b| b != 0));
        // And the hyperlink feature on a terminal without links draws nothing.
        let mut o = Overlay::new(&size(100, 60)).unwrap();
        o.apply_features(&state, &[Feature::HighlightHyperlinks]);
        assert!(o.data().iter().all(|&b| b == 0));
    }
}
