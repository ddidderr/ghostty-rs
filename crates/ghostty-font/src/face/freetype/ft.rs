//! Thin, safe-ish wrappers over the raw `freetype-sys` / `harfbuzz-sys`
//! bindings. All FFI in the FreeType face lives here so that the rest of the
//! port (`freetype.rs`) is safe Rust.
//!
//! Callers are responsible for serialising access to a face's glyph slot
//! (see `Face::ft_mutex`); nothing in this module locks.
#![allow(unsafe_code)]
// c_long is i64 on Linux but not everywhere; keep the casts portable.
#![allow(clippy::unnecessary_cast)]

use std::{
    ffi::{CStr, c_int, c_long, c_uint, c_ulong, c_void},
    ptr::{self, NonNull},
};

use freetype_sys as sys;

/// FreeType error code (0 is never used for an error).
pub type FtError = i32;

// ---------------------------------------------------------------------------
// Declarations missing from `freetype-sys`.

unsafe extern "C" {
    fn FT_Bitmap_Init(abitmap: *mut sys::FT_Bitmap);
    fn FT_Bitmap_Convert(
        library: sys::FT_Library,
        source: *const sys::FT_Bitmap,
        target: *mut sys::FT_Bitmap,
        alignment: c_int,
    ) -> sys::FT_Error;
    fn FT_Bitmap_Done(library: sys::FT_Library, bitmap: *mut sys::FT_Bitmap) -> sys::FT_Error;
}

/// `FT_LOAD_NO_SVG`
pub const FT_LOAD_NO_SVG: i32 = 1 << 24;

/// Prefix of `TT_Header` (only fields up to `Units_Per_EM` are needed).
#[repr(C)]
struct TtHeader {
    table_version: c_long,
    font_revision: c_long,
    checksum_adjust: c_long,
    magic_number: c_long,
    flags: u16,
    units_per_em: u16,
}

/// Prefix of `TT_HoriHeader` (only fields up to `Line_Gap` are needed).
#[repr(C)]
struct TtHoriHeader {
    version: c_long,
    ascender: i16,
    descender: i16,
    line_gap: i16,
}

// ---------------------------------------------------------------------------

/// An owned FT_Library.
pub struct Lib {
    handle: sys::FT_Library,
}

// SAFETY: FT_Library is only used from behind the `Library` mutex for
// face creation/destruction; FreeType libraries may be moved between threads.
unsafe impl Send for Lib {}
// SAFETY: see above; shared use is serialised by `Library::mutex`.
unsafe impl Sync for Lib {}

impl Lib {
    pub fn new() -> Result<Lib, FtError> {
        let mut handle: sys::FT_Library = ptr::null_mut();
        // SAFETY: `handle` is a valid out pointer.
        let err = unsafe { sys::FT_Init_FreeType(&raw mut handle) };
        if err != 0 {
            return Err(err);
        }
        Ok(Lib { handle })
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        // SAFETY: `handle` was returned by FT_Init_FreeType and is freed once.
        unsafe {
            sys::FT_Done_FreeType(self.handle);
        }
    }
}

/// Copy of the `FT_Size_Metrics` fields we use.
#[derive(Debug, Clone, Copy)]
pub struct SizeMetrics {
    pub x_ppem: u16,
    pub y_ppem: u16,
    pub y_scale: i64,
    pub ascender: i64,
    pub descender: i64,
    pub height: i64,
    pub max_advance: i64,
}

/// Copy of the `TT_OS2` fields we use.
#[derive(Debug, Clone, Copy)]
pub struct Os2 {
    pub version: u16,
    pub fs_selection: u16,
    pub s_typo_ascender: i16,
    pub s_typo_descender: i16,
    pub s_typo_line_gap: i16,
    pub us_win_ascent: u16,
    pub us_win_descent: u16,
    pub y_strikeout_size: i16,
    pub y_strikeout_position: i16,
    pub sx_height: i16,
    pub s_cap_height: i16,
}

/// Copy of the `TT_Postscript` fields we use.
#[derive(Debug, Clone, Copy)]
pub struct Post {
    pub underline_position: i16,
    pub underline_thickness: i16,
}

/// Copy of the `TT_HoriHeader` fields we use.
#[derive(Debug, Clone, Copy)]
pub struct Hhea {
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
}

/// A `FT_SfntName` entry (string copied).
#[derive(Debug, Clone)]
pub struct SfntName {
    pub platform_id: u16,
    pub encoding_id: u16,
    pub name_id: u16,
    pub string: Vec<u8>,
}

/// A variation axis of a multiple-master / variable font.
#[derive(Debug, Clone, Copy)]
pub struct VarAxis {
    pub tag: u32,
    pub minimum: i64,
    pub def: i64,
    pub maximum: i64,
}

/// A copy of an `FT_Bitmap` (bytes included).
#[derive(Debug, Clone, Default)]
pub struct Bitmap {
    pub buffer: Vec<u8>,
    pub rows: u32,
    pub width: u32,
    pub pitch: i32,
    pub num_grays: u16,
    pub pixel_mode: u32,
}

/// Copy of the glyph-slot fields we use.
#[derive(Debug, Clone, Copy)]
pub struct SlotInfo {
    pub format: u32,
    pub metrics_width: i64,
    pub metrics_height: i64,
    pub hori_bearing_x: i64,
    pub hori_bearing_y: i64,
    pub advance_x: i64,
    pub bitmap_left: i32,
    pub bitmap_top: i32,
    pub pixel_mode: u32,
}

pub const GLYPH_FORMAT_OUTLINE: u32 = sys::FT_GLYPH_FORMAT_OUTLINE;
pub const GLYPH_FORMAT_BITMAP: u32 = sys::FT_GLYPH_FORMAT_BITMAP;
pub const PIXEL_MODE_GRAY: u32 = sys::FT_PIXEL_MODE_GRAY;
pub const PIXEL_MODE_BGRA: u32 = sys::FT_PIXEL_MODE_BGRA;

/// Target load/render modes.
pub const TARGET_NORMAL: i32 = sys::FT_LOAD_TARGET_NORMAL;
pub const TARGET_LIGHT: i32 = sys::FT_LOAD_TARGET_LIGHT;
pub const TARGET_MONO: i32 = sys::FT_LOAD_TARGET_MONO;

/// Load flag bits.
pub const LOAD_NO_HINTING: i32 = sys::FT_LOAD_NO_HINTING;
pub const LOAD_RENDER: i32 = sys::FT_LOAD_RENDER;
pub const LOAD_FORCE_AUTOHINT: i32 = sys::FT_LOAD_FORCE_AUTOHINT;
pub const LOAD_NO_AUTOHINT: i32 = sys::FT_LOAD_NO_AUTOHINT;
pub const LOAD_COLOR: i32 = sys::FT_LOAD_COLOR;

pub const RENDER_NORMAL: c_uint = sys::FT_RENDER_MODE_NORMAL;
pub const RENDER_MONO: c_uint = sys::FT_RENDER_MODE_MONO;

pub const TT_NAME_ID_FONT_FAMILY: u16 = 1;
pub const TT_PLATFORM_MICROSOFT: u16 = 3;
pub const TT_MS_ID_UNICODE_CS: u16 = 1;

/// An owned reference to an `FT_Face` (released with `FT_Done_Face`).
pub struct FtFace {
    ptr: NonNull<sys::FT_FaceRec>,
}

// SAFETY: FT_Face is moved between threads freely; concurrent use is
// serialised by the owner (`Face::ft_mutex`).
unsafe impl Send for FtFace {}
// SAFETY: see above.
unsafe impl Sync for FtFace {}

impl Drop for FtFace {
    fn drop(&mut self) {
        // SAFETY: we own one reference to the face.
        unsafe {
            sys::FT_Done_Face(self.ptr.as_ptr());
        }
    }
}

impl FtFace {
    fn from_raw(raw: sys::FT_Face) -> Result<FtFace, FtError> {
        NonNull::new(raw)
            .map(|ptr| FtFace { ptr })
            .ok_or(sys::FT_Err_Invalid_Handle)
    }

    /// Open a face from memory. The memory must outlive the face.
    pub fn new_memory(lib: &Lib, data: &[u8], index: i32) -> Result<FtFace, FtError> {
        let mut raw: sys::FT_Face = ptr::null_mut();
        // SAFETY: `data` is valid for the call; the caller keeps it alive for
        // the lifetime of the face.
        let err = unsafe {
            sys::FT_New_Memory_Face(
                lib.handle,
                data.as_ptr(),
                data.len() as c_long,
                index as c_long,
                &raw mut raw,
            )
        };
        if err != 0 {
            return Err(err);
        }
        FtFace::from_raw(raw)
    }

    pub fn new_file(lib: &Lib, path: &CStr, index: i32) -> Result<FtFace, FtError> {
        let mut raw: sys::FT_Face = ptr::null_mut();
        // SAFETY: `path` is NUL terminated.
        let err =
            unsafe { sys::FT_New_Face(lib.handle, path.as_ptr(), index as c_long, &raw mut raw) };
        if err != 0 {
            return Err(err);
        }
        FtFace::from_raw(raw)
    }

    /// Create another owning handle to the same face (`FT_Reference_Face`).
    pub fn reference(&self) -> Result<FtFace, FtError> {
        // SAFETY: valid face.
        let err = unsafe { sys::FT_Reference_Face(self.ptr.as_ptr()) };
        if err != 0 {
            return Err(err);
        }
        Ok(FtFace { ptr: self.ptr })
    }

    pub fn as_ptr(&self) -> sys::FT_Face {
        self.ptr.as_ptr()
    }

    fn rec(&self) -> &sys::FT_FaceRec {
        // SAFETY: the pointer is valid while we hold a reference.
        unsafe { self.ptr.as_ref() }
    }

    pub fn select_unicode_charmap(&self) -> Result<(), FtError> {
        // SAFETY: valid face.
        let err = unsafe { sys::FT_Select_Charmap(self.as_ptr(), sys::FT_ENCODING_UNICODE) };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    pub fn set_char_size(
        &self,
        char_width: i32,
        char_height: i32,
        xdpi: u16,
        ydpi: u16,
    ) -> Result<(), FtError> {
        // SAFETY: valid face.
        let err = unsafe {
            sys::FT_Set_Char_Size(
                self.as_ptr(),
                char_width as c_long,
                char_height as c_long,
                c_uint::from(xdpi),
                c_uint::from(ydpi),
            )
        };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    pub fn select_size(&self, idx: i32) -> Result<(), FtError> {
        // SAFETY: valid face.
        let err = unsafe { sys::FT_Select_Size(self.as_ptr(), idx) };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    pub fn num_fixed_sizes(&self) -> i32 {
        self.rec().num_fixed_sizes
    }

    /// Width of the `i`th available fixed size.
    pub fn fixed_size_width(&self, i: i32) -> i32 {
        debug_assert!(i >= 0 && i < self.num_fixed_sizes());
        // SAFETY: `i` is within `num_fixed_sizes`.
        i32::from(unsafe { (*self.rec().available_sizes.add(i as usize)).width })
    }

    fn flags(&self) -> c_long {
        self.rec().face_flags
    }

    pub fn is_scalable(&self) -> bool {
        self.flags() & sys::FT_FACE_FLAG_SCALABLE != 0
    }

    pub fn has_color(&self) -> bool {
        self.flags() & sys::FT_FACE_FLAG_COLOR != 0
    }

    pub fn has_multiple_masters(&self) -> bool {
        self.flags() & sys::FT_FACE_FLAG_MULTIPLE_MASTERS != 0
    }

    pub fn units_per_em(&self) -> u16 {
        self.rec().units_per_EM
    }

    pub fn char_index(&self, cp: u32) -> Option<u32> {
        // SAFETY: valid face.
        let i = unsafe { sys::FT_Get_Char_Index(self.as_ptr(), c_ulong::from(cp)) };
        if i == 0 { None } else { Some(i) }
    }

    pub fn load_glyph(&self, glyph_index: u32, flags: i32) -> Result<(), FtError> {
        // SAFETY: valid face; glyph slot access is serialised by the caller.
        let err = unsafe { sys::FT_Load_Glyph(self.as_ptr(), glyph_index, flags) };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    pub fn size_metrics(&self) -> SizeMetrics {
        // SAFETY: a face always has an active size once a size has been set.
        let m = unsafe { (*self.rec().size).metrics };
        SizeMetrics {
            x_ppem: m.x_ppem,
            y_ppem: m.y_ppem,
            y_scale: m.y_scale as i64,
            ascender: m.ascender as i64,
            descender: m.descender as i64,
            height: m.height as i64,
            max_advance: m.max_advance as i64,
        }
    }

    fn sfnt_table(&self, tag: u32) -> *mut c_void {
        // SAFETY: valid face; the returned pointer is owned by the face.
        unsafe { sys::FT_Get_Sfnt_Table(self.as_ptr(), tag) }
    }

    /// `Units_Per_EM` from the `head` table.
    pub fn head_units_per_em(&self) -> Option<u16> {
        let p = self.sfnt_table(sys::FT_SFNT_HEAD).cast::<TtHeader>();
        // SAFETY: non-null pointers refer to a `TT_Header` owned by the face.
        unsafe { p.as_ref() }.map(|h| h.units_per_em)
    }

    pub fn post(&self) -> Option<Post> {
        let p = self
            .sfnt_table(sys::FT_SFNT_POST)
            .cast::<sys::TT_Postscript>();
        // SAFETY: non-null pointers refer to a `TT_Postscript` owned by the face.
        unsafe { p.as_ref() }.map(|p| Post {
            underline_position: p.underlinePosition,
            underline_thickness: p.underlineThickness,
        })
    }

    /// The OS/2 table; `None` if absent or if FreeType flags it as invalid
    /// (version 0xFFFF).
    pub fn os2(&self) -> Option<Os2> {
        let p = self.sfnt_table(sys::FT_SFNT_OS2).cast::<sys::TT_OS2>();
        // SAFETY: non-null pointers refer to a `TT_OS2` owned by the face.
        let o = unsafe { p.as_ref() }?;
        if o.version == 0xFFFF {
            return None;
        }
        Some(Os2 {
            version: o.version,
            fs_selection: o.fsSelection,
            s_typo_ascender: o.sTypoAscender,
            s_typo_descender: o.sTypoDescender,
            s_typo_line_gap: o.sTypoLineGap,
            us_win_ascent: o.usWinAscent,
            us_win_descent: o.usWinDescent,
            y_strikeout_size: o.yStrikeoutSize,
            y_strikeout_position: o.yStrikeoutPosition,
            sx_height: o.sxHeight,
            s_cap_height: o.sCapHeight,
        })
    }

    pub fn hhea(&self) -> Option<Hhea> {
        let p = self.sfnt_table(sys::FT_SFNT_HHEA).cast::<TtHoriHeader>();
        // SAFETY: non-null pointers refer to a `TT_HoriHeader` owned by the face.
        unsafe { p.as_ref() }.map(|h| Hhea {
            ascender: h.ascender,
            descender: h.descender,
            line_gap: h.line_gap,
        })
    }

    pub fn sfnt_name_count(&self) -> usize {
        // SAFETY: valid face.
        unsafe { sys::FT_Get_Sfnt_Name_Count(self.as_ptr()) as usize }
    }

    pub fn sfnt_name(&self, i: usize) -> Option<SfntName> {
        let mut name = std::mem::MaybeUninit::<sys::FT_SfntName>::uninit();
        // SAFETY: out pointer is valid for a FT_SfntName.
        let err = unsafe { sys::FT_Get_Sfnt_Name(self.as_ptr(), i as c_uint, name.as_mut_ptr()) };
        if err != 0 {
            return None;
        }
        // SAFETY: initialised by the successful call above.
        let name = unsafe { name.assume_init() };
        let string = if name.string.is_null() {
            Vec::new()
        } else {
            // SAFETY: FreeType guarantees `string_len` readable bytes.
            unsafe { std::slice::from_raw_parts(name.string, name.string_len as usize) }.to_vec()
        };
        Some(SfntName {
            platform_id: name.platform_id,
            encoding_id: name.encoding_id,
            name_id: name.name_id,
            string,
        })
    }

    /// Copy of an SFNT table. `Ok(None)` if the table has zero length.
    pub fn load_sfnt_table(&self, tag: [u8; 4]) -> Result<Option<Vec<u8>>, FtError> {
        let tag_c = c_ulong::from(u32::from_be_bytes(tag));
        let mut len: c_ulong = 0;
        // SAFETY: a null buffer asks only for the length.
        let err = unsafe {
            sys::FT_Load_Sfnt_Table(self.as_ptr(), tag_c, 0, ptr::null_mut(), &raw mut len)
        };
        if err != 0 {
            return Err(err);
        }
        if len == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` has `len` bytes.
        let err = unsafe {
            sys::FT_Load_Sfnt_Table(self.as_ptr(), tag_c, 0, buf.as_mut_ptr(), &raw mut len)
        };
        if err != 0 {
            return Err(err);
        }
        Ok(Some(buf))
    }

    /// The variation axes of this face (`FT_Get_MM_Var`).
    pub fn mm_axes(&self, lib: &Lib) -> Result<Vec<VarAxis>, FtError> {
        let mut mm: *mut sys::FT_MM_Var = ptr::null_mut();
        // SAFETY: out pointer is valid.
        let err = unsafe { sys::FT_Get_MM_Var(self.as_ptr(), &raw mut mm) };
        if err != 0 {
            return Err(err);
        }
        // SAFETY: `mm` is valid until FT_Done_MM_Var; `axis` has `num_axis` entries.
        let axes = unsafe {
            let m = &*mm;
            (0..m.num_axis as usize)
                .map(|i| {
                    let a = &*m.axis.add(i);
                    VarAxis {
                        tag: a.tag as u32,
                        minimum: a.minimum as i64,
                        def: a.def as i64,
                        maximum: a.maximum as i64,
                    }
                })
                .collect::<Vec<_>>()
        };
        // SAFETY: `mm` came from FT_Get_MM_Var on this library.
        unsafe {
            sys::FT_Done_MM_Var(lib.handle, mm);
        }
        Ok(axes)
    }

    pub fn var_design_coordinates(&self, n: usize) -> Result<Vec<i64>, FtError> {
        let mut coords: Vec<c_long> = vec![0; n];
        // SAFETY: buffer has `n` entries.
        let err = unsafe {
            sys::FT_Get_Var_Design_Coordinates(self.as_ptr(), n as c_uint, coords.as_mut_ptr())
        };
        if err != 0 {
            return Err(err);
        }
        Ok(coords.into_iter().map(|c| c as i64).collect())
    }

    pub fn set_var_design_coordinates(&self, coords: &[i64]) -> Result<(), FtError> {
        let c: Vec<c_long> = coords.iter().map(|&v| v as c_long).collect();
        // SAFETY: buffer has `c.len()` entries.
        let err = unsafe {
            sys::FT_Set_Var_Design_Coordinates(self.as_ptr(), c.len() as c_uint, c.as_ptr())
        };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    // -- glyph slot ---------------------------------------------------------

    fn slot(&self) -> &sys::FT_GlyphSlotRec {
        // SAFETY: a face always has a glyph slot; access is serialised by caller.
        unsafe { &*self.rec().glyph }
    }

    pub fn slot_info(&self) -> SlotInfo {
        let s = self.slot();
        SlotInfo {
            format: s.format,
            metrics_width: s.metrics.width as i64,
            metrics_height: s.metrics.height as i64,
            hori_bearing_x: s.metrics.horiBearingX as i64,
            hori_bearing_y: s.metrics.horiBearingY as i64,
            advance_x: s.advance.x as i64,
            bitmap_left: s.bitmap_left,
            bitmap_top: s.bitmap_top,
            pixel_mode: s.bitmap.pixel_mode as u8 as u32,
        }
    }

    /// Outline bounding box `(xMin, yMin, xMax, yMax)` in 26.6 units.
    pub fn slot_outline_bbox(&self) -> (i64, i64, i64, i64) {
        let mut bbox = std::mem::MaybeUninit::<sys::FT_BBox>::zeroed();
        // SAFETY: valid outline in the slot; out pointer valid.
        unsafe {
            sys::FT_Outline_Get_BBox(&raw const self.slot().outline, bbox.as_mut_ptr());
            let b = bbox.assume_init();
            (b.xMin as i64, b.yMin as i64, b.xMax as i64, b.yMax as i64)
        }
    }

    pub fn slot_embolden(&self, strength: i64) {
        // SAFETY: the slot's outline is modified in place; access is serialised.
        unsafe {
            let slot = self.rec().glyph;
            sys::FT_Outline_Embolden(&raw mut (*slot).outline, strength as c_long);
        }
    }

    /// Rewrites each outline point (26.6 coordinates) in place.
    pub fn slot_map_outline_points(&self, mut f: impl FnMut(i64, i64) -> (i32, i32)) {
        // SAFETY: `points` has `n_points` entries; access is serialised.
        unsafe {
            let slot = self.rec().glyph;
            let outline = &mut (*slot).outline;
            if outline.points.is_null() {
                return;
            }
            let pts =
                std::slice::from_raw_parts_mut(outline.points, outline.n_points.max(0) as usize);
            for p in pts {
                let (x, y) = f(p.x as i64, p.y as i64);
                p.x = c_long::from(x);
                p.y = c_long::from(y);
            }
        }
    }

    pub fn slot_render(&self, mono: bool) -> Result<(), FtError> {
        let mode = if mono { RENDER_MONO } else { RENDER_NORMAL };
        // SAFETY: valid glyph slot.
        let err = unsafe { sys::FT_Render_Glyph(self.rec().glyph, mode) };
        if err == 0 { Ok(()) } else { Err(err) }
    }

    /// Copy the slot's bitmap as-is.
    pub fn slot_bitmap(&self) -> Bitmap {
        let b = &self.slot().bitmap;
        Bitmap {
            buffer: copy_bitmap_bytes(b),
            rows: b.rows as u32,
            width: b.width as u32,
            pitch: b.pitch,
            num_grays: b.num_grays as u16,
            pixel_mode: b.pixel_mode as u8 as u32,
        }
    }

    /// `FT_Bitmap_Convert(slot.bitmap, 1)`: a dense 8bpp copy of the slot's
    /// bitmap.
    pub fn slot_bitmap_converted(&self, lib: &Lib) -> Result<Bitmap, FtError> {
        let mut target = std::mem::MaybeUninit::<sys::FT_Bitmap>::uninit();
        // SAFETY: initialises `target`; converts into it; freed with FT_Bitmap_Done.
        unsafe {
            FT_Bitmap_Init(target.as_mut_ptr());
            let mut target = target.assume_init();
            if FT_Bitmap_Convert(
                lib.handle,
                &raw const self.slot().bitmap,
                &raw mut target,
                1,
            ) != 0
            {
                FT_Bitmap_Done(lib.handle, &raw mut target);
                return Err(sys::FT_Err_Invalid_Argument);
            }
            let out = Bitmap {
                buffer: copy_bitmap_bytes(&target),
                rows: target.rows as u32,
                width: target.width as u32,
                pitch: target.pitch,
                num_grays: target.num_grays as u16,
                pixel_mode: target.pixel_mode as u8 as u32,
            };
            FT_Bitmap_Done(lib.handle, &raw mut target);
            Ok(out)
        }
    }

    /// Creates a harfbuzz font for this face (`hb_ft_font_create_referenced`).
    pub fn create_hb_font(&self) -> Option<HbFont> {
        // SAFETY: valid face; harfbuzz takes its own FreeType reference.
        let p = unsafe { harfbuzz_sys::freetype::hb_ft_font_create_referenced(self.as_ptr()) };
        NonNull::new(p).map(HbFont)
    }
}

fn copy_bitmap_bytes(b: &sys::FT_Bitmap) -> Vec<u8> {
    let len = b.pitch.unsigned_abs() as usize * b.rows as usize;
    if b.buffer.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: FreeType bitmaps own `|pitch| * rows` bytes at `buffer`.
    unsafe { std::slice::from_raw_parts(b.buffer, len) }.to_vec()
}

/// A harfbuzz font owned by a [`Face`](super::Face).
pub struct HbFont(NonNull<harfbuzz_sys::hb_font_t>);

// SAFETY: hb_font_t is thread-safe for shaping when not mutated; Ghostty uses
// it behind the collection lock exactly like upstream.
unsafe impl Send for HbFont {}
// SAFETY: see above.
unsafe impl Sync for HbFont {}

impl HbFont {
    /// The raw `hb_font_t` pointer (valid while the `Face` lives).
    pub fn as_ptr(&self) -> *mut harfbuzz_sys::hb_font_t {
        self.0.as_ptr()
    }
}

impl Drop for HbFont {
    fn drop(&mut self) {
        // SAFETY: we own one reference to the hb font.
        unsafe { harfbuzz_sys::hb_font_destroy(self.0.as_ptr()) };
    }
}
