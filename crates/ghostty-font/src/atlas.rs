//! Implements a texture atlas (<https://en.wikipedia.org/wiki/Texture_atlas>).
//! Port of `font/Atlas.zig`.
//!
//! The implementation is based on "A Thousand Ways to Pack the Bin - A
//! Practical Approach to Two-Dimensional Rectangle Bin Packing" by Jukka
//! Jylänki. This specific implementation is based heavily on
//! Nicolas P. Rougier's freetype-gl project as well as Jukka's C++
//! implementation: <https://github.com/juj/RectangleBinPack>
//!
//! Limitations that are easy to fix, but I didn't need them:
//!
//!   * Written data must be packed, no support for custom strides.
//!   * Texture is always a square, no ability to set width != height. Note
//!     that regions written INTO the atlas do not have to be square, only
//!     the full atlas texture itself.

use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

/// The format of the texture data being written into the Atlas. This must be
/// uniform for all textures in the Atlas. If you have some textures with
/// different formats, you must use multiple atlases or convert the textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Format {
    /// 1 byte per pixel grayscale.
    #[default]
    Grayscale = 0,
    /// 3 bytes per pixel BGR.
    Bgr = 1,
    /// 4 bytes per pixel BGRA.
    Bgra = 2,
}

impl Format {
    pub fn depth(self) -> u8 {
        match self {
            Format::Grayscale => 1,
            Format::Bgr => 3,
            Format::Bgra => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Node {
    x: u32,
    y: u32,
    width: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Atlas cannot fit the desired region. You must enlarge the atlas.
    #[error("atlas cannot fit the desired region")]
    AtlasFull,
}

/// A region within the texture atlas. These can be acquired using the
/// "reserve" function. A region reservation is required to write data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Number of nodes to preallocate in the list on init.
const NODE_PREALLOC: usize = 64;

#[derive(Debug)]
pub struct Atlas {
    /// Data is the raw texture data.
    pub data: Vec<u8>,

    /// Width and height of the atlas texture. The current implementation is
    /// always square so this is both the width and the height.
    pub size: u32,

    /// The nodes (rectangles) of available space.
    nodes: Vec<Node>,

    /// The format of the texture data being written into the Atlas.
    pub format: Format,

    /// This will be incremented every time the atlas is modified. This is useful
    /// for knowing if the texture data has changed since the last time it was
    /// sent to the GPU.
    pub modified: AtomicUsize,

    /// This will be incremented every time the atlas is resized. This is useful
    /// for knowing if a GPU texture can be updated in-place or if it requires
    /// a resize operation.
    pub resized: AtomicUsize,
}

impl Atlas {
    pub fn new(size: u32, format: Format) -> Atlas {
        let mut result = Atlas {
            data: vec![0; size as usize * size as usize * format.depth() as usize],
            size,
            nodes: Vec::with_capacity(NODE_PREALLOC),
            format,
            modified: AtomicUsize::new(0),
            resized: AtomicUsize::new(0),
        };

        // This sets up our initial state
        result.clear();
        result
    }

    /// Reserve a region within the atlas with the given width and height.
    ///
    /// This will not automatically enlarge the texture if it is full.
    pub fn reserve(&mut self, width: u32, height: u32) -> Result<Region, Error> {
        // x, y are populated within the search below
        let mut region = Region {
            x: 0,
            y: 0,
            width,
            height,
        };

        // If our width/height are 0, then we return the region as-is. This
        // may seem like an error case but it simplifies downstream callers who
        // might be trying to write empty data.
        if width == 0 && height == 0 {
            return Ok(region);
        }

        // Find the location in our nodes list to insert the new node for this region.
        let best_idx: usize = {
            let mut best_height = u32::MAX;
            let mut best_width = best_height;
            let mut chosen: Option<usize> = None;

            for i in 0..self.nodes.len() {
                // Check if our region fits within this node.
                let Some(y) = self.fit(i, width, height) else {
                    continue;
                };

                let node = self.nodes[i];
                if (y + height) < best_height
                    || ((y + height) == best_height && (node.width > 0 && node.width < best_width))
                {
                    chosen = Some(i);
                    best_width = node.width;
                    best_height = y + height;
                    region.x = node.x;
                    region.y = y;
                }
            }

            // If we never found a chosen index, the atlas cannot fit our region.
            chosen.ok_or(Error::AtlasFull)?
        };

        // Insert our new node for this rectangle at the exact best index
        self.nodes.insert(
            best_idx,
            Node {
                x: region.x,
                y: region.y + height,
                width,
            },
        );

        // Optimize our rectangles
        let i = best_idx + 1;
        while i < self.nodes.len() {
            let prev = self.nodes[i - 1];
            let node = &mut self.nodes[i];
            if node.x < (prev.x + prev.width) {
                let shrink = prev.x + prev.width - node.x;
                node.x += shrink;
                node.width = node.width.saturating_sub(shrink);
                if node.width == 0 {
                    self.nodes.remove(i);
                    // Zig does `i -= 1; continue;` followed by `i += 1` in the
                    // while continue-expression, i.e. re-examines index i.
                    continue;
                }
            }

            break;
        }
        self.merge();

        Ok(region)
    }

    /// Attempts to fit a rectangle of width x height into the node at idx.
    /// The return value is the y within the texture where the rectangle can be
    /// placed. The x is the same as the node.
    fn fit(&self, idx: usize, width: u32, height: u32) -> Option<u32> {
        // If the added width exceeds our texture size, it doesn't fit.
        let node = self.nodes[idx];
        if (node.x + width) > (self.size - 1) {
            return None;
        }

        // Go node by node looking for space that can fit our width.
        let mut y = node.y;
        let mut i = idx;
        let mut width_left = width;
        while width_left > 0 {
            let n = self.nodes[i];
            if n.y > y {
                y = n.y;
            }

            // If the added height exceeds our texture size, it doesn't fit.
            if (y + height) > (self.size - 1) {
                return None;
            }

            width_left = width_left.saturating_sub(n.width);
            i += 1;
        }

        Some(y)
    }

    /// Merge adjacent nodes with the same y value.
    fn merge(&mut self) {
        let mut i = 0;
        while i + 1 < self.nodes.len() {
            let next = self.nodes[i + 1];
            let node = &mut self.nodes[i];
            if node.y == next.y {
                node.width += next.width;
                self.nodes.remove(i + 1);
                continue;
            }

            i += 1;
        }
    }

    /// Set the data associated with a reserved region. The data is expected
    /// to fit exactly within the region. The data must be formatted with the
    /// proper bpp configured on init.
    pub fn set(&mut self, reg: Region, data: &[u8]) {
        assert!(reg.x < (self.size - 1));
        assert!((reg.x + reg.width) <= (self.size - 1));
        assert!(reg.y < (self.size - 1));
        assert!((reg.y + reg.height) <= (self.size - 1));

        let depth = self.format.depth() as usize;
        let size = self.size as usize;
        let row_len = reg.width as usize * depth;
        for i in 0..reg.height as usize {
            let tex_offset = ((reg.y as usize + i) * size + reg.x as usize) * depth;
            let data_offset = i * row_len;
            self.data[tex_offset..tex_offset + row_len]
                .copy_from_slice(&data[data_offset..data_offset + row_len]);
        }

        self.modified.fetch_add(1, Ordering::Relaxed);
    }

    /// Like `set` but allows specifying a width for the source data and an
    /// offset x and y, so that a section of a larger buffer may be copied
    /// in to the atlas.
    pub fn set_from_larger(
        &mut self,
        reg: Region,
        src: &[u8],
        src_width: u32,
        src_x: u32,
        src_y: u32,
    ) {
        assert!(reg.x < (self.size - 1));
        assert!((reg.x + reg.width) <= (self.size - 1));
        assert!(reg.y < (self.size - 1));
        assert!((reg.y + reg.height) <= (self.size - 1));

        let depth = self.format.depth() as usize;
        let size = self.size as usize;
        let row_len = reg.width as usize * depth;
        for i in 0..reg.height as usize {
            let tex_offset = ((reg.y as usize + i) * size + reg.x as usize) * depth;
            let src_offset = ((src_y as usize + i) * src_width as usize + src_x as usize) * depth;
            self.data[tex_offset..tex_offset + row_len]
                .copy_from_slice(&src[src_offset..src_offset + row_len]);
        }

        self.modified.fetch_add(1, Ordering::Relaxed);
    }

    /// Grow the texture to the new size, preserving all previously written data.
    pub fn grow(&mut self, size_new: u32) {
        assert!(size_new >= self.size);
        if size_new == self.size {
            return;
        }

        let data_new =
            vec![0u8; size_new as usize * size_new as usize * self.format.depth() as usize];

        // Keep track of our old data so that we can copy it.
        let data_old = std::mem::replace(&mut self.data, data_new);
        let size_old = self.size;

        // Update our size to our new one.
        self.size = size_new;

        // Copy the old data over (data is already zeroed).
        self.set(
            Region {
                x: 0, // don't bother skipping border so we can avoid strides
                y: 1, // skip the first border row
                width: size_old,
                height: size_old - 2, // skip the last border row
            },
            &data_old[size_old as usize * self.format.depth() as usize..],
        );

        // Add the new rectangle for our added righthand space.
        self.nodes.push(Node {
            x: size_old - 1,
            y: 1,
            width: size_new - size_old,
        });

        // We are both modified and resized
        self.modified.fetch_add(1, Ordering::Relaxed);
        self.resized.fetch_add(1, Ordering::Relaxed);
    }

    /// Empty the atlas. This doesn't reclaim any previously allocated memory.
    pub fn clear(&mut self) {
        self.modified.fetch_add(1, Ordering::Relaxed);
        self.data.fill(0);
        self.nodes.clear();

        // Add our initial rectangle. This is the size of the full texture
        // and is the initial rectangle we fit our regions in. We keep a 1px border
        // to avoid artifacting when sampling the texture.
        self.nodes.push(Node {
            x: 1,
            y: 1,
            width: self.size - 2,
        });
    }

    /// Dump the atlas as a PPM to a writer, for debug purposes.
    /// Only supports grayscale and bgr atlases.
    ///
    /// NOTE: BGR atlases will have the red and blue channels
    ///       swapped because PPM expects RGB.
    ///
    /// # Panics
    /// Panics for formats other than grayscale and bgr (as Zig does).
    pub fn dump<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let c = match self.format {
            Format::Grayscale => '5',
            Format::Bgr => '6',
            f @ Format::Bgra => {
                log::error!("Unsupported format for dump: {f:?}");
                panic!("Cannot dump this atlas format.");
            }
        };
        write!(writer, "P{c}\n{} {}\n255\n", self.size, self.size)?;
        writer.write_all(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fit() {
        let mut atlas = Atlas::new(34, Format::Grayscale); // +2 for 1px border

        let modified = atlas.modified.load(Ordering::Relaxed);
        atlas.reserve(32, 32).unwrap();
        assert_eq!(modified, atlas.modified.load(Ordering::Relaxed));
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(1, 1));
    }

    #[test]
    fn doesnt_fit() {
        let mut atlas = Atlas::new(32, Format::Grayscale);

        // doesn't fit due to border
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(32, 32));
    }

    #[test]
    fn fit_multiple() {
        let mut atlas = Atlas::new(32, Format::Grayscale);

        atlas.reserve(15, 30).unwrap();
        atlas.reserve(15, 30).unwrap();
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(1, 1));
    }

    #[test]
    fn writing_data() {
        let mut atlas = Atlas::new(32, Format::Grayscale);

        let reg = atlas.reserve(2, 2).unwrap();
        let old = atlas.modified.load(Ordering::Relaxed);
        atlas.set(reg, &[1, 2, 3, 4]);
        let new = atlas.modified.load(Ordering::Relaxed);
        assert!(new > old);

        // 33 because of the 1px border and so on
        assert_eq!(1, atlas.data[33]);
        assert_eq!(2, atlas.data[34]);
        assert_eq!(3, atlas.data[65]);
        assert_eq!(4, atlas.data[66]);
    }

    #[test]
    fn writing_data_from_a_larger_source() {
        let mut atlas = Atlas::new(32, Format::Grayscale);

        let reg = atlas.reserve(2, 2).unwrap();
        let old = atlas.modified.load(Ordering::Relaxed);
        #[rustfmt::skip]
        atlas.set_from_larger(reg, &[
            8, 8, 8, 8, 8,
            8, 8, 1, 2, 8,
            8, 8, 3, 4, 8,
            8, 8, 8, 8, 8,
        ], 5, 2, 1);
        let new = atlas.modified.load(Ordering::Relaxed);
        assert!(new > old);

        // 33 because of the 1px border and so on
        assert_eq!(1, atlas.data[33]);
        assert_eq!(2, atlas.data[34]);
        assert_eq!(3, atlas.data[65]);
        assert_eq!(4, atlas.data[66]);

        // None of the `8`s from the source data outside of the
        // specified region should have made it on to the atlas.
        assert!(!atlas.data.contains(&8));
    }

    #[test]
    fn grow() {
        let mut atlas = Atlas::new(4, Format::Grayscale); // +2 for 1px border

        let reg = atlas.reserve(2, 2).unwrap();
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(1, 1));

        // Write some data so we can verify that growing doesn't mess it up
        atlas.set(reg, &[1, 2, 3, 4]);
        assert_eq!(1, atlas.data[5]);
        assert_eq!(2, atlas.data[6]);
        assert_eq!(3, atlas.data[9]);
        assert_eq!(4, atlas.data[10]);

        // Expand by exactly 1 should fit our new 1x1 block.
        let old_modified = atlas.modified.load(Ordering::Relaxed);
        let old_resized = atlas.resized.load(Ordering::Relaxed);
        atlas.grow(atlas.size + 1);
        let new_modified = atlas.modified.load(Ordering::Relaxed);
        let new_resized = atlas.resized.load(Ordering::Relaxed);
        assert!(new_modified > old_modified);
        assert!(new_resized > old_resized);
        atlas.reserve(1, 1).unwrap();

        // Ensure our data is still set. Not the offsets change due to size.
        let s = atlas.size as usize;
        assert_eq!(1, atlas.data[s + 1]);
        assert_eq!(2, atlas.data[s + 2]);
        assert_eq!(3, atlas.data[s * 2 + 1]);
        assert_eq!(4, atlas.data[s * 2 + 2]);
    }

    #[test]
    fn writing_bgr_data() {
        let mut atlas = Atlas::new(32, Format::Bgr);

        // This is BGR so its 3 bpp
        let reg = atlas.reserve(1, 2).unwrap();
        atlas.set(reg, &[1, 2, 3, 4, 5, 6]);

        // 33 because of the 1px border and so on
        let depth = atlas.format.depth() as usize;
        assert_eq!(1, atlas.data[33 * depth]);
        assert_eq!(2, atlas.data[33 * depth + 1]);
        assert_eq!(3, atlas.data[33 * depth + 2]);
        assert_eq!(4, atlas.data[65 * depth]);
        assert_eq!(5, atlas.data[65 * depth + 1]);
        assert_eq!(6, atlas.data[65 * depth + 2]);
    }

    #[test]
    fn grow_bgr() {
        // Atlas is 4x4 so its a 1px border meaning we only have 2x2 available
        let mut atlas = Atlas::new(4, Format::Bgr);

        // Get our 2x2, which should be ALL our usable space
        let reg = atlas.reserve(2, 2).unwrap();
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(1, 1));

        // This is BGR so its 3 bpp
        #[rustfmt::skip]
        atlas.set(reg, &[
            10, 11, 12, // (0, 0) (x, y) from top-left
            13, 14, 15, // (1, 0)
            20, 21, 22, // (0, 1)
            23, 24, 25, // (1, 1)
        ]);

        // Our top left skips the first row (size * depth) and the first
        // column (depth) for the 1px border.
        let depth = atlas.format.depth() as usize;
        let check = |atlas: &Atlas| {
            let mut tl = (atlas.size as usize * depth) + depth;
            assert_eq!(&[10, 11, 12, 13, 14, 15, 0], &atlas.data[tl..tl + 7]); // 0 = border
            tl += atlas.size as usize * depth; // next row
            assert_eq!(&[20, 21, 22, 23, 24, 25, 0], &atlas.data[tl..tl + 7]);
        };
        check(&atlas);

        // Expand by exactly 1 should fit our new 1x1 block.
        atlas.grow(atlas.size + 1);

        // Data should be in same place accounting for the new size
        check(&atlas);

        // Should fit the new blocks around the edges
        atlas.reserve(1, 3).unwrap();
        atlas.reserve(2, 1).unwrap();
        assert_eq!(Err(Error::AtlasFull), atlas.reserve(1, 1));
    }

    #[test]
    fn reserve_empty_region() {
        let mut atlas = Atlas::new(32, Format::Grayscale);
        let r = atlas.reserve(0, 0).unwrap();
        assert_eq!(Region::default(), r);
    }

    #[test]
    fn dump_ppm_header() {
        let atlas = Atlas::new(4, Format::Grayscale);
        let mut out = Vec::new();
        atlas.dump(&mut out).unwrap();
        assert!(out.starts_with(b"P5\n4 4\n255\n"));
        assert_eq!(out.len(), 11 + 16);
    }
}
