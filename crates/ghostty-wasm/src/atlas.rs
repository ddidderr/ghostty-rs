//! `font/Atlas.zig` `Wasm`: the wasm-compatible atlas API.

use ghostty_font::atlas::{Atlas, Format, Region};
use ghostty_os::wasm::js::{self, Arg, Object};

use crate::{ensure_init, host_bytes};

/// C layout of `Atlas.Region`.
#[repr(C)]
pub struct RegionC {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl From<RegionC> for Region {
    fn from(r: RegionC) -> Region {
        Region {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

fn format_from(v: u8) -> Option<Format> {
    match v {
        0 => Some(Format::Grayscale),
        1 => Some(Format::Bgr),
        2 => Some(Format::Bgra),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn atlas_new(size: u32, format: u8) -> *mut Atlas {
    ensure_init();
    let Some(format) = format_from(format) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(Atlas::new(size, format)))
}

#[unsafe(no_mangle)]
pub extern "C" fn atlas_free(ptr: *mut Atlas) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `atlas_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// The return value must be freed by the caller with [`region_free`].
#[unsafe(no_mangle)]
pub extern "C" fn atlas_reserve(atlas: *mut Atlas, width: u32, height: u32) -> *mut RegionC {
    // SAFETY: the host passes a live pointer from `atlas_new`.
    let atlas = unsafe { &mut *atlas };
    match atlas.reserve(width, height) {
        Ok(r) => Box::into_raw(Box::new(RegionC {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn region_free(ptr: *mut RegionC) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `atlas_reserve` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn atlas_set(atlas: *mut Atlas, reg: *const RegionC, data: *const u8, len: usize) {
    // SAFETY: live pointers from the host; `data` is valid for `len` bytes.
    let (atlas, reg, data) = unsafe { (&mut *atlas, &*reg, host_bytes(data, len)) };
    atlas.set(
        Region {
            x: reg.x,
            y: reg.y,
            width: reg.width,
            height: reg.height,
        },
        data,
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn atlas_grow(atlas: *mut Atlas, size_new: u32) -> bool {
    // SAFETY: live pointer from `atlas_new`.
    unsafe { &mut *atlas }.grow(size_new);
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn atlas_clear(atlas: *mut Atlas) {
    // SAFETY: live pointer from `atlas_new`.
    unsafe { &mut *atlas }.clear();
}

/// Returns a pointer to the atlas pixel data and its length through `len`
/// (host convenience; Zig hosts read `atlas.data` through the struct layout).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_data(atlas: *const Atlas, len: *mut usize) -> *const u8 {
    // SAFETY: live pointers from the host.
    let (atlas, len) = unsafe { (&*atlas, &mut *len) };
    *len = atlas.data.len();
    atlas.data.as_ptr()
}

/// The atlas edge length in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_size(atlas: *const Atlas) -> u32 {
    // SAFETY: live pointer from `atlas_new`.
    unsafe { &*atlas }.size
}

/// This creates a Canvas element identified by the handle returned that the
/// caller can draw into the DOM to visualize the atlas. The returned handle
/// must be released from the JS runtime (`ghostty_js` release / zigjs
/// `deleteValue`). Returns 0 on error.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_debug_canvas(atlas: *const Atlas) -> u32 {
    // SAFETY: live pointer from `atlas_new`.
    let atlas = unsafe { &*atlas };
    match debug_canvas(atlas) {
        Ok(c) => c.into_raw(),
        Err(err) => {
            log::warn!("error dumping atlas canvas err={err}");
            0
        }
    }
}

fn debug_canvas(atlas: &Atlas) -> Result<Object, js::Error> {
    // Create our canvas
    let doc = js::global()?.get_object("document")?;
    let canvas = doc.call("createElement", &[Arg::Str("canvas")])?;

    // Setup our canvas size
    {
        canvas.set("width", Arg::from(atlas.size))?;
        canvas.set("height", Arg::from(atlas.size))?;
        let style = canvas.get_object("style")?;
        let px = format!("{}px", atlas.size);
        style.set("width", Arg::Str(&px))?;
        style.set("height", Arg::Str(&px))?;
    }

    // This will return the same context on subsequent calls.
    let ctx = canvas.call("getContext", &[Arg::Str("2d")])?;

    // We need to draw pixels so this is format dependent.
    let buf: Vec<u8> = match atlas.format {
        // Convert from BGRA to RGBA by swapping every R and B.
        Format::Bgra => {
            let mut buf = atlas.data.clone();
            for px in buf.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            buf
        }
        // Convert from A8 to RGBA so every 4th byte is set to a value.
        Format::Grayscale => {
            let mut buf = vec![0u8; atlas.data.len() * 4];
            for (i, v) in atlas.data.iter().enumerate() {
                buf[i * 4 + 3] = *v;
            }
            buf
        }
        Format::Bgr => {
            log::warn!("error dumping atlas canvas err=UnsupportedAtlasFormat");
            return Err(js::Error::Js);
        }
    };

    // Create an ImageData from our buffer and then write it to the canvas
    let arr = Object::clamped_array(&buf)?;
    let image_data_ctor = js::global()?.get_object("ImageData")?;
    let image_data = image_data_ctor.new_instance(&[
        Arg::Object(&arr),
        Arg::from(atlas.size),
        Arg::from(atlas.size),
    ])?;
    ctx.call(
        "putImageData",
        &[Arg::Object(&image_data), Arg::Num(0.0), Arg::Num(0.0)],
    )?;

    Ok(canvas)
}
