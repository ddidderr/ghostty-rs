//! Generic image rendering state for the renderer (`renderer/image.zig`).
//!
//! This stores all images and their placements and exposes only a limited
//! public API for adding images and placements and drawing them. Image data
//! is converted to RGBA on the CPU (`prep_for_upload`, safe to do without a
//! GL context) and uploaded to the GPU on the thread that owns the context
//! (`State::upload`, called from `draw_frame`).

use std::{collections::HashMap, time::Instant};

use ghostty_terminal::{
    kitty::{
        Image as KittyImage,
        graphics_command::TransmissionFormat,
        graphics_storage::{ImageStorage, TerminalInfo},
        unicode,
    },
    page_list::PageList,
    point::Tag,
    terminal::Terminal,
};

use crate::{
    graphics_api::{
        Draw,
        GpuBuffer,
        GpuRenderPass,
        GraphicsApi,
        ImageTextureFormat,
        Primitive,
        Step,
        TextureKind,
    },
    overlay::Overlay,
    shader_types::Image as ImageVertex,
    size::CellSize,
};

/// The GPU objects needed to upload and draw images. Built per frame from
/// the graphics API.
pub struct Gpu<'a, G: GraphicsApi> {
    pub api: &'a G,
}

/// Generic image rendering state for the renderer. This stores all
/// images and their placements and exposes only a limited public API
/// for adding images and placements and drawing them.
pub struct State<G: GraphicsApi> {
    /// The full image state for the renderer that specifies what images
    /// need to be uploaded, pruned, etc.
    pub images: HashMap<Id, Entry<G>>,

    /// The placements for the Kitty image protocol.
    pub kitty_placements: Vec<Placement>,

    /// The end index (exclusive) for placements that should be
    /// drawn below the background, below the text, etc.
    pub kitty_bg_end: u32,
    pub kitty_text_end: u32,

    /// True if there are any virtual placements. This needs to be known
    /// because virtual placements need to be recalculated more often
    /// on frame builds and are generally more expensive to handle.
    pub kitty_virtual: bool,

    /// Overlays
    pub overlay_placements: Vec<Placement>,

    /// Forces the next [`State::kitty_requires_update`] to return true. Set
    /// when the GPU context was recreated and the textures must be rebuilt.
    pub force_kitty_update: bool,
}

/// An image in the map plus the time it was transmitted.
pub struct Entry<G: GraphicsApi> {
    pub image: Image<G>,
    pub transmit_time: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawPlacements {
    KittyBelowBg,
    KittyBelowText,
    KittyAboveText,
    Overlay,
}

#[derive(Debug, thiserror::Error)]
pub enum PrepImageError {
    #[error("image conversion failed")]
    ImageConversionError,
}

impl<G: GraphicsApi> Default for State<G> {
    fn default() -> Self {
        State {
            images: HashMap::new(),
            kitty_placements: Vec::new(),
            kitty_bg_end: 0,
            kitty_text_end: 0,
            kitty_virtual: false,
            overlay_placements: Vec::new(),
            force_kitty_update: false,
        }
    }
}

impl<G: GraphicsApi> State<G> {
    pub fn new() -> State<G> {
        State::default()
    }

    /// Upload any images to the GPU that need to be uploaded,
    /// and remove any images that are no longer needed on the GPU.
    ///
    /// If any uploads fail, they are ignored. The return value
    /// can be used to detect if upload was a total success (true)
    /// or not (false).
    pub fn upload(&mut self, gpu: &Gpu<'_, G>) -> bool {
        let mut success = true;
        self.images.retain(|_, entry| {
            let img = &mut entry.image;
            if img.is_unloading() {
                return false;
            }

            if img.is_pending()
                && let Err(err) = img.upload(gpu)
            {
                log::warn!("error uploading image to GPU err={err}");
                success = false;
            }
            true
        });

        success
    }

    /// Draw the given named set of placements.
    ///
    /// Any placements that have non-uploaded images are ignored. Any
    /// graphics API errors during drawing are also ignored.
    pub fn draw<'p, P: GpuRenderPass<'p, G>>(
        &self,
        gpu: &Gpu<'_, G>,
        pipeline: &G::Pipeline,
        pass: &mut P,
        placement_type: DrawPlacements,
    ) {
        let bg_end = self.kitty_bg_end as usize;
        let text_end = self.kitty_text_end as usize;
        let placements: &[Placement] = match placement_type {
            DrawPlacements::KittyBelowBg => &self.kitty_placements[..bg_end],
            DrawPlacements::KittyBelowText => &self.kitty_placements[bg_end..text_end],
            DrawPlacements::KittyAboveText => &self.kitty_placements[text_end..],
            DrawPlacements::Overlay => &self.overlay_placements,
        };

        for p in placements {
            // Look up the image
            let Some(entry) = self.images.get(&p.image_id) else {
                log::warn!("image not found for placement image_id={:?}", p.image_id);
                continue;
            };

            // Get the texture
            let Some(texture) = entry.image.ready_texture() else {
                log::warn!("image not ready for placement image_id={:?}", p.image_id);
                continue;
            };

            // Create our vertex buffer, which is always exactly one item.
            // future(mitchellh): we can group rendering multiple instances of a single image
            let buf = match gpu.api.init_buffer_fill::<ImageVertex>(&[ImageVertex {
                grid_pos: [p.x as f32, p.y as f32],
                cell_offset: [p.cell_offset_x as f32, p.cell_offset_y as f32],
                source_rect: [
                    p.source_x as f32,
                    p.source_y as f32,
                    p.source_width as f32,
                    p.source_height as f32,
                ],
                dest_size: [p.width as f32, p.height as f32],
                pad: [0.0; 2],
            }]) {
                Ok(b) => b,
                Err(err) => {
                    log::warn!("error creating image vertex buffer err={err}");
                    continue;
                }
            };

            pass.step(&Step {
                pipeline,
                uniforms: None,
                buffers: &[Some(buf.raw())],
                textures: &[Some(texture)],
                samplers: &[],
                draw: Draw::new(Primitive::TriangleStrip, 4),
            });
        }
    }

    /// Update our overlay state. None deletes any existing overlay.
    pub fn overlay_update(&mut self, overlay: Option<&Overlay>) -> Result<(), PrepImageError> {
        let Some(overlay) = overlay else {
            // If we don't have an overlay, remove any existing one.
            if let Some(data) = self.images.get_mut(&Id::Overlay) {
                data.image.mark_for_unload();
            }
            return Ok(());
        };

        // For transmit time we always just use the current time
        // and overwrite the overlay.
        let transmit_time = Instant::now();

        // Setup our image.
        let pending = overlay.pending_image();
        let (width, height) = (pending.width, pending.height);
        self.overlay_placements.clear();
        self.prep_image(Id::Overlay, transmit_time, pending)?;

        // Setup our placement
        self.overlay_placements.push(Placement {
            image_id: Id::Overlay,
            x: 0,
            y: 0,
            z: 0,
            width,
            height,
            cell_offset_x: 0,
            cell_offset_y: 0,
            source_x: 0,
            source_y: 0,
            source_width: width,
            source_height: height,
        });
        Ok(())
    }

    /// Returns true if the Kitty graphics state requires an update based
    /// on the terminal state and our internal state.
    ///
    /// This does not read/write state used by drawing.
    pub fn kitty_requires_update(&self, t: &Terminal) -> bool {
        // If the terminal kitty image state is dirty, we must update.
        if self.force_kitty_update || t.screen().kitty_images.dirty {
            return true;
        }

        // If we have any virtual references, we must also rebuild our
        // kitty state on every frame because any cell change can move
        // an image. If the virtual placements were removed, this will
        // be set to false on the next update.
        self.kitty_virtual
    }

    /// Update the Kitty graphics state from the terminal's image storage.
    ///
    /// This reads/writes state used by drawing. The caller is responsible
    /// for clearing `storage.dirty` afterwards (Zig: `defer storage.dirty =
    /// false`).
    ///
    /// Rust deviation: Zig takes the whole `*Terminal`; the Rust kitty
    /// storage lives beside the page list it tracks pins in, so the pieces
    /// are passed explicitly (`info` = `TerminalInfo::from_terminal(t)`).
    pub fn kitty_update(
        &mut self,
        storage: &ImageStorage,
        pages: &PageList,
        info: &TerminalInfo,
        cell_size: CellSize,
    ) {
        // We always clear our previous placements no matter what because
        // we rebuild them from scratch.
        self.kitty_placements.clear();
        self.kitty_virtual = false;
        self.force_kitty_update = false;

        // Go through our known images and if there are any that are no longer
        // in use then mark them to be freed.
        //
        // This never conflicts with the below because a placement can't
        // reference an image that doesn't exist.
        for (id, entry) in &mut self.images {
            match id {
                // We're only looking at Kitty images
                Id::Kitty(id) => {
                    if storage.image_by_id(*id).is_none() {
                        entry.image.mark_for_unload();
                    }
                }
                Id::Overlay => {}
            }
        }

        // The top-left and bottom-right corners of our viewport in screen
        // points. This lets us determine offsets and containment of placements.
        let (Some(top), Some(bot)) = (
            pages.get_top_left(Tag::Viewport),
            pages.get_bottom_right(Tag::Viewport),
        ) else {
            return;
        };
        let (Some(top_pt), Some(bot_pt)) = (
            pages.point_from_pin(Tag::Screen, top),
            pages.point_from_pin(Tag::Screen, bot),
        ) else {
            return;
        };
        let top_y = top_pt.coord().y;
        let bot_y = bot_pt.coord().y;

        // Go through the placements and ensure the image is
        // on the GPU or else is ready to be sent to the GPU.
        for (key, p) in &storage.placements {
            // Special logic based on location
            match p.location {
                ghostty_terminal::kitty::graphics_storage::Location::Pin(_) => {}
                ghostty_terminal::kitty::graphics_storage::Location::Virtual => {
                    // We need to mark virtual placements on our renderer so that
                    // we know to rebuild in more scenarios since cell changes can
                    // now trigger placement changes.
                    self.kitty_virtual = true;

                    // We also continue out because virtual placements are
                    // only triggered by the unicode placeholder, not by the
                    // placement itself.
                    continue;
                }
            }

            // Get the image for the placement
            let Some(image) = storage.image_by_id(key.image_id) else {
                log::warn!(
                    "missing image for placement, ignoring image_id={}",
                    key.image_id
                );
                continue;
            };

            if let Err(err) = self.prep_kitty_placement(pages, info, top_y, bot_y, image, p) {
                // For errors we log and continue. We try to place
                // other placements even if one fails.
                log::warn!("error preparing kitty placement err={err}");
            }
        }

        // If we have virtual placements then we need to scan for placeholders.
        if self.kitty_virtual {
            for virtual_p in unicode::placement_iterator(pages, top, Some(bot)) {
                if let Err(err) =
                    self.prep_kitty_virtual_placement(storage, pages, &virtual_p, cell_size)
                {
                    // For errors we log and continue. We try to place
                    // other placements even if one fails.
                    log::warn!("error preparing kitty placement err={err}");
                }
            }
        }

        // Sort the placements by their Z value.
        self.kitty_placements.sort_unstable_by(|lhs, rhs| {
            lhs.z.cmp(&rhs.z).then_with(|| {
                if lhs.image_id.z_less_than(rhs.image_id) {
                    std::cmp::Ordering::Less
                } else if rhs.image_id.z_less_than(lhs.image_id) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });

        // Find our indices. The values are sorted by z so we can
        // find the first placement out of bounds to find the limits.
        let bg_limit = i32::MIN / 2;
        let mut bg_end: Option<u32> = None;
        let mut text_end: Option<u32> = None;
        for (i, p) in self.kitty_placements.iter().enumerate() {
            if bg_end.is_none() && p.z >= bg_limit {
                bg_end = Some(i as u32);
            }
            if text_end.is_none() && p.z >= 0 {
                text_end = Some(i as u32);
            }
        }

        // If we didn't see any images with a z > the bg limit,
        // then our bg end is the end of our placement list.
        self.kitty_bg_end = bg_end.unwrap_or(self.kitty_placements.len() as u32);
        // Same idea for the image_text_end.
        self.kitty_text_end = text_end.unwrap_or(self.kitty_placements.len() as u32);
    }

    /// Get the viewport-relative position for this
    /// placement and add it to the placements list.
    fn prep_kitty_placement(
        &mut self,
        pages: &PageList,
        info: &TerminalInfo,
        top_y: u32,
        bot_y: u32,
        image: &KittyImage,
        p: &ghostty_terminal::kitty::graphics_storage::Placement,
    ) -> Result<(), PrepImageError> {
        // Get the rect for the placement. If this placement doesn't have
        // a rect then its virtual or something so skip it.
        let Some(rect) = p.rect(image, info, pages) else {
            return Ok(());
        };

        // This is expensive but necessary.
        let (Some(top_pt), Some(bot_pt)) = (
            pages.point_from_pin(Tag::Screen, rect.top_left),
            pages.point_from_pin(Tag::Screen, rect.bottom_right),
        ) else {
            return Ok(());
        };
        let img_top_y = top_pt.coord().y;
        let img_bot_y = bot_pt.coord().y;

        // If the selection isn't within our viewport then skip it.
        if img_top_y > bot_y {
            return Ok(());
        }
        if img_bot_y < top_y {
            return Ok(());
        }

        // We need to prep this image for upload if it isn't in the
        // cache OR it is in the cache but the transmit time doesn't
        // match meaning this image is different.
        self.prep_kitty_image(image)?;

        // Calculate the dimensions of our image, taking in to
        // account the rows / columns specified by the placement.
        let dest_size = p.calculated_size(image, info);

        // Calculate the source rectangle
        let source_x = image.width.min(p.source_x);
        let source_y = image.height.min(p.source_y);
        let source_width = if p.source_width > 0 {
            (image.width - source_x).min(p.source_width)
        } else {
            image.width
        };
        let source_height = if p.source_height > 0 {
            (image.height - source_y).min(p.source_height)
        } else {
            image.height
        };

        // Get the viewport-relative Y position of the placement.
        let y_pos = img_top_y as i32 - top_y as i32;

        // Accumulate the placement
        if dest_size.width > 0 && dest_size.height > 0 {
            self.kitty_placements.push(Placement {
                image_id: Id::Kitty(image.id),
                x: i32::from(rect.top_left.x),
                y: y_pos,
                z: p.z,
                width: dest_size.width,
                height: dest_size.height,
                cell_offset_x: p.x_offset,
                cell_offset_y: p.y_offset,
                source_x,
                source_y,
                source_width,
                source_height,
            });
        }
        Ok(())
    }

    fn prep_kitty_virtual_placement(
        &mut self,
        storage: &ImageStorage,
        pages: &PageList,
        p: &unicode::Placement,
        cell_size: CellSize,
    ) -> Result<(), PrepImageError> {
        let Some(image) = storage.image_by_id(p.image_id) else {
            log::warn!(
                "missing image for virtual placement, ignoring image_id={}",
                p.image_id
            );
            return Ok(());
        };

        let rp = match p.render_placement(storage, image, cell_size.width, cell_size.height) {
            Ok(rp) => rp,
            Err(err) => {
                log::warn!("error rendering virtual placement err={err}");
                return Ok(());
            }
        };

        // If our placement is zero sized then we don't do anything.
        if rp.dest_width == 0 || rp.dest_height == 0 {
            return Ok(());
        }

        // This should be impossible with virtual placements because we should
        // only ever be looking at virtual placements that are in our
        // viewport in the renderer and virtual placements only ever take
        // up one row.
        let Some(viewport) = pages.point_from_pin(Tag::Viewport, rp.top_left) else {
            return Ok(());
        };

        // Prepare the image for the GPU and store the placement.
        self.prep_kitty_image(image)?;
        self.kitty_placements.push(Placement {
            image_id: Id::Kitty(image.id),
            x: i32::from(rp.top_left.x),
            y: viewport.coord().y as i32,
            z: -1,
            width: rp.dest_width,
            height: rp.dest_height,
            cell_offset_x: rp.offset_x,
            cell_offset_y: rp.offset_y,
            source_x: rp.source_x,
            source_y: rp.source_y,
            source_width: rp.source_width,
            source_height: rp.source_height,
        });
        Ok(())
    }

    /// Prepare an image for upload to the GPU.
    pub fn prep_image(
        &mut self,
        id: Id,
        transmit_time: Instant,
        pending: Pending,
    ) -> Result<(), PrepImageError> {
        // If this image exists and its transmit time is the same we assume
        // it is the identical image so we don't need to send it to the GPU.
        if let Some(existing) = self.images.get(&id)
            && existing.transmit_time == transmit_time
        {
            return Ok(());
        }

        // Store it in the map. We own the data (the caller handed us an
        // owned copy).
        match self.images.get_mut(&id) {
            None => {
                self.images.insert(
                    id,
                    Entry {
                        image: Image::Pending(pending),
                        transmit_time,
                    },
                );
            }
            Some(entry) => entry.image.mark_for_replace(Image::Pending(pending)),
        }

        let Some(entry) = self.images.get_mut(&id) else {
            return Ok(());
        };
        // If any error happens, we unload the image and it is invalid.
        if let Err(err) = entry.image.prep_for_upload() {
            log::warn!("error preparing image for upload err={err}");
            entry.image.mark_for_unload();
            return Err(PrepImageError::ImageConversionError);
        }
        entry.transmit_time = transmit_time;
        Ok(())
    }

    /// Prepare the provided Kitty image for upload to the GPU by copying its
    /// data and setting it to the pending state.
    fn prep_kitty_image(&mut self, image: &KittyImage) -> Result<(), PrepImageError> {
        let pixel_format = match image.format {
            TransmissionFormat::Gray => PixelFormat::Gray,
            TransmissionFormat::GrayAlpha => PixelFormat::GrayAlpha,
            TransmissionFormat::Rgb => PixelFormat::Rgb,
            TransmissionFormat::Rgba => PixelFormat::Rgba,
            // should be decoded by now
            TransmissionFormat::Png => {
                log::warn!("png kitty image was not decoded id={}", image.id);
                return Err(PrepImageError::ImageConversionError);
            }
        };
        self.prep_image(
            Id::Kitty(image.id),
            image.transmit_time,
            Pending {
                width: image.width,
                height: image.height,
                pixel_format,
                data: image.data.clone(),
            },
        )
    }
}

/// Represents a single image placement on the grid.
/// A placement is a request to render an instance of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The image being rendered. This MUST be in the image map.
    pub image_id: Id,

    /// The grid x/y where this placement is located.
    pub x: i32,
    pub y: i32,
    pub z: i32,

    /// The width/height of the placed image.
    pub width: u32,
    pub height: u32,

    /// The offset in pixels from the top left of the cell.
    /// This is clamped to the size of a cell.
    pub cell_offset_x: u32,
    pub cell_offset_y: u32,

    /// The source rectangle of the placement.
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
}

/// Image identifier used to store and lookup images.
///
/// This is tagged by different image types to make it easier to
/// store different kinds of images in the same map without having
/// to worry about ID collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Id {
    /// Image sent to the terminal state via the kitty graphics protocol.
    /// The value is the ID assigned by the terminal.
    Kitty(u32),

    /// Debug overlay. This is always composited down to a single
    /// image for now. In the future we can support layers here if we want.
    Overlay,
}

impl Id {
    /// Z-ordering tie-breaker for images with the same z value.
    pub fn z_less_than(self, rhs: Id) -> bool {
        match (self, rhs) {
            // Kitty images always sort before (lower z) non-kitty images.
            (Id::Kitty(_), Id::Overlay) => true,
            (Id::Overlay, Id::Kitty(_)) => false,
            (Id::Kitty(l), Id::Kitty(r)) => l < r,
            // No sensical ordering
            (Id::Overlay, Id::Overlay) => false,
        }
    }
}

/// Pixel formats of pending image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 1 byte per pixel grayscale.
    Gray,
    /// 2 bytes per pixel grayscale + alpha.
    GrayAlpha,
    /// 3 bytes per pixel RGB.
    Rgb,
    /// 3 byte per pixel BGR.
    Bgr,
    /// 4 byte per pixel RGBA.
    Rgba,
    /// 4 byte per pixel BGRA.
    Bgra,
}

impl PixelFormat {
    /// Get bytes per pixel for this format.
    #[inline]
    pub fn bpp(self) -> usize {
        match self {
            PixelFormat::Gray => 1,
            PixelFormat::GrayAlpha => 2,
            PixelFormat::Rgb | PixelFormat::Bgr => 3,
            PixelFormat::Rgba | PixelFormat::Bgra => 4,
        }
    }
}

/// Pending image data that needs to be uploaded to the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub height: u32,
    pub width: u32,
    pub pixel_format: PixelFormat,

    /// Data is always expected to be (width * height * bpp).
    pub data: Vec<u8>,
}

impl Pending {
    /// The expected number of bytes of image data.
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize * self.pixel_format.bpp()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to RGBA in place (the wuffs swizzle step). No-op for RGBA.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn convert(&mut self) -> Result<(), ConvertError> {
        // As things stand, we currently convert all images to RGBA before
        // uploading to the GPU. This just makes things easier. In the future
        // we may want to support other formats.
        if self.pixel_format == PixelFormat::Rgba {
            return Ok(());
        }
        let n = self.width as usize * self.height as usize;
        let bpp = self.pixel_format.bpp();
        if self.data.len() < n * bpp {
            return Err(ConvertError::ShortData);
        }
        let src = &self.data[..n * bpp];
        let mut out = Vec::with_capacity(n * 4);
        match self.pixel_format {
            PixelFormat::Gray => {
                for &g in src {
                    out.extend_from_slice(&[g, g, g, 255]);
                }
            }
            PixelFormat::GrayAlpha => {
                for px in src.chunks_exact(2) {
                    out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
            }
            PixelFormat::Rgb => {
                for px in src.chunks_exact(3) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            PixelFormat::Bgr => {
                for px in src.chunks_exact(3) {
                    out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            PixelFormat::Bgra => {
                for px in src.chunks_exact(4) {
                    out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                }
            }
            PixelFormat::Rgba => unreachable!(),
        }
        self.data = out;
        self.pixel_format = PixelFormat::Rgba;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("image data is shorter than width * height * bpp")]
    ShortData,
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error(transparent)]
    Convert(#[from] ConvertError),
    /// Texture creation failed, usually a GPU memory issue.
    #[error("texture upload failed")]
    UploadFailed,
}

/// The state for a single image that is to be rendered.
pub enum Image<G: GraphicsApi> {
    /// The image data is pending upload to the GPU.
    Pending(Pending),

    /// This is the same as the pending states but there is
    /// a texture already allocated that we want to replace.
    Replace {
        texture: G::Texture,
        pending: Pending,
    },

    /// The image is uploaded and ready to be used.
    Ready(G::Texture),

    /// The image isn't uploaded yet but is scheduled to be unloaded.
    UnloadPending(Pending),
    /// The image is uploaded and is scheduled to be unloaded.
    UnloadReady(G::Texture),
    /// The image is uploaded and scheduled to be replaced
    /// with new data, but it's also scheduled to be unloaded.
    UnloadReplace {
        texture: G::Texture,
        pending: Pending,
    },
}

impl<G: GraphicsApi> Image<G> {
    /// Mark this image for unload whatever state it is in.
    pub fn mark_for_unload(&mut self) {
        let old = std::mem::replace(self, Image::Pending(Pending::empty()));
        *self = match old {
            v @ (Image::UnloadPending(_) | Image::UnloadReplace { .. } | Image::UnloadReady(_)) => {
                v
            }
            Image::Ready(t) => Image::UnloadReady(t),
            Image::Pending(p) => Image::UnloadPending(p),
            Image::Replace { texture, pending } => Image::UnloadReplace { texture, pending },
        };
    }

    /// Mark the current image to be replaced with a pending one. This will
    /// attempt to update the existing texture if we have one, otherwise it
    /// will act like a new upload.
    pub fn mark_for_replace(&mut self, img: Image<G>) {
        let Image::Pending(new_pending) = img else {
            debug_assert!(false, "mark_for_replace requires a pending image");
            return;
        };

        // Take the current state apart; any pending data we have is dropped
        // (freed), and an existing texture is reused in the replace.
        let old = std::mem::replace(self, Image::Pending(Pending::empty()));
        *self = match old {
            Image::Ready(t) | Image::UnloadReady(t) => Image::Replace {
                texture: t,
                pending: new_pending,
            },
            Image::Replace { texture, .. } | Image::UnloadReplace { texture, .. } => {
                Image::Replace {
                    texture,
                    pending: new_pending,
                }
            }
            Image::Pending(_) | Image::UnloadPending(_) => Image::Pending(new_pending),
        };
    }

    /// Returns true if this image is pending upload.
    pub fn is_pending(&self) -> bool {
        self.pending().is_some()
    }

    /// Returns true if this image has an associated texture.
    pub fn has_texture(&self) -> bool {
        self.texture().is_some()
    }

    /// Returns true if this image is marked for unload.
    pub fn is_unloading(&self) -> bool {
        matches!(
            self,
            Image::UnloadPending(_) | Image::UnloadReplace { .. } | Image::UnloadReady(_)
        )
    }

    /// The texture if the image is ready or unload_ready (usable for drawing).
    pub fn ready_texture(&self) -> Option<&G::Texture> {
        match self {
            Image::Ready(t) | Image::UnloadReady(t) => Some(t),
            _ => None,
        }
    }

    fn pending(&self) -> Option<&Pending> {
        match self {
            Image::Pending(p) | Image::UnloadPending(p) => Some(p),
            Image::Replace { pending, .. } | Image::UnloadReplace { pending, .. } => Some(pending),
            _ => None,
        }
    }

    fn pending_mut(&mut self) -> Option<&mut Pending> {
        match self {
            Image::Pending(p) | Image::UnloadPending(p) => Some(p),
            Image::Replace { pending, .. } | Image::UnloadReplace { pending, .. } => Some(pending),
            _ => None,
        }
    }

    fn texture(&self) -> Option<&G::Texture> {
        match self {
            Image::Ready(t) | Image::UnloadReady(t) => Some(t),
            Image::Replace { texture, .. } | Image::UnloadReplace { texture, .. } => Some(texture),
            _ => None,
        }
    }

    /// Prepare the pending image data for upload to the GPU.
    /// This doesn't need GPU access so is safe to call any time.
    pub fn prep_for_upload(&mut self) -> Result<(), ConvertError> {
        if let Some(p) = self.pending_mut() {
            p.convert()
        } else {
            debug_assert!(false, "prep_for_upload requires pending data");
            Ok(())
        }
    }

    /// Upload the pending image to the GPU and change the state of this
    /// image to ready.
    pub fn upload(&mut self, gpu: &Gpu<'_, G>) -> Result<(), UploadError> {
        // No error recovery is required after this call because it just
        // converts in place and is idempotent.
        self.prep_for_upload()?;

        // Get our pending info
        let Some(p) = self.pending() else {
            return Ok(());
        };

        // Create our texture
        // (`api.imageTextureOptions(.rgba, true)`)
        let texture = gpu
            .api
            .init_texture(
                TextureKind::Image {
                    format: ImageTextureFormat::Rgba,
                    srgb: true,
                },
                p.width as usize,
                p.height as usize,
                Some(&p.data),
            )
            .map_err(|_| UploadError::UploadFailed)?;

        // Uploaded. We can now clear our data and change our state.
        //
        // NOTE: For the `replace` state, this frees the old texture. We
        //       don't currently actually replace the existing texture
        //       in-place but that is an optimization we can do later.
        *self = Image::Ready(texture);
        Ok(())
    }
}

impl Pending {
    fn empty() -> Pending {
        Pending {
            height: 0,
            width: 0,
            pixel_format: PixelFormat::Rgba,
            data: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opengl::OpenGL;

    fn pending(fmt: PixelFormat, data: Vec<u8>, w: u32, h: u32) -> Pending {
        Pending {
            width: w,
            height: h,
            pixel_format: fmt,
            data,
        }
    }

    #[test]
    fn pixel_format_bpp() {
        assert_eq!(PixelFormat::Gray.bpp(), 1);
        assert_eq!(PixelFormat::GrayAlpha.bpp(), 2);
        assert_eq!(PixelFormat::Rgb.bpp(), 3);
        assert_eq!(PixelFormat::Bgr.bpp(), 3);
        assert_eq!(PixelFormat::Rgba.bpp(), 4);
        assert_eq!(PixelFormat::Bgra.bpp(), 4);
    }

    #[test]
    fn convert_to_rgba() {
        let mut p = pending(PixelFormat::Gray, vec![7], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [7, 7, 7, 255]);
        assert_eq!(p.pixel_format, PixelFormat::Rgba);

        let mut p = pending(PixelFormat::GrayAlpha, vec![7, 9], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [7, 7, 7, 9]);

        let mut p = pending(PixelFormat::Rgb, vec![1, 2, 3], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [1, 2, 3, 255]);

        let mut p = pending(PixelFormat::Bgr, vec![1, 2, 3], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [3, 2, 1, 255]);

        let mut p = pending(PixelFormat::Bgra, vec![1, 2, 3, 4], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [3, 2, 1, 4]);

        let mut p = pending(PixelFormat::Rgba, vec![1, 2, 3, 4], 1, 1);
        p.convert().unwrap();
        assert_eq!(p.data, [1, 2, 3, 4]);

        // Short data is an error rather than a panic.
        let mut p = pending(PixelFormat::Rgb, vec![1, 2], 1, 1);
        assert!(p.convert().is_err());
    }

    #[test]
    fn id_z_ordering() {
        assert!(Id::Kitty(5).z_less_than(Id::Overlay));
        assert!(!Id::Overlay.z_less_than(Id::Kitty(5)));
        assert!(Id::Kitty(1).z_less_than(Id::Kitty(2)));
        assert!(!Id::Kitty(2).z_less_than(Id::Kitty(1)));
        assert!(!Id::Overlay.z_less_than(Id::Overlay));
    }

    #[test]
    fn state_transitions_without_gpu() {
        let mut img: Image<OpenGL> = Image::Pending(pending(PixelFormat::Rgb, vec![1, 2, 3], 1, 1));
        assert!(img.is_pending());
        assert!(!img.is_unloading());
        assert!(!img.has_texture());
        img.prep_for_upload().unwrap();
        assert_eq!(img.pending().unwrap().data, [1, 2, 3, 255]);

        // Replace a pending image with another pending one.
        img.mark_for_replace(Image::Pending(pending(
            PixelFormat::Rgba,
            vec![9, 9, 9, 9],
            1,
            1,
        )));
        assert!(matches!(img, Image::Pending(_)));
        assert_eq!(img.pending().unwrap().data, [9, 9, 9, 9]);

        img.mark_for_unload();
        assert!(img.is_unloading());
        assert!(matches!(img, Image::UnloadPending(_)));
        // Idempotent.
        img.mark_for_unload();
        assert!(matches!(img, Image::UnloadPending(_)));
    }

    #[test]
    fn prep_image_same_transmit_time_is_noop() {
        let mut s: State<OpenGL> = State::new();
        let t = Instant::now();
        s.prep_image(
            Id::Kitty(1),
            t,
            pending(PixelFormat::Rgb, vec![1, 2, 3], 1, 1),
        )
        .unwrap();
        // Same transmit time: data is not replaced.
        s.prep_image(
            Id::Kitty(1),
            t,
            pending(PixelFormat::Rgb, vec![9, 9, 9], 1, 1),
        )
        .unwrap();
        assert_eq!(
            s.images[&Id::Kitty(1)].image.pending().unwrap().data,
            [1, 2, 3, 255]
        );
        // A different transmit time replaces it.
        s.prep_image(
            Id::Kitty(1),
            Instant::now() + std::time::Duration::from_millis(1),
            pending(PixelFormat::Rgb, vec![9, 9, 9], 1, 1),
        )
        .unwrap();
        assert_eq!(
            s.images[&Id::Kitty(1)].image.pending().unwrap().data,
            [9, 9, 9, 255]
        );
    }

    fn kitty_img(id: u32, w: u32, h: u32) -> KittyImage {
        KittyImage {
            id,
            width: w,
            height: h,
            format: TransmissionFormat::Rgba,
            data: vec![255; (w * h * 4) as usize],
            ..KittyImage::default()
        }
    }

    #[test]
    fn kitty_update_builds_sorted_viewport_placements() {
        use ghostty_terminal::{
            kitty::graphics_storage::{Location, Placement as KPlacement},
            point::{Coordinate, Point},
            terminal::Options,
        };

        let mut t = Terminal::new(Options::new(10, 10)).unwrap();
        t.width_px = 100;
        t.height_px = 200;

        let mut storage = ImageStorage::default();
        storage
            .add_image(&mut t.screen_mut().pages, kitty_img(1, 20, 20))
            .unwrap();
        storage
            .add_image(&mut t.screen_mut().pages, kitty_img(2, 10, 10))
            .unwrap();
        storage
            .add_image(&mut t.screen_mut().pages, kitty_img(3, 10, 10))
            .unwrap();

        // (image, z, x, y)
        for (id, z, x, y) in [
            (1u32, 5, 2u16, 3u32),
            (2, -1, 4, 1),
            (3, i32::MIN / 2 - 1, 0, 0),
        ] {
            let pages = &mut t.screen_mut().pages;
            let pin = pages.pin(Point::Active(Coordinate { x, y })).unwrap();
            let loc = Location::Pin(pages.track_pin(pin).unwrap());
            let mut p = KPlacement::new(loc);
            p.z = z;
            storage.add_placement(pages, id, 0, p).unwrap();
        }
        // A virtual placement only flags that we need to rebuild every frame.
        storage
            .add_placement(
                &mut t.screen_mut().pages,
                1,
                7,
                KPlacement::new(Location::Virtual),
            )
            .unwrap();

        let info = TerminalInfo::from_terminal(&t);
        let mut s: State<OpenGL> = State::new();
        s.kitty_update(
            &storage,
            &t.screen().pages,
            &info,
            CellSize {
                width: 10,
                height: 20,
            },
        );

        assert!(s.kitty_virtual);
        let order: Vec<_> = s
            .kitty_placements
            .iter()
            .map(|p| (p.image_id, p.z, p.x, p.y))
            .collect();
        assert_eq!(
            order,
            [
                (Id::Kitty(3), i32::MIN / 2 - 1, 0, 0),
                (Id::Kitty(2), -1, 4, 1),
                (Id::Kitty(1), 5, 2, 3),
            ]
        );
        // Below-bg is z < i32::MIN/2, below-text is z < 0, the rest is above.
        assert_eq!(s.kitty_bg_end, 1);
        assert_eq!(s.kitty_text_end, 2);
        // Native image size is used when the placement has no columns/rows.
        assert_eq!(
            (s.kitty_placements[2].width, s.kitty_placements[2].height),
            (20, 20)
        );
        // The images were converted and queued for upload.
        assert!(s.images[&Id::Kitty(1)].image.is_pending());

        // Deleting an image from the storage marks it for unload next update.
        storage.images.remove(&2);
        s.kitty_update(
            &storage,
            &t.screen().pages,
            &info,
            CellSize {
                width: 10,
                height: 20,
            },
        );
        assert!(s.images[&Id::Kitty(2)].image.is_unloading());
        assert!(
            !s.kitty_placements
                .iter()
                .any(|p| p.image_id == Id::Kitty(2))
        );
    }
}
