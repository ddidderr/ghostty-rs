//! `font/Atlas.zig` `Wasm`: the wasm-compatible atlas API.

use ghostty_font::atlas::{Atlas, Format, Region};
use ghostty_os::wasm::js::{self, Object};

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

/// This creates a Canvas element identified by the id returned that the
/// caller can draw into the DOM to visualize the atlas. The returned ID must
/// be freed from the JS runtime by calling `zigjs.deleteValue`. Returns 0 on
/// error (that is also never a valid id: 0 is the predefined `NaN`).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_debug_canvas(atlas: *const Atlas) -> u32 {
    // SAFETY: live pointer from `atlas_new`.
    let atlas = unsafe { &*atlas };
    match debug_canvas(atlas) {
        Ok(id) => id,
        Err(err) => {
            log::warn!("error dumping atlas canvas err={err:?}");
            0
        }
    }
}

fn debug_canvas(atlas: &Atlas) -> Result<u32, DebugCanvasError> {
    // Create our canvas
    let doc = js::global().get::<Object>("document")?;
    let canvas = doc.call::<Object>("createElement", &[&js::string("canvas")])?;

    // Setup our canvas size
    {
        canvas.set("width", atlas.size)?;
        canvas.set("height", atlas.size)?;

        let width_str = format!("{}px", atlas.size);
        let style = canvas.get::<Object>("style")?;
        style.set("width", js::string(&width_str))?;
        style.set("height", js::string(&width_str))?;
    }

    // This will return the same context on subsequent calls so it is
    // important to reset it.
    let ctx = canvas.call::<Object>("getContext", &[&js::string("2d")])?;

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
        Format::Bgr => return Err(DebugCanvasError::UnsupportedAtlasFormat),
    };

    // Create an ImageData from our buffer and then write it to the canvas
    let image_data = {
        // Get our runtime memory
        let mem = js::runtime().get::<Object>("memory")?;
        let mem_buf = mem.get::<Object>("buffer")?;

        // Create an array that points to our buffer
        let arr = {
            let uint8_clamped_array = js::global().get::<Object>("Uint8ClampedArray")?;
            let arr = uint8_clamped_array.new_instance(&[
                &mem_buf,
                &(buf.as_ptr() as usize),
                &buf.len(),
            ])?;
            if ghostty_os::wasm::SHARED_MEM {
                // If we're sharing memory then we have to copy the data since
                // we can't set ImageData directly using a SharedArrayBuffer.
                arr.call::<Object>("slice", &[])?
            } else {
                arr
            }
        };

        // Create the image data from our array
        let image_data = js::global().get::<Object>("ImageData")?;
        image_data.new_instance(&[&arr, &atlas.size, &atlas.size])?
    };

    // Draw it
    ctx.call::<()>("putImageData", &[&image_data, &0, &0])?;

    Ok(canvas.into_id())
}

#[derive(Debug)]
enum DebugCanvasError {
    Js(js::Error),
    UnsupportedAtlasFormat,
}

impl From<js::Error> for DebugCanvasError {
    fn from(e: js::Error) -> Self {
        DebugCanvasError::Js(e)
    }
}
