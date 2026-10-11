//! This file contains various logic and data for working with the
//! Kitty graphics protocol unicode placeholder, virtual placement feature
//! (`ZIG/src/terminal/kitty/graphics_unicode.zig`).

use super::graphics_image::Image;
use super::graphics_render::Placement as RenderPlacement;
use super::graphics_storage::{ImageStorage, Location, PlacementKey};
use crate::page::Cell;
use crate::page_list::{CellSubset, Direction, PageList, Pin, RowIterator};
use crate::point::Tag;
use crate::size::CellCountInt;
use crate::style::Color;

/// Codepoint for the unicode placeholder character.
pub const PLACEHOLDER: u32 = 0x10EEEE;

/// Returns an iterator that iterates over all of the virtual placements
/// in the given pin. If `limit` is provided, the iterator will stop
/// when it reaches that pin (inclusive). If `limit` is not provided,
/// the iterator will continue until the end of the page list.
pub fn placement_iterator(list: &PageList, pin: Pin, limit: Option<Pin>) -> PlacementIterator<'_> {
    let start = list
        .point_from_pin(Tag::Screen, pin)
        .expect("pin must be in the page list");
    let end = limit.map(|l| {
        list.point_from_pin(Tag::Screen, l)
            .expect("limit pin must be in the page list")
    });
    let mut row_it = list.row_iterator(Direction::RightDown, start, end);
    let row = row_it.next();
    PlacementIterator { list, row_it, row }
}

/// Iterator over unicode virtual placements.
pub struct PlacementIterator<'a> {
    list: &'a PageList,
    row_it: RowIterator<'a>,
    row: Option<Pin>,
}

impl Iterator for PlacementIterator<'_> {
    type Item = Placement;

    fn next(&mut self) -> Option<Placement> {
        while let Some(row_pin) = self.row {
            // This row flag is set on rows that have the virtual placeholder
            let (row, _) = row_pin.row_and_cell(self.list)?;
            if !row.kitty_virtual_placeholder() {
                self.row = self.row_it.next();
                continue;
            }

            // Our current run. A run is always only a single row. This
            // assumption is built-in to our logic so if we want to change
            // this later we have to redo the logic; tests should cover;
            let mut run: Option<IncompletePlacement> = None;

            // Iterate over our remaining cells and find one with a placeholder.
            let cells = row_pin.cells(self.list, CellSubset::Right)?;
            for (i, cell) in cells.iter().enumerate() {
                // "row" now points to the top-left pin of the placement.
                // We need this temporary state to build our incomplete
                // placement.
                let pin = Pin {
                    x: row_pin.x + i as CellCountInt,
                    ..row_pin
                };
                self.row = Some(pin);

                // If this cell doesn't have the placeholder, then we
                // complete the run if we have it otherwise we just move
                // on and keep searching.
                if cell.codepoint() != PLACEHOLDER {
                    if let Some(prev) = run {
                        return Some(prev.complete());
                    }
                    continue;
                }

                // If we don't have a previous run, then we save this
                // incomplete one, start a run, and move on.
                let curr = IncompletePlacement::init(self.list, pin, *cell);
                if let Some(prev) = run.as_mut() {
                    // If we can't append, then we complete the previous
                    // run and return it.
                    if !prev.append(&curr) {
                        // Note: self.row is already updated due to the
                        // row pin above. It points back at this same
                        // cell so we can continue the new placements from
                        // here.
                        return Some(prev.complete());
                    }

                    // append is mutating so if we reached this point
                    // then prev has been updated.
                } else {
                    // For appending, we need to set our initial values.
                    let mut prev = curr;
                    if prev.row.is_none() {
                        prev.row = Some(0);
                    }
                    if prev.col.is_none() {
                        prev.col = Some(0);
                    }
                    run = Some(prev);
                }
            }

            // We move to the next row no matter what
            self.row = self.row_it.next();

            // If we have a run, we complete it here.
            if let Some(prev) = run {
                return Some(prev.complete());
            }
        }

        None
    }
}

/// Errors from [`Placement::render_placement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("placement grid out of bounds")]
    PlacementGridOutOfBounds,
    #[error("placement missing placement")]
    PlacementMissingPlacement,
}

/// The grid size for a virtual placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grid {
    rows: u32,
    columns: u32,
}

#[derive(Default)]
struct PScale {
    /// The offsets are pixels from the top-left of the placement-sized
    /// image in order to center the image as necessary.
    x_offset: f64,
    y_offset: f64,

    /// The multipliers to apply to the width/height of the original
    /// image size in order to reach the placement size.
    x_scale: f64,
    y_scale: f64,
}

struct ImgScaled {
    x_offset: f64,
    y_offset: f64,
    width: f64,
    height: f64,
}

struct Source {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

struct Dest {
    x_offset: f64,
    y_offset: f64,
    width: f64,
    height: f64,
}

/// A virtual placement in the terminal. This can represent more than
/// one cell if the cells combine to be a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The top-left pin of the placement. This can be used to get the
    /// screen x/y.
    pub pin: Pin,

    /// The image ID and placement ID for this virtual placement. The
    /// image ID is encoded in the fg color (plus optional a 8-bit high
    /// value in the 3rd diacritic). The placement ID is encoded in the
    /// underline color (optionally).
    pub image_id: u32,
    pub placement_id: u32,

    /// Starting row/col index for the image itself. This is the "fragment"
    /// of the image we want to show in this placement. This is 0-indexed.
    pub col: u32,
    pub row: u32,

    /// The width/height in cells of this placement.
    pub width: u32,
    pub height: u32,
}

impl Placement {
    /// Take this virtual placement and convert it to a render placement.
    pub fn render_placement(
        &self,
        storage: &ImageStorage,
        img: &Image,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<RenderPlacement, Error> {
        // In this function, there is a variable naming convention to try
        // to make it slightly less confusing. The prefix will tell you what
        // coordinate/size space a variable lives in:
        // - img_* is for the original image
        // - p_* is for the final placement
        // - vp_* is for the virtual placement

        // Determine the grid size that this virtual placement fits into.
        let p_grid = self.grid(storage, img, cell_width, cell_height)?;

        // From here on out we switch to floating point math. These are
        // constants that we'll reference repeatedly.
        let img_width_f64 = f64::from(img.width);
        let img_height_f64 = f64::from(img.height);

        // Next we have to fit the source image into the grid size while preserving
        // aspect ratio. We will center the image horizontally/vertically if
        // necessary.
        let p_scale = {
            let p_rows_px = f64::from(p_grid.rows * cell_height);
            let p_cols_px = f64::from(p_grid.columns * cell_width);
            if img_width_f64 * p_rows_px > img_height_f64 * p_cols_px {
                // Image is wider than the grid, fit width and center height
                let x_scale = p_cols_px / img_width_f64.max(1.0);
                let y_scale = x_scale;
                let y_offset = (p_rows_px - img_height_f64 * y_scale) / 2.0;
                PScale {
                    y_offset,
                    x_scale,
                    y_scale,
                    ..PScale::default()
                }
            } else {
                // Image is taller than the grid, fit height and center width
                let y_scale = p_rows_px / img_height_f64.max(1.0);
                let x_scale = y_scale;
                let x_offset = (p_cols_px - img_width_f64 * x_scale) / 2.0;
                PScale {
                    x_offset,
                    x_scale,
                    y_scale,
                    ..PScale::default()
                }
            }
        };

        // Scale our original image according to the aspect ratio
        // and padding calculated for p_scale.
        let img_scaled = {
            let x_offset = p_scale.x_offset / p_scale.x_scale;
            let y_offset = p_scale.y_offset / p_scale.y_scale;
            let width = img_width_f64 + (x_offset * 2.0);
            let height = img_height_f64 + (y_offset * 2.0);
            ImgScaled {
                x_offset,
                y_offset,
                width,
                height,
            }
        };

        // Calculate the source rectangle for the scaled image. These
        // coordinates are in the scaled image space.
        let mut img_scale_source = {
            // Float-converted values we already have
            let vp_width = f64::from(self.width);
            let vp_height = f64::from(self.height);
            let vp_col = f64::from(self.col);
            let vp_row = f64::from(self.row);
            let p_grid_cols = f64::from(p_grid.columns);
            let p_grid_rows = f64::from(p_grid.rows);

            // Calculate the scaled source rectangle for the image, undoing
            // the aspect ratio scaling as necessary.
            let width = img_scaled.width * (vp_width / p_grid_cols);
            let height = img_scaled.height * (vp_height / p_grid_rows);
            let x = img_scaled.width * (vp_col / p_grid_cols);
            let y = img_scaled.height * (vp_row / p_grid_rows);

            Source {
                x,
                y,
                width,
                height,
            }
        };

        // The destination rectangle. The x/y is specified by offsets from
        // the top-left since that's how our RenderPlacement works.
        let p_dest = {
            let mut x_offset = 0.0f64;
            let mut y_offset = 0.0f64;
            let mut width = f64::from(self.width * cell_width);
            let mut height = f64::from(self.height * cell_height);

            if img_scale_source.y < img_scaled.y_offset {
                // If our source rect y is within the offset area, we need to
                // adjust our source rect and destination since the source texture
                // doesn't actually have the offset area blank.
                let offset = img_scaled.y_offset - img_scale_source.y;
                img_scale_source.height -= offset;
                y_offset = offset;
                height -= offset * p_scale.y_scale;
                img_scale_source.y = 0.0;

                // If our height is greater than our original height,
                // bring it back down. This addresses the case where the top
                // and bottom offsets are both used.
                if img_scale_source.height > img_height_f64 {
                    img_scale_source.height = img_height_f64;
                    height = img_height_f64 * p_scale.y_scale;
                }
            } else if img_scale_source.y + img_scale_source.height
                > img_scaled.height - img_scaled.y_offset
            {
                // if our y is in our bottom offset area, we need to shorten the
                // source to fit in the cell.
                img_scale_source.y -= img_scaled.y_offset;
                img_scale_source.height =
                    img_scaled.height - img_scaled.y_offset - img_scale_source.y;
                img_scale_source.height -= img_scaled.y_offset;
                height = img_scale_source.height * p_scale.y_scale;
            } else {
                img_scale_source.y -= img_scaled.y_offset;
            }

            if img_scale_source.x < img_scaled.x_offset {
                // If our source rect x is within the offset area, we need to
                // adjust our source rect and destination since the source texture
                // doesn't actually have the offset area blank.
                let offset = img_scaled.x_offset - img_scale_source.x;
                img_scale_source.width -= offset;
                x_offset = offset;
                width -= offset * p_scale.x_scale;
                img_scale_source.x = 0.0;

                // If our width is greater than our original width,
                // bring it back down. This addresses the case where the left
                // and right offsets are both used.
                if img_scale_source.width > img_width_f64 {
                    img_scale_source.width = img_width_f64;
                    width = img_width_f64 * p_scale.x_scale;
                }
            } else if img_scale_source.x + img_scale_source.width
                > img_scaled.width - img_scaled.x_offset
            {
                // if our x is in our right offset area, we need to shorten the
                // source to fit in the cell.
                img_scale_source.x -= img_scaled.x_offset;
                img_scale_source.width =
                    img_scaled.width - img_scaled.x_offset - img_scale_source.x;
                img_scale_source.width -= img_scaled.x_offset;
                width = img_scale_source.width * p_scale.x_scale;
            } else {
                img_scale_source.x -= img_scaled.x_offset;
            }

            // If our modified source width/height is less than zero then
            // we render nothing because it means we're rendering outside
            // of the visible image.
            if img_scale_source.width <= 0.0 || img_scale_source.height <= 0.0 {
                return Ok(RenderPlacement::new(self.pin));
            }

            Dest {
                x_offset: x_offset * p_scale.x_scale,
                y_offset: y_offset * p_scale.y_scale,
                width,
                height,
            }
        };

        Ok(RenderPlacement {
            top_left: self.pin,
            offset_x: p_dest.x_offset.round() as u32,
            offset_y: p_dest.y_offset.round() as u32,
            source_x: img_scale_source.x.round() as u32,
            source_y: img_scale_source.y.round() as u32,
            source_width: img_scale_source.width.round() as u32,
            source_height: img_scale_source.height.round() as u32,
            dest_width: p_dest.width.round() as u32,
            dest_height: p_dest.height.round() as u32,
        })
    }

    // Calculate the grid size for the placement. For virtual placements,
    // we use the requested row/cols. If either isn't specified, we choose
    // the best size based on the image size to fit the entire image in its
    // original size.
    //
    // This part of the code does NOT do preserve any aspect ratios. Its
    // dumbly fitting the image into the grid size -- possibly user specified.
    fn grid(
        &self,
        storage: &ImageStorage,
        image: &Image,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<Grid, Error> {
        // Get the placement. If an ID is specified we look for the exact one.
        // If no ID, then we find the first virtual placement for this image.
        let placement = if self.placement_id > 0 {
            storage
                .placements
                .get(&PlacementKey::external(self.image_id, self.placement_id))
                .ok_or(Error::PlacementMissingPlacement)?
        } else {
            storage
                .placements
                .iter()
                .find(|(k, p)| k.image_id == self.image_id && p.location == Location::Virtual)
                .map(|(_, p)| p)
                .ok_or(Error::PlacementMissingPlacement)?
        };

        // Use requested rows/columns if specified
        // For unspecified rows/columns, calculate based on the image size.
        let mut rows = placement.rows;
        let mut columns = placement.columns;
        if rows == 0 {
            rows = image.height.div_ceil(cell_height);
        }
        if columns == 0 {
            columns = image.width.div_ceil(cell_width);
        }
        Ok(Grid {
            rows: CellCountInt::try_from(rows)
                .map_err(|_| Error::PlacementGridOutOfBounds)?
                .into(),
            columns: CellCountInt::try_from(columns)
                .map_err(|_| Error::PlacementGridOutOfBounds)?
                .into(),
        })
    }
}

/// IncompletePlacement is the placement information present in a single
/// cell. It is "incomplete" because the specification allows for missing
/// diacritics and so on that continue from previous valid placements.
#[derive(Debug, Clone, Copy)]
struct IncompletePlacement {
    /// The pin of the cell that created this incomplete placement.
    pin: Pin,

    /// Lower 24 bits of the image ID. This is specified in the fg color
    /// and is always required.
    image_id_low: u32,

    /// Higher 8 bits of the image ID specified using the 3rd diacritic.
    /// This is optional.
    image_id_high: Option<u8>,

    /// Placement ID is optionally specified in the underline color.
    placement_id: Option<u32>,

    /// The row/col index for the image. These are 0-indexed. These
    /// are specified using diacritics. The row is first and the col
    /// is second. Both are optional. If not specified, they can continue
    /// a previous placement under certain conditions.
    row: Option<u32>,
    col: Option<u32>,

    /// The run width so far in cells.
    width: u32,
}

impl IncompletePlacement {
    /// Parse the incomplete placement information from a row (pin at the
    /// cell) and cell.
    ///
    /// The cell could be derived from the pin but in our usage we already
    /// have the cell and we don't want to waste cycles recomputing it.
    fn init(list: &PageList, pin: Pin, cell: Cell) -> IncompletePlacement {
        debug_assert_eq!(cell.codepoint(), PLACEHOLDER);
        let style = pin.style(list).unwrap_or_default();

        let mut result = IncompletePlacement {
            pin,
            image_id_low: color_to_id(style.fg_color),
            placement_id: {
                let id = color_to_id(style.underline_color);
                if id != 0 { Some(id) } else { None }
            },
            image_id_high: None,
            row: None,
            col: None,
            width: 1,
        };

        // Try to decode all our diacritics. Any invalid diacritics are
        // treated as if they don't exist. This isn't explicitly specified
        // at the time of writing this but it appears to be how Kitty behaves.
        let cps: &[u32] = pin.grapheme(list).unwrap_or(&[]);
        if !cps.is_empty() {
            result.row = get_index(cps[0]).or_else(|| {
                log::warn!(
                    "virtual placement with invalid row diacritic cp={:X}",
                    cps[0]
                );
                None
            });

            if cps.len() > 1 {
                result.col = get_index(cps[1]).or_else(|| {
                    log::warn!(
                        "virtual placement with invalid col diacritic cp={:X}",
                        cps[1]
                    );
                    None
                });

                if cps.len() > 2 {
                    let high_ = get_index(cps[2]).or_else(|| {
                        log::warn!(
                            "virtual placement with invalid high diacritic cp={:X}",
                            cps[2]
                        );
                        None
                    });

                    if let Some(high) = high_ {
                        result.image_id_high = u8::try_from(high).ok().or_else(|| {
                            log::warn!(
                                "virtual placement with invalid high diacritic cp={:X} value={}",
                                cps[2],
                                high
                            );
                            None
                        });
                    }

                    // Any additional diacritics are ignored.
                }
            }
        }

        result
    }

    /// Append this incomplete placement to an existing placement to
    /// create a run. This returns true if the placements are compatible
    /// and were combined. If this returns false, the other placement is
    /// unchanged.
    fn append(&mut self, other: &IncompletePlacement) -> bool {
        if !self.can_append(other) {
            return false;
        }
        self.width += 1;
        true
    }

    fn can_append(&self, other: &IncompletePlacement) -> bool {
        // Converted from Kitty's logic, don't @ me.
        self.image_id_low == other.image_id_low
            && self.placement_id == other.placement_id
            && (other.row.is_none() || other.row == self.row)
            && (other.col.is_none() || other.col == self.col.map(|c| c + self.width))
            && (other.image_id_high.is_none() || other.image_id_high == self.image_id_high)
    }

    /// Complete the incomplete placement to create a full placement.
    /// This creates a new placement that isn't continuous with any previous
    /// placements.
    ///
    /// The pin is the pin of the cell that created this incomplete placement.
    fn complete(&self) -> Placement {
        Placement {
            pin: self.pin,
            image_id: {
                let low = self.image_id_low;
                let high = u32::from(self.image_id_high.unwrap_or(0));
                low | (high << 24)
            },

            placement_id: self.placement_id.unwrap_or(0),
            col: self.col.unwrap_or(0),
            row: self.row.unwrap_or(0),
            width: self.width,
            height: 1,
        }
    }
}

/// Convert a style color to a Kitty image protocol ID. This works by
/// taking the 24 most significant bits of the color, which lets it work
/// for both palette and rgb-based colors.
fn color_to_id(c: Color) -> u32 {
    match c {
        Color::None => 0,
        Color::Palette(v) => u32::from(v),
        Color::Rgb(rgb) => (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b),
    }
}

/// Get the row/col index for a diacritic codepoint. These are 0-indexed.
fn get_index(cp: u32) -> Option<u32> {
    DIACRITICS.binary_search(&cp).ok().map(|i| i as u32)
}

/// These are the diacritics used with the Kitty graphics protocol
/// Unicode placement feature to specify the row/column for placement.
/// The index into the array determines the value.
///
/// This is derived from:
/// https://sw.kovidgoyal.net/kitty/_downloads/f0a0de9ec8d9ff4456206db8e0814937/rowcolumn-diacritics.txt
const DIACRITICS: &[u32] = &[
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kitty::graphics_storage::Placement as StoragePlacement;
    use crate::terminal::{Options, Terminal};
    use ghostty_vt::modes::Mode;
    use ghostty_vt::sgr::Attribute;

    fn term(cols: u16, rows: u16) -> Terminal {
        let mut t = Terminal::new(Options::new(cols, rows)).unwrap();
        t.modes.set(Mode::GraphemeCluster, true);
        t
    }

    fn top_left(t: &Terminal) -> Pin {
        t.screens
            .active()
            .pages
            .get_top_left(Tag::Viewport)
            .unwrap()
    }

    #[test]
    fn unicode_diacritic_sorted() {
        // diacritics must be sorted since we use a binary search.
        assert!(DIACRITICS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn unicode_diacritic() {
        // Some spot checks based on Kitty behavior
        assert_eq!(30, get_index(0x483).unwrap());
        assert_eq!(294, get_index(0x1d242).unwrap());
    }

    #[test]
    fn unicode_placement_none() {
        let mut t = term(5, 5);

        // Single cell
        t.print_string("hello\nworld\n1\n2").unwrap();

        // No placements
        let pin = top_left(&t);
        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_single_row_col() {
        let mut t = term(5, 5);

        // Single cell
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();

        let pin = top_left(&t);

        // Should have exactly one placement
        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_continuation_break() {
        let mut t = term(10, 5);

        // Two runs because it jumps cols
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{030E}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(1, p.width);
        }
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(2, p.col);
            assert_eq!(1, p.width);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_continuation_with_diacritics_set() {
        let mut t = term(10, 5);

        // Three cells. They'll continue even though they're explicit
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{030D}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{030E}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(3, p.width);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_continuation_with_no_col() {
        let mut t = term(10, 5);

        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(3, p.width);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_continuation_with_no_diacritics() {
        let mut t = term(10, 5);

        t.print_string("\u{10EEEE}").unwrap();
        t.print_string("\u{10EEEE}").unwrap();
        t.print_string("\u{10EEEE}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(3, p.width);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_run_ending() {
        let mut t = term(10, 5);

        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{030D}").unwrap();
        t.print_string("ABC").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(2, p.width);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_run_starting_in_the_middle() {
        let mut t = term(10, 5);

        t.print_string("ABC").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{030D}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(0, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
            assert_eq!(2, p.width);
            assert_eq!(3, p.pin.x);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_specifying_image_id_as_palette() {
        let mut t = term(5, 5);

        t.set_attribute(Attribute::Fg256(42)).unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(42, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_specifying_image_id_with_high_bits() {
        let mut t = term(5, 5);

        t.set_attribute(Attribute::Fg256(42)).unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{0305}\u{030E}")
            .unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(33554474, p.image_id);
            assert_eq!(0, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_specifying_placement_id_as_palette() {
        let mut t = term(5, 5);

        t.set_attribute(Attribute::Fg256(42)).unwrap();
        t.set_attribute(Attribute::UnderlineColor256(21)).unwrap();
        t.print_string("\u{10EEEE}\u{0305}\u{0305}").unwrap();

        let pin = top_left(&t);

        let mut it = placement_iterator(&t.screens.active().pages, pin, None);
        {
            let p = it.next().unwrap();
            assert_eq!(42, p.image_id);
            assert_eq!(21, p.placement_id);
            assert_eq!(0, p.row);
            assert_eq!(0, p.col);
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn unicode_placement_multiple_rows_with_limit() {
        let mut t = term(5, 5);
        t.print_string("\u{10EEEE}\u{0305}\u{0305}\r\n").unwrap();
        t.print_string("\u{10EEEE}\u{030D}\u{0305}\r\n").unwrap();
        t.print_string("\u{10EEEE}\u{030E}\u{0305}").unwrap();

        let pages = &t.screens.active().pages;
        let pin = top_left(&t);
        let rows: Vec<u32> = placement_iterator(pages, pin, None)
            .map(|p| p.row)
            .collect();
        assert_eq!(rows, vec![0, 1, 2]);

        // Limit is inclusive.
        let limit = pages.pin(crate::point::Point::Active(crate::point::Coordinate {
            x: 0,
            y: 1,
        }));
        let rows: Vec<u32> = placement_iterator(pages, pin, limit)
            .map(|p| p.row)
            .collect();
        assert_eq!(rows, vec![0, 1]);
    }

    fn dog_storage(t: &mut Terminal, columns: u32, rows: u32) -> (ImageStorage, Image) {
        let mut s = ImageStorage::default();
        let image = Image {
            id: 1,
            width: 500,
            height: 306,
            ..Image::default()
        };
        let pages = &mut t.screens.active_mut().pages;
        s.add_image(pages, image.clone()).unwrap();
        let mut p = StoragePlacement::new(Location::Virtual);
        p.columns = columns;
        p.rows = rows;
        s.add_placement(pages, 1, 0, p).unwrap();
        (s, image)
    }

    fn vp(t: &Terminal, row: u32, width: u32) -> Placement {
        Placement {
            pin: t.screens.active().cursor_pin(),
            image_id: 1,
            placement_id: 0,
            col: 0,
            row,
            width,
            height: 1,
        }
    }

    // Fish:
    // printf "\033_Gf=100,i=1,t=f,q=2;$(printf dog.png | base64)\033\\"
    // printf "\e[38;5;1m\U10EEEE\U0305\U0305\U10EEEE\U0305\U030D\U10EEEE\U0305\U030E\U10EEEE\U0305\U0310\e[39m\n"
    // printf "\e[38;5;1m\U10EEEE\U030D\U0305\U10EEEE\U030D\U030D\U10EEEE\U030D\U030E\U10EEEE\U030D\U0310\e[39m\n"
    // printf "\033_Ga=p,i=1,U=1,q=2,c=4,r=2\033\\"
    #[test]
    fn unicode_render_placement_dog_4x2() {
        let (cell_width, cell_height) = (36, 80);
        let mut t = Terminal::new(Options::new(100, 100)).unwrap();
        let (s, image) = dog_storage(&mut t, 4, 2);

        // Row 1
        {
            let p = vp(&t, 0, 4);
            let rp = p
                .render_placement(&s, &image, cell_width, cell_height)
                .unwrap();
            assert_eq!(0, rp.offset_x);
            assert_eq!(36, rp.offset_y);
            assert_eq!(0, rp.source_x);
            assert_eq!(0, rp.source_y);
            assert_eq!(500, rp.source_width);
            assert_eq!(153, rp.source_height);
            assert_eq!(144, rp.dest_width);
            assert_eq!(44, rp.dest_height);
        }
        // Row 2
        {
            let p = vp(&t, 1, 4);
            let rp = p
                .render_placement(&s, &image, cell_width, cell_height)
                .unwrap();
            assert_eq!(0, rp.offset_x);
            assert_eq!(0, rp.offset_y);
            assert_eq!(0, rp.source_x);
            assert_eq!(153, rp.source_y);
            assert_eq!(500, rp.source_width);
            assert_eq!(153, rp.source_height);
            assert_eq!(144, rp.dest_width);
            assert_eq!(44, rp.dest_height);
        }
    }

    // printf "\033_Ga=p,i=1,U=1,q=2,c=2,r=2\033\\"
    #[test]
    fn unicode_render_placement_dog_2x2_with_blank_cells() {
        let (cell_width, cell_height) = (36, 80);
        let mut t = Terminal::new(Options::new(100, 100)).unwrap();
        let (s, image) = dog_storage(&mut t, 2, 2);

        // Row 1
        {
            let p = vp(&t, 0, 4);
            let rp = p
                .render_placement(&s, &image, cell_width, cell_height)
                .unwrap();
            assert_eq!(0, rp.offset_x);
            assert_eq!(58, rp.offset_y);
            assert_eq!(0, rp.source_x);
            assert_eq!(0, rp.source_y);
            assert_eq!(500, rp.source_width);
            assert_eq!(153, rp.source_height);
            assert_eq!(72, rp.dest_width);
            assert_eq!(22, rp.dest_height);
        }
        // Row 2
        {
            let p = vp(&t, 1, 4);
            let rp = p
                .render_placement(&s, &image, cell_width, cell_height)
                .unwrap();
            assert_eq!(0, rp.offset_x);
            assert_eq!(0, rp.offset_y);
            assert_eq!(0, rp.source_x);
            assert_eq!(153, rp.source_y);
            assert_eq!(500, rp.source_width);
            assert_eq!(153, rp.source_height);
            assert_eq!(72, rp.dest_width);
            assert_eq!(22, rp.dest_height);
        }
    }

    // printf "\033_Ga=p,i=1,U=1,q=2,c=1,r=1\033\\"
    #[test]
    fn unicode_render_placement_dog_1x1() {
        let (cell_width, cell_height) = (36, 80);
        let mut t = Terminal::new(Options::new(100, 100)).unwrap();
        let (s, image) = dog_storage(&mut t, 1, 1);

        let p = vp(&t, 0, 4);
        let rp = p
            .render_placement(&s, &image, cell_width, cell_height)
            .unwrap();
        assert_eq!(0, rp.offset_x);
        assert_eq!(29, rp.offset_y);
        assert_eq!(0, rp.source_x);
        assert_eq!(0, rp.source_y);
        assert_eq!(500, rp.source_width);
        assert_eq!(306, rp.source_height);
        assert_eq!(36, rp.dest_width);
        assert_eq!(22, rp.dest_height);
    }

    #[test]
    fn unicode_render_placement_missing_placement() {
        let mut t = Terminal::new(Options::new(10, 10)).unwrap();
        let s = ImageStorage::default();
        let image = Image {
            id: 1,
            width: 5,
            height: 5,
            ..Image::default()
        };
        let p = vp(&t, 0, 1);
        assert_eq!(
            p.render_placement(&s, &image, 10, 10).unwrap_err(),
            Error::PlacementMissingPlacement
        );
        let _ = &mut t;
    }
}
