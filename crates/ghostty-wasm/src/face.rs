//! `font/face/web_canvas.zig` `Wasm` (`face_*`) and `font/DeferredFace.zig`
//! `Wasm` (`deferred_face_*`).

use ghostty_font::{
    Glyph,
    Presentation,
    atlas::Atlas,
    deferred_face::{DeferredFace, WebCanvas},
    face::{DesiredSize, web_canvas::Face},
};

use crate::{ensure_init, host_bytes};

fn presentation_from(v: u16) -> Option<Presentation> {
    match v {
        0 => Some(Presentation::Text),
        1 => Some(Presentation::Emoji),
        _ => None,
    }
}

/// C layout of `font.Glyph`.
#[repr(C)]
pub struct GlyphC {
    pub width: u32,
    pub height: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub atlas_x: u32,
    pub atlas_y: u32,
}

impl From<Glyph> for GlyphC {
    fn from(g: Glyph) -> GlyphC {
        GlyphC {
            width: g.width,
            height: g.height,
            offset_x: g.offset_x,
            offset_y: g.offset_y,
            atlas_x: g.atlas_x,
            atlas_y: g.atlas_y,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn face_new(ptr: *const u8, len: usize, pts: u16, p: u16) -> *mut Face {
    ensure_init();
    // SAFETY: the host passes a buffer valid for `len` bytes.
    let raw = unsafe { host_bytes(ptr, len) };
    let (Ok(raw), Some(p)) = (std::str::from_utf8(raw), presentation_from(p)) else {
        return std::ptr::null_mut();
    };
    match Face::init_named(raw, DesiredSize::new(f32::from(pts)), p) {
        Ok(face) => Box::into_raw(Box::new(face)),
        Err(err) => {
            log::warn!("error creating face err={err}");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn face_free(ptr: *mut Face) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `face_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// Resulting pointer must be freed with [`glyph_free`].
#[unsafe(no_mangle)]
pub extern "C" fn face_render_glyph(
    face: *mut Face,
    atlas: *mut Atlas,
    codepoint: u32,
) -> *mut GlyphC {
    // SAFETY: live pointers from the host.
    let (face, atlas) = unsafe { (&*face, &mut *atlas) };
    match face.render_glyph(atlas, codepoint) {
        Ok(g) => Box::into_raw(Box::new(g.into())),
        Err(err) => {
            log::warn!("error rendering glyph err={err}");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn glyph_free(ptr: *mut GlyphC) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `face_render_glyph` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn face_debug_canvas(face: *mut Face) {
    // SAFETY: live pointer from `face_new`.
    if let Err(err) = unsafe { &*face }.debug_canvas() {
        log::warn!("error adding debug canvas err={err}");
    }
}

/// Extra (not in Zig): the cell metrics `cell_width, cell_height,
/// cell_baseline` of a face, written to `out[0..3]`.
#[unsafe(no_mangle)]
pub extern "C" fn face_metrics(face: *const Face, out: *mut u32) {
    // SAFETY: live pointer from `face_new`; `out` has room for 3 u32.
    let (face, out) = unsafe { (&*face, std::slice::from_raw_parts_mut(out, 3)) };
    out[0] = face.metrics.cell_width;
    out[1] = face.metrics.cell_height;
    out[2] = face.metrics.cell_baseline;
}

/// Extra (not in Zig): `Face.glyphIndex`; returns 0xFFFF_FFFF for none.
#[unsafe(no_mangle)]
pub extern "C" fn face_glyph_index(face: *const Face, cp: u32) -> u32 {
    // SAFETY: live pointer from `face_new`.
    unsafe { &*face }.glyph_index(cp).unwrap_or(u32::MAX)
}

#[unsafe(no_mangle)]
pub extern "C" fn deferred_face_new(
    ptr: *const u8,
    len: usize,
    presentation: u16,
) -> *mut DeferredFace {
    ensure_init();
    // SAFETY: the host passes a buffer valid for `len` bytes.
    let raw = unsafe { host_bytes(ptr, len) };
    let (Ok(raw), Some(p)) = (std::str::from_utf8(raw), presentation_from(presentation)) else {
        log::warn!("error creating deferred face err=InvalidArgument");
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(DeferredFace::from_web_canvas(WebCanvas {
        font_str: raw.to_owned(),
        presentation: p,
    })))
}

#[unsafe(no_mangle)]
pub extern "C" fn deferred_face_free(ptr: *mut DeferredFace) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `deferred_face_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn deferred_face_load(face: *mut DeferredFace, pts: f32) {
    // SAFETY: live pointer from `deferred_face_new`.
    let face = unsafe { &mut *face };
    // Like Zig, the loaded face is dropped (this only checks that it loads).
    if let Err(err) = face.load(DesiredSize::new(pts)) {
        log::warn!("error loading deferred face err={err}");
    }
}
