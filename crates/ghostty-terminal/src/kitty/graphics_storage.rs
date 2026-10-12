//! Kitty graphics image + placement storage
//! (`ZIG/src/terminal/kitty/graphics_storage.zig`).
//!
//! # Deviations from Zig
//!
//! * Zig's `ImageStorage` methods take a `*Screen`/`*Terminal`. In Rust the
//!   storage lives *inside* the `Screen` (`screen.kitty_images`), so taking
//!   `&mut Screen` would alias. The methods instead take the
//!   [`PageList`] (`&mut screen.pages`, disjoint field borrow) and, where the
//!   Zig code reads terminal state (`t.cols`, `t.width_px`, cursor, ...), a
//!   small [`TerminalInfo`] value.
//! * `add_image`, `add_placement` and `set_limit` also take the `PageList`
//!   so that placements that are evicted / replaced have their tracked pin
//!   released (the Zig code leaks those tracked pins).
//! * `Image` lookups return references instead of copies.
//! * Placements are stored in a `BTreeMap` so iteration order is
//!   deterministic.

use std::collections::{BTreeMap, HashMap};

use ghostty_vt::kitty::graphics_command::Delete;

use super::graphics_image::{Image, LoadingImage, Rect};
use crate::{
    page_list::{Overflow, PageList, TrackedPin},
    point::{Coordinate, Point},
    size::CellCountInt,
};

/// The subset of terminal state that the storage needs to read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalInfo {
    /// `terminal.cols` / `terminal.rows`.
    pub cols: CellCountInt,
    pub rows: CellCountInt,
    /// `terminal.width_px` / `terminal.height_px`.
    pub width_px: u32,
    pub height_px: u32,
    /// `terminal.screens.active.cursor.x` / `.y`.
    pub cursor_x: CellCountInt,
    pub cursor_y: CellCountInt,
}

impl TerminalInfo {
    /// Snapshot the relevant state of a terminal.
    pub fn from_terminal(t: &crate::terminal::Terminal) -> Self {
        let cursor = &t.screens.active().cursor;
        TerminalInfo {
            cols: t.cols,
            rows: t.rows,
            width_px: t.width_px,
            height_px: t.height_px,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
        }
    }
}

/// Errors from storage operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("out of memory")]
    OutOfMemory,
}

/// The tag of a placement ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlacementIdTag {
    Internal,
    External,
}

/// A placement ID with its tag (`packed struct { tag, id }` in Zig).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacementId {
    pub tag: PlacementIdTag,
    pub id: u32,
}

/// Every placement is uniquely identified by the image ID and the
/// placement ID. If an image ID isn't specified it is assumed to be 0.
/// Likewise, if a placement ID isn't specified it is assumed to be 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacementKey {
    pub image_id: u32,
    pub placement_id: PlacementId,
}

impl PlacementKey {
    pub fn external(image_id: u32, id: u32) -> Self {
        PlacementKey {
            image_id,
            placement_id: PlacementId {
                tag: PlacementIdTag::External,
                id,
            },
        }
    }

    pub fn internal(image_id: u32, id: u32) -> Self {
        PlacementKey {
            image_id,
            placement_id: PlacementId {
                tag: PlacementIdTag::Internal,
                id,
            },
        }
    }
}

/// Where a placement lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// Exactly placed on a screen pin (tracked in the screen's PageList).
    Pin(TrackedPin),

    /// Virtual placement (U=1) for unicode placeholders.
    Virtual,
}

/// The pixel size of a placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculatedSize {
    pub width: u32,
    pub height: u32,
}

/// The size of a placement in grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The location where this placement should be drawn.
    pub location: Location,

    /// Offset of the x/y from the top-left of the cell.
    pub x_offset: u32,
    pub y_offset: u32,

    /// Source rectangle for the image to pull from
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,

    /// The columns/rows this image occupies.
    pub columns: u32,
    pub rows: u32,

    /// The z-index for this placement.
    pub z: i32,
}

impl Placement {
    pub fn new(location: Location) -> Self {
        Placement {
            location,
            x_offset: 0,
            y_offset: 0,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
            columns: 0,
            rows: 0,
            z: 0,
        }
    }

    /// Release resources (the tracked pin) held by this placement.
    pub fn deinit(&self, pages: &mut PageList) {
        match self.location {
            Location::Pin(p) => pages.untrack_pin(p),
            Location::Virtual => {}
        }
    }

    /// Calculates the size of this placement's image in pixels,
    /// taking in to account the specified rows and columns.
    pub fn calculated_size(&self, image: &Image, t: &TerminalInfo) -> CalculatedSize {
        // Height / width of the image in px.
        let width = if self.source_width > 0 {
            self.source_width
        } else {
            image.width
        };
        let height = if self.source_height > 0 {
            self.source_height
        } else {
            image.height
        };

        // If we don't have any specified cols or rows then the placement
        // should be the native size of the image, and doesn't need to be
        // re-scaled.
        if self.columns == 0 && self.rows == 0 {
            return CalculatedSize { width, height };
        }

        // We calculate the size of a cell so that we can multiply
        // it by the specified cols/rows to get the correct px size.
        //
        // We assume that the width is divided evenly by the column
        // count and the height by the row count, because it should be.
        let cell_width: u32 = t.width_px / u32::from(t.cols);
        let cell_height: u32 = t.height_px / u32::from(t.rows);

        let width_f64 = f64::from(width);
        let height_f64 = f64::from(height);

        // If we have a specified cols AND rows then we calculate
        // the width and height from them directly, we don't need
        // to adjust for aspect ratio.
        if self.columns > 0 && self.rows > 0 {
            return CalculatedSize {
                width: cell_width * self.columns,
                height: cell_height * self.rows,
            };
        }

        // Either the columns or the rows were specified, but not both,
        // so we need to calculate the other one based on the aspect ratio.

        // If only the columns were specified, we determine
        // the height of the image based on the aspect ratio.
        if self.columns > 0 {
            let aspect = height_f64 / width_f64;
            let calc_width: u32 = cell_width * self.columns;
            let calc_height = (f64::from(calc_width) * aspect).round() as u32;
            return CalculatedSize {
                width: calc_width,
                height: calc_height,
            };
        }

        // Otherwise, only the rows were specified, so we
        // determine the width based on the aspect ratio.
        let aspect = width_f64 / height_f64;
        let calc_height: u32 = cell_height * self.rows;
        let calc_width = (f64::from(calc_height) * aspect).round() as u32;
        CalculatedSize {
            width: calc_width,
            height: calc_height,
        }
    }

    /// Returns the size in grid cells that this placement takes up.
    pub fn grid_size(&self, image: &Image, t: &TerminalInfo) -> GridSize {
        // If we have a specified columns and rows then this is trivial.
        if self.columns > 0 && self.rows > 0 {
            return GridSize {
                cols: self.columns,
                rows: self.rows,
            };
        }

        // Otherwise we calculate the pixel size, divide by
        // cell size, and round up to the nearest integer.
        let calc_size = self.calculated_size(image, t);
        // NOTE: `div_ceil` can only fail if the cell size is 0, in such a
        // case it seems safe to return 0 for this.
        GridSize {
            cols: div_ceil(
                calc_size.width + self.x_offset,
                t.width_px / u32::from(t.cols),
            ),
            rows: div_ceil(
                calc_size.height + self.y_offset,
                t.height_px / u32::from(t.rows),
            ),
        }
    }

    /// Returns a selection of the entire rectangle this placement
    /// occupies within the screen. This can return None if the placement
    /// doesn't have an associated rect (i.e. a virtual placement).
    pub fn rect(&self, image: &Image, t: &TerminalInfo, pages: &PageList) -> Option<Rect> {
        let grid_size = self.grid_size(image, t);
        let handle = match self.location {
            Location::Pin(p) => p,
            Location::Virtual => return None,
        };
        let pin = pages.tracked_pin(handle)?;

        // NOTE: Zig does `grid_size.rows - 1` which would underflow for a
        // zero-row grid; we saturate instead.
        let mut br = match pin.down_overflow(pages, grid_size.rows.saturating_sub(1) as usize)? {
            Overflow::Offset(v) => v,
            Overflow::Overflow { end, .. } => end,
        };
        br.x = std::cmp::min(
            // We need to sub one here because the x value is
            // one width already. So if the image is width "1"
            // then we add zero to X because X itself is width 1.
            u32::from(pin.x) + grid_size.cols.saturating_sub(1),
            u32::from(t.cols) - 1,
        ) as CellCountInt;

        Some(Rect {
            top_left: pin,
            bottom_right: br,
        })
    }
}

fn div_ceil(a: u32, b: u32) -> u32 {
    if b == 0 { 0 } else { a.div_ceil(b) }
}

/// An image storage is associated with a terminal screen (i.e. main
/// screen, alt screen) and contains all the transmitted images and
/// placements.
#[derive(Debug)]
pub struct ImageStorage {
    /// Dirty is set to true if placements or images change. This is
    /// purely informational for the renderer and doesn't affect the
    /// correctness of the program. The renderer must set this to false
    /// if it cares about this value.
    pub dirty: bool,

    /// This is the next automatically assigned image ID. We start mid-way
    /// through the u32 range to avoid collisions with buggy programs.
    /// TODO: This isn't good enough, it's perfectly legal for programs
    ///       to use IDs in the latter half of the range and collisions
    ///       are not gracefully handled.
    pub next_image_id: u32,

    /// This is the next automatically assigned placement ID. This is never
    /// user-facing so we can start at 0. This is 32-bits because we use
    /// the same space for external placement IDs. We can start at zero
    /// because any number is valid.
    pub next_internal_placement_id: u32,

    /// The set of images that are currently known.
    pub images: HashMap<u32, Image>,

    /// The set of placements for loaded images.
    pub placements: BTreeMap<PlacementKey, Placement>,

    /// Non-null if there is an in-progress loading image.
    pub loading: Option<Box<LoadingImage>>,

    /// The total bytes of image data that have been loaded and the limit.
    /// If the limit is reached, the oldest images will be evicted to make
    /// space. Unused images take priority.
    pub total_bytes: usize,
    pub total_limit: usize,
}

impl Default for ImageStorage {
    fn default() -> Self {
        ImageStorage {
            dirty: false,
            next_image_id: 2147483647,
            next_internal_placement_id: 0,
            images: HashMap::new(),
            placements: BTreeMap::new(),
            loading: None,
            total_bytes: 0,
            total_limit: 320 * 1000 * 1000, // 320MB
        }
    }
}

impl ImageStorage {
    /// Release all images/placements. Placements' tracked pins are
    /// released in `pages` (the screen's page list).
    pub fn deinit(&mut self, pages: &mut PageList) {
        self.loading = None;
        self.images.clear();
        self.clear_placements(pages);
    }

    /// Kitty image protocol is enabled if we have a non-zero limit.
    pub fn enabled(&self) -> bool {
        self.total_limit != 0
    }

    /// Sets the limit in bytes for the total amount of image data that
    /// can be loaded. If this limit is lower, this will do an eviction
    /// if necessary. If the value is zero, then Kitty image protocol will
    /// be disabled.
    pub fn set_limit(&mut self, pages: &mut PageList, limit: usize) -> Result<(), Error> {
        // Special case disabling by quickly deleting all
        if limit == 0 {
            self.deinit(pages);
            *self = ImageStorage::default();
        }

        // If we re lowering our limit, check if we need to evict.
        if limit < self.total_bytes {
            let req_bytes = self.total_bytes - limit;
            log::info!("evicting images to lower limit, evicting={req_bytes}");
            if !self.evict_image(pages, req_bytes) {
                log::warn!("failed to evict enough images for required bytes");
            }
        }

        self.total_limit = limit;
        Ok(())
    }

    /// Add an already-loaded image to the storage. This will automatically
    /// free any existing image with the same ID.
    pub fn add_image(&mut self, pages: &mut PageList, img: Image) -> Result<(), Error> {
        // If the image itself is over the limit, then error immediately
        if img.data.len() > self.total_limit {
            return Err(Error::OutOfMemory);
        }

        // If this would put us over the limit, then evict.
        let total_bytes = self.total_bytes + img.data.len();
        if total_bytes > self.total_limit {
            let req_bytes = total_bytes - self.total_limit;
            log::info!("evicting images to make space for {req_bytes} bytes");
            if !self.evict_image(pages, req_bytes) {
                log::warn!("failed to evict enough images for required bytes");
                return Err(Error::OutOfMemory);
            }
        }

        log::debug!("addImage image={:?}", img.without_data());

        // Write our new image
        let len = img.data.len();
        if let Some(old) = self.images.insert(img.id, img) {
            self.total_bytes -= old.data.len();
        }
        self.total_bytes += len;

        self.dirty = true;
        Ok(())
    }

    /// Add a placement for a given image. The caller must verify in advance
    /// the image exists.
    pub fn add_placement(
        &mut self,
        pages: &mut PageList,
        image_id: u32,
        placement_id: u32,
        p: Placement,
    ) -> Result<(), Error> {
        debug_assert!(self.images.contains_key(&image_id));
        log::debug!("placement image_id={image_id} placement_id={placement_id} placement={p:?}");

        // The important piece here is that the placement ID needs to
        // be marked internal if it is zero. This allows multiple placements
        // to be added for the same image. If it is non-zero, then it is
        // an external placement ID and we can only have one placement
        // per (image id, placement id) pair.
        let key = if placement_id == 0 {
            let id = self.next_internal_placement_id;
            self.next_internal_placement_id = id.wrapping_add(1);
            PlacementKey::internal(image_id, id)
        } else {
            PlacementKey::external(image_id, placement_id)
        };

        // (Zig leaks the old tracked pin if the key already exists.)
        if let Some(old) = self.placements.insert(key, p) {
            old.deinit(pages);
        }

        self.dirty = true;
        Ok(())
    }

    fn clear_placements(&mut self, pages: &mut PageList) {
        for p in self.placements.values() {
            p.deinit(pages);
        }
        self.placements.clear();
    }

    /// Get an image by its ID. If the image doesn't exist, None is returned.
    pub fn image_by_id(&self, image_id: u32) -> Option<&Image> {
        self.images.get(&image_id)
    }

    /// Get an image by its number. If the image doesn't exist, return None.
    pub fn image_by_number(&self, image_number: u32) -> Option<&Image> {
        let mut newest: Option<&Image> = None;
        for img in self.images.values() {
            if img.number == image_number
                && newest.is_none_or(|n| img.transmit_time > n.transmit_time)
            {
                newest = Some(img);
            }
        }
        newest
    }

    /// Remove (and release) the placement with the given key.
    fn remove_placement(&mut self, pages: &mut PageList, key: &PlacementKey) {
        if let Some(p) = self.placements.remove(key) {
            p.deinit(pages);
        }
    }

    /// Delete placements, images.
    pub fn delete(&mut self, pages: &mut PageList, t: &TerminalInfo, cmd: Delete) {
        match cmd {
            Delete::All(delete_images) => {
                let keys: Vec<PlacementKey> = self
                    .placements
                    .iter()
                    .filter(|(_, p)| matches!(p.location, Location::Pin(_)))
                    .map(|(k, _)| *k)
                    .collect();
                for key in keys {
                    self.remove_placement(pages, &key);
                    if delete_images {
                        self.delete_if_unused(key.image_id);
                    }
                }

                if delete_images {
                    let ids: Vec<u32> = self.images.keys().copied().collect();
                    for id in ids {
                        self.delete_if_unused(id);
                    }
                }

                self.dirty = true;
            }

            Delete::Id {
                delete,
                image_id,
                placement_id,
            } => self.delete_by_id(pages, image_id, placement_id, delete),

            Delete::Newest {
                delete,
                image_number,
                placement_id,
            } => {
                let Some(img_id) = self.image_by_number(image_number).map(|i| i.id) else {
                    return;
                };
                self.delete_by_id(pages, img_id, placement_id, delete);
            }

            Delete::IntersectCursor(delete_images) => {
                self.delete_intersecting(
                    pages,
                    t,
                    Point::Active(Coordinate {
                        x: t.cursor_x,
                        y: u32::from(t.cursor_y),
                    }),
                    delete_images,
                    None,
                );
            }

            Delete::IntersectCell { delete, x, y } => {
                if x == 0 || y == 0 {
                    log::warn!("delete intersect cell coords must be at least 1");
                    return;
                }
                let (Some(x), Some(y)) = (cast_cell(x - 1), cast_cell(y - 1)) else {
                    return;
                };
                self.delete_intersecting(
                    pages,
                    t,
                    Point::Active(Coordinate { x, y: u32::from(y) }),
                    delete,
                    None,
                );
            }

            Delete::IntersectCellZ { delete, x, y, z } => {
                if x == 0 || y == 0 {
                    log::warn!("delete intersect cell coords must be at least 1");
                    return;
                }
                let (Some(x), Some(y)) = (cast_cell(x - 1), cast_cell(y - 1)) else {
                    return;
                };
                self.delete_intersecting(
                    pages,
                    t,
                    Point::Active(Coordinate { x, y: u32::from(y) }),
                    delete,
                    Some(z),
                );
            }

            Delete::Column { delete, x } => {
                if x == 0 {
                    log::warn!("delete column must be greater than zero");
                    return;
                }

                let x = x - 1;
                let keys: Vec<PlacementKey> = self.placements.keys().copied().collect();
                for key in keys {
                    let Some(img) = self.images.get(&key.image_id) else {
                        continue;
                    };
                    let Some(rect) = self.placements[&key].rect(img, t, pages) else {
                        continue;
                    };
                    if u32::from(rect.top_left.x) <= x && u32::from(rect.bottom_right.x) >= x {
                        let img_id = img.id;
                        self.remove_placement(pages, &key);
                        if delete {
                            self.delete_if_unused(img_id);
                        }
                    }
                }

                // Mark dirty to force redraw
                self.dirty = true;
            }

            Delete::Row { delete, y } => {
                if y == 0 {
                    log::warn!("delete row must be greater than zero");
                    return;
                }

                // v.y is in active coords so we want to convert it to a pin
                // so we can compare by page offsets.
                let Some(y) = cast_cell(y - 1) else {
                    return;
                };
                let Some(target_pin) = pages.pin(Point::Active(Coordinate {
                    x: 0,
                    y: u32::from(y),
                })) else {
                    return;
                };

                let keys: Vec<PlacementKey> = self.placements.keys().copied().collect();
                for key in keys {
                    let Some(img) = self.images.get(&key.image_id) else {
                        continue;
                    };
                    let Some(rect) = self.placements[&key].rect(img, t, pages) else {
                        continue;
                    };

                    // We need to copy our pin to ensure we are at least at
                    // the top-left x.
                    let mut target_pin_copy = target_pin;
                    target_pin_copy.x = rect.top_left.x;
                    if target_pin_copy.is_between(pages, rect.top_left, rect.bottom_right) {
                        let img_id = img.id;
                        self.remove_placement(pages, &key);
                        if delete {
                            self.delete_if_unused(img_id);
                        }
                    }
                }

                // Mark dirty to force redraw
                self.dirty = true;
            }

            Delete::Z { delete, z } => {
                let keys: Vec<PlacementKey> = self
                    .placements
                    .iter()
                    // Virtual placeholders cannot delete by z according
                    // to the spec.
                    .filter(|(_, p)| matches!(p.location, Location::Pin(_)) && p.z == z)
                    .map(|(k, _)| *k)
                    .collect();
                for key in keys {
                    self.remove_placement(pages, &key);
                    if delete {
                        self.delete_if_unused(key.image_id);
                    }
                }

                // Mark dirty to force redraw
                self.dirty = true;
            }

            Delete::Range {
                delete,
                first,
                last,
            } => {
                if first == 0 || last == 0 {
                    log::warn!("delete range values must be greater than zero");
                    return;
                }
                if first > last {
                    log::warn!(
                        "delete range 'x' ({first}) must be less than or equal to 'y' ({last})"
                    );
                    return;
                }

                let keys: Vec<PlacementKey> = self.placements.keys().copied().collect();
                for key in keys {
                    // NOTE: this is `or` (not `and`) in the Zig source, which
                    // makes it match every placement. Preserved faithfully.
                    if key.image_id >= first || key.image_id <= last {
                        self.remove_placement(pages, &key);
                        if delete {
                            self.delete_if_unused(key.image_id);
                        }
                    }
                }

                // Mark dirty to force redraw
                self.dirty = true;
            }

            // We don't support animation frames yet so they are successfully
            // deleted!
            Delete::AnimationFrames(_) => {}
        }
    }

    fn delete_by_id(
        &mut self,
        pages: &mut PageList,
        image_id: u32,
        placement_id: u32,
        delete_unused: bool,
    ) {
        // If no placement, we delete all placements with the ID
        if placement_id == 0 {
            let keys: Vec<PlacementKey> = self
                .placements
                .keys()
                .filter(|k| k.image_id == image_id)
                .copied()
                .collect();
            for key in keys {
                self.remove_placement(pages, &key);
            }
        } else {
            self.remove_placement(pages, &PlacementKey::external(image_id, placement_id));
        }

        // If this is specified, then we also delete the image
        // if it is no longer in use.
        if delete_unused {
            self.delete_if_unused(image_id);
        }

        // Mark dirty to force redraw
        self.dirty = true;
    }

    /// Delete an image if it is unused.
    fn delete_if_unused(&mut self, image_id: u32) {
        if self.placements.keys().any(|k| k.image_id == image_id) {
            return;
        }

        // If we get here, we can delete the image.
        if let Some(img) = self.images.remove(&image_id) {
            self.total_bytes -= img.data.len();
        }
    }

    /// Deletes all placements intersecting a screen point. `filter_z`
    /// restricts deletion to placements with that z-index.
    fn delete_intersecting(
        &mut self,
        pages: &mut PageList,
        t: &TerminalInfo,
        p: Point,
        delete_unused: bool,
        filter_z: Option<i32>,
    ) {
        // Convert our target point to a pin for comparison.
        let Some(target_pin) = pages.pin(p) else {
            return;
        };

        let keys: Vec<PlacementKey> = self.placements.keys().copied().collect();
        for key in keys {
            let Some(img) = self.images.get(&key.image_id) else {
                continue;
            };
            let placement = self.placements[&key];
            let Some(rect) = placement.rect(img, t, pages) else {
                continue;
            };
            if target_pin.is_between(pages, rect.top_left, rect.bottom_right) {
                if let Some(z) = filter_z
                    && placement.z != z
                {
                    continue;
                }
                let img_id = img.id;
                self.remove_placement(pages, &key);
                if delete_unused {
                    self.delete_if_unused(img_id);
                }
            }
        }

        // Mark dirty to force redraw
        self.dirty = true;
    }

    /// Evict image to make space. This will evict the oldest image,
    /// prioritizing unused images first, as recommended by the published
    /// Kitty spec.
    ///
    /// This will evict as many images as necessary to make space for
    /// req bytes.
    fn evict_image(&mut self, pages: &mut PageList, req: usize) -> bool {
        debug_assert!(req <= self.total_limit);

        let mut candidates: Vec<Candidate> = self
            .images
            .values()
            .map(|img| Candidate {
                id: img.id,
                time: img.transmit_time,
                used: self.placements.keys().any(|k| k.image_id == img.id),
            })
            .collect();

        // Sort: unused first, then oldest first, then lowest id.
        candidates.sort_unstable_by(|lhs, rhs| {
            lhs.used
                .cmp(&rhs.used)
                .then(lhs.time.cmp(&rhs.time))
                .then(lhs.id.cmp(&rhs.id))
        });

        // They're in order of best to evict.
        let mut evicted: usize = 0;
        for c in candidates {
            // Delete all the placements for this image and the image.
            let keys: Vec<PlacementKey> = self
                .placements
                .keys()
                .filter(|k| k.image_id == c.id)
                .copied()
                .collect();
            for key in keys {
                self.remove_placement(pages, &key);
            }

            if let Some(img) = self.images.remove(&c.id) {
                log::info!("evicting image id={} bytes={}", c.id, img.data.len());

                evicted += img.data.len();
                self.total_bytes -= img.data.len();

                if evicted > req {
                    return true;
                }
            }
        }

        false
    }
}

struct Candidate {
    id: u32,
    time: std::time::Instant,
    used: bool,
}

fn cast_cell(v: u32) -> Option<CellCountInt> {
    CellCountInt::try_from(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Options, Terminal};

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(Options::new(cols, rows)).unwrap()
    }

    fn pages(t: &mut Terminal) -> &mut PageList {
        &mut t.screens.active_mut().pages
    }

    fn tracked(t: &Terminal) -> usize {
        t.screens.active().pages.count_tracked_pins()
    }

    fn track_pin(t: &mut Terminal, pt: Coordinate) -> Location {
        let pages = pages(t);
        let p = pages.pin(Point::Active(pt)).unwrap();
        Location::Pin(pages.track_pin(p).unwrap())
    }

    fn img(id: u32, w: u32, h: u32) -> Image {
        Image {
            id,
            width: w,
            height: h,
            ..Image::default()
        }
    }

    fn coord(x: u16, y: u32) -> Coordinate {
        Coordinate { x, y }
    }

    fn add_p(s: &mut ImageStorage, t: &mut Terminal, image: u32, pid: u32, c: Coordinate) {
        let loc = track_pin(t, c);
        s.add_placement(pages(t), image, pid, Placement::new(loc))
            .unwrap();
    }

    fn add_i(s: &mut ImageStorage, t: &mut Terminal, i: Image) {
        s.add_image(pages(t), i).unwrap();
    }

    fn del(s: &mut ImageStorage, t: &mut Terminal, cmd: Delete) {
        let info = TerminalInfo::from_terminal(t);
        s.delete(pages(t), &info, cmd);
    }

    #[test]
    fn storage_add_placement_with_zero_placement_id() {
        let mut t = term(100, 100);
        t.width_px = 100;
        t.height_px = 100;

        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 50, 50));
        add_i(&mut s, &mut t, img(2, 25, 25));
        add_p(&mut s, &mut t, 1, 0, coord(25, 25));
        add_p(&mut s, &mut t, 1, 0, coord(25, 25));

        assert_eq!(s.placements.len(), 2);
        assert_eq!(s.images.len(), 2);

        // verify the placement is what we expect
        assert!(s.placements.contains_key(&PlacementKey::internal(1, 0)));
        assert!(s.placements.contains_key(&PlacementKey::internal(1, 1)));
        s.deinit(pages(&mut t));
    }

    fn setup3() -> (Terminal, ImageStorage, usize) {
        let mut t = term(3, 3);
        let tr = tracked(&t);
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 0, 0));
        add_i(&mut s, &mut t, img(2, 0, 0));
        add_i(&mut s, &mut t, img(3, 0, 0));
        add_p(&mut s, &mut t, 1, 1, coord(1, 1));
        add_p(&mut s, &mut t, 2, 1, coord(1, 1));
        (t, s, tr)
    }

    #[test]
    fn storage_delete_all_placements_and_images() {
        let (mut t, mut s, tr) = setup3();
        s.dirty = false;
        del(&mut s, &mut t, Delete::All(true));
        assert!(s.dirty);
        assert_eq!(s.images.len(), 0);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_delete_all_placements_and_images_preserves_limit() {
        let mut t = term(3, 3);
        let tr = tracked(&t);
        let mut s = ImageStorage {
            total_limit: 5000,
            ..ImageStorage::default()
        };
        add_i(&mut s, &mut t, img(1, 0, 0));
        add_i(&mut s, &mut t, img(2, 0, 0));
        add_i(&mut s, &mut t, img(3, 0, 0));
        add_p(&mut s, &mut t, 1, 1, coord(1, 1));
        add_p(&mut s, &mut t, 2, 1, coord(1, 1));

        s.dirty = false;
        del(&mut s, &mut t, Delete::All(true));
        assert!(s.dirty);
        assert_eq!(s.images.len(), 0);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(s.total_limit, 5000);
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_delete_all_placements() {
        let (mut t, mut s, tr) = setup3();
        s.dirty = false;
        del(&mut s, &mut t, Delete::All(false));
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(s.images.len(), 3);
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_delete_all_placements_by_image_id() {
        let (mut t, mut s, tr) = setup3();
        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Id {
                delete: false,
                image_id: 2,
                placement_id: 0,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 3);
        assert_eq!(tr + 1, tracked(&t));
    }

    #[test]
    fn storage_delete_all_placements_by_image_id_and_unused_images() {
        let (mut t, mut s, tr) = setup3();
        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Id {
                delete: true,
                image_id: 2,
                placement_id: 0,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 2);
        assert_eq!(tr + 1, tracked(&t));
    }

    #[test]
    fn storage_delete_placement_by_specific_id() {
        let mut t = term(3, 3);
        let tr = tracked(&t);
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 0, 0));
        add_i(&mut s, &mut t, img(2, 0, 0));
        add_i(&mut s, &mut t, img(3, 0, 0));
        add_p(&mut s, &mut t, 1, 1, coord(1, 1));
        add_p(&mut s, &mut t, 1, 2, coord(1, 1));
        add_p(&mut s, &mut t, 2, 1, coord(1, 1));

        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Id {
                delete: true,
                image_id: 1,
                placement_id: 2,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 2);
        assert_eq!(s.images.len(), 3);
        assert_eq!(tr + 2, tracked(&t));
    }

    fn setup100() -> (Terminal, ImageStorage, usize) {
        let mut t = term(100, 100);
        t.width_px = 100;
        t.height_px = 100;
        let tr = tracked(&t);
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 50, 50));
        add_i(&mut s, &mut t, img(2, 25, 25));
        add_p(&mut s, &mut t, 1, 1, coord(0, 0));
        add_p(&mut s, &mut t, 1, 2, coord(25, 25));
        (t, s, tr)
    }

    #[test]
    fn storage_delete_intersecting_cursor() {
        let (mut t, mut s, tr) = setup100();
        t.screens.active_mut().cursor_absolute(12, 12);

        s.dirty = false;
        del(&mut s, &mut t, Delete::IntersectCursor(false));
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 2);
        assert_eq!(tr + 1, tracked(&t));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 2)));
    }

    #[test]
    fn storage_delete_intersecting_cursor_plus_unused() {
        let (mut t, mut s, tr) = setup100();
        t.screens.active_mut().cursor_absolute(12, 12);

        s.dirty = false;
        del(&mut s, &mut t, Delete::IntersectCursor(true));
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 2);
        assert_eq!(tr + 1, tracked(&t));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 2)));
    }

    #[test]
    fn storage_delete_intersecting_cursor_hits_multiple() {
        let (mut t, mut s, tr) = setup100();
        t.screens.active_mut().cursor_absolute(26, 26);

        s.dirty = false;
        del(&mut s, &mut t, Delete::IntersectCursor(true));
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(s.images.len(), 1);
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_delete_by_column() {
        let (mut t, mut s, tr) = setup100();
        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Column {
                delete: false,
                x: 60,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 2);
        assert_eq!(tr + 1, tracked(&t));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 1)));
    }

    #[test]
    fn storage_delete_by_column_1x1() {
        let mut t = term(100, 100);
        t.width_px = 100;
        t.height_px = 100;
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 1, 1));
        add_p(&mut s, &mut t, 1, 1, coord(0, 0));
        add_p(&mut s, &mut t, 1, 2, coord(1, 0));
        add_p(&mut s, &mut t, 1, 3, coord(2, 0));

        del(
            &mut s,
            &mut t,
            Delete::Column {
                delete: false,
                x: 2,
            },
        );
        assert_eq!(s.placements.len(), 2);
        assert_eq!(s.images.len(), 1);
        assert!(s.placements.contains_key(&PlacementKey::external(1, 1)));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 3)));
    }

    #[test]
    fn storage_delete_by_row() {
        let (mut t, mut s, tr) = setup100();
        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Row {
                delete: false,
                y: 60,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.placements.len(), 1);
        assert_eq!(s.images.len(), 2);
        assert_eq!(tr + 1, tracked(&t));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 1)));
    }

    #[test]
    fn storage_delete_by_row_1x1() {
        let mut t = term(100, 100);
        t.width_px = 100;
        t.height_px = 100;
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 1, 1));
        add_p(&mut s, &mut t, 1, 1, coord(0, 0));
        add_p(&mut s, &mut t, 1, 2, coord(0, 1));
        add_p(&mut s, &mut t, 1, 3, coord(0, 2));

        del(
            &mut s,
            &mut t,
            Delete::Row {
                delete: false,
                y: 2,
            },
        );
        assert_eq!(s.placements.len(), 2);
        assert_eq!(s.images.len(), 1);
        assert!(s.placements.contains_key(&PlacementKey::external(1, 1)));
        assert!(s.placements.contains_key(&PlacementKey::external(1, 3)));
    }

    fn range_test(delete: bool, first: u32, last: u32, images_left: usize) {
        let (mut t, mut s, tr) = setup3();
        assert_eq!(s.images.len(), 3);
        assert_eq!(s.placements.len(), 2);

        s.dirty = false;
        del(
            &mut s,
            &mut t,
            Delete::Range {
                delete,
                first,
                last,
            },
        );
        assert!(s.dirty);
        assert_eq!(s.images.len(), images_left);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_delete_images_by_range_1() {
        range_test(false, 1, 2, 3);
    }

    #[test]
    fn storage_delete_images_by_range_2() {
        range_test(true, 1, 2, 1);
    }

    #[test]
    fn storage_delete_images_by_range_3() {
        range_test(false, 1, 1, 3);
    }

    #[test]
    fn storage_delete_images_by_range_4() {
        range_test(true, 1, 1, 1);
    }

    #[test]
    fn storage_aspect_ratio_calculation_when_only_columns_or_rows_specified() {
        let mut t = term(100, 100);
        t.width_px = 1000; // 10 px per col
        t.height_px = 2000; // 20 px per row
        let info = TerminalInfo::from_terminal(&t);

        // Case 1: Only columns specified
        {
            let image = img(1, 16, 9);
            let mut placement = Placement::new(Location::Virtual);
            placement.columns = 10;
            placement.rows = 0;

            // Image is 16x9, set to a width of 10 columns, at 10px per column
            // that's 100px width. 100px * (9 / 16) = 56.25, which should round
            // to a height of 56px.
            let calc = placement.calculated_size(&image, &info);
            assert_eq!(calc.width, 100);
            assert_eq!(calc.height, 56);
        }

        // Case 2: Only rows specified
        {
            let image = img(2, 16, 9);
            let mut placement = Placement::new(Location::Virtual);
            placement.columns = 0;
            placement.rows = 5;

            // Image is 16x9, set to a height of 5 rows, at 20px per row that's
            // 100px height. 100px * (16 / 9) = 177.77..., which should round to
            // a width of 178px.
            let calc = placement.calculated_size(&image, &info);
            assert_eq!(calc.width, 178);
            assert_eq!(calc.height, 100);
        }
    }

    #[test]
    fn storage_add_placement_replaces_and_releases_pin() {
        let mut t = term(3, 3);
        let tr = tracked(&t);
        let mut s = ImageStorage::default();
        add_i(&mut s, &mut t, img(1, 0, 0));
        add_p(&mut s, &mut t, 1, 5, coord(0, 0));
        add_p(&mut s, &mut t, 1, 5, coord(1, 1));
        assert_eq!(s.placements.len(), 1);
        assert_eq!(tr + 1, tracked(&t));
        s.deinit(pages(&mut t));
        assert_eq!(tr, tracked(&t));
    }

    #[test]
    fn storage_eviction_prefers_unused_then_oldest() {
        let mut t = term(3, 3);
        let tr = tracked(&t);
        let mut s = ImageStorage {
            total_limit: 30,
            ..ImageStorage::default()
        };
        let mk = |id: u32, n: usize| Image {
            id,
            data: vec![0; n],
            ..Image::default()
        };
        add_i(&mut s, &mut t, mk(1, 10));
        add_i(&mut s, &mut t, mk(2, 10));
        add_i(&mut s, &mut t, mk(3, 10));
        // Image 1 is oldest but in use; image 2 is the oldest unused.
        add_p(&mut s, &mut t, 1, 1, coord(0, 0));
        assert_eq!(s.total_bytes, 30);

        add_i(&mut s, &mut t, mk(4, 10));
        assert!(s.images.contains_key(&1));
        assert!(!s.images.contains_key(&2));
        // Eviction continues until strictly more than the requested bytes
        // were freed (`evicted > req` in Zig), so image 3 goes too.
        assert!(!s.images.contains_key(&3));
        assert!(s.images.contains_key(&4));
        assert_eq!(s.total_bytes, 20);

        // Too large for the limit outright.
        assert_eq!(
            s.add_image(pages(&mut t), mk(5, 31)),
            Err(Error::OutOfMemory)
        );

        // Lowering the limit evicts (the used image goes last) and releases
        // the evicted placement's pin.
        s.set_limit(pages(&mut t), 5).unwrap();
        assert_eq!(s.images.len(), 0);
        assert_eq!(s.placements.len(), 0);
        assert_eq!(tr, tracked(&t));

        // Zero disables.
        s.set_limit(pages(&mut t), 0).unwrap();
        assert!(!s.enabled());
    }

    #[test]
    fn storage_image_by_number_newest() {
        let mut t = term(3, 3);
        let mut s = ImageStorage::default();
        let a = Image {
            id: 1,
            number: 7,
            ..Image::default()
        };
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = Image {
            id: 2,
            number: 7,
            ..Image::default()
        };
        add_i(&mut s, &mut t, a);
        add_i(&mut s, &mut t, b);
        assert_eq!(s.image_by_number(7).unwrap().id, 2);
        assert!(s.image_by_number(8).is_none());
    }
}
