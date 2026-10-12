//! `font/shaper/web_canvas.zig` `Wasm` (`shaper_*`).

use ghostty_font::{face::web_canvas::Face, shaper::web_canvas::Shaper};
use ghostty_terminal::terminal::{Options, Terminal};

use crate::{ensure_init, host_bytes};

#[unsafe(no_mangle)]
pub extern "C" fn shaper_new(cap: usize) -> *mut Shaper {
    ensure_init();
    Box::into_raw(Box::new(Shaper::new(cap)))
}

#[unsafe(no_mangle)]
pub extern "C" fn shaper_free(ptr: *mut Shaper) {
    if !ptr.is_null() {
        // SAFETY: pointer came from `shaper_new` and is freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// Runs a test to verify shaping works properly: prints the string into a
/// terminal, then shapes and logs every row.
///
/// Zig takes a `GroupCache` (stale upstream); this takes the face that
/// reserves grapheme glyph indexes.
#[unsafe(no_mangle)]
pub extern "C" fn shaper_test(shaper: *mut Shaper, face: *mut Face, s: *const u8, len: usize) {
    // SAFETY: live pointers from the host; `s` is valid for `len` bytes.
    let (shaper, face, s) = unsafe { (&mut *shaper, &mut *face, host_bytes(s, len)) };
    if let Err(err) = shaper_test_(shaper, face, s) {
        log::warn!("error during shaper test err={err}");
    }
}

fn shaper_test_(shaper: &mut Shaper, face: &mut Face, s: &[u8]) -> Result<(), String> {
    let s = std::str::from_utf8(s).map_err(|e| e.to_string())?;

    // Create a terminal and print all our characters into it.
    let cols = u16::try_from(shaper.cell_buf_len()).map_err(|e| e.to_string())?;
    let mut term = Terminal::new(Options::new(cols, 80)).map_err(|e| format!("{e:?}"))?;
    for c in s.chars() {
        term.print(u32::from(c)).map_err(|e| format!("{e:?}"))?;
    }

    // Iterate over the rows and print out all the runs we get.
    let plain = term.plain_string().map_err(|e| format!("{e:?}"))?;
    for (y, row) in plain.lines().enumerate() {
        if row.is_empty() {
            continue;
        }
        shaper.prepare();
        for (x, c) in row.chars().enumerate() {
            shaper.add_codepoint(u32::from(c), x as u32);
        }
        shaper.finalize();
        let cells = shaper
            .shape(|cluster| face.grapheme_glyph_index(cluster))
            .map_err(|e| e.to_string())?;
        log::info!("y={y} shape={cells:?}");
    }
    Ok(())
}

/// Extra (not in Zig): prepares a run for the hook-style API.
#[unsafe(no_mangle)]
pub extern "C" fn shaper_prepare(shaper: *mut Shaper) {
    // SAFETY: live pointer from `shaper_new`.
    unsafe { &mut *shaper }.prepare();
}

/// Extra (not in Zig): `RunIteratorHook.addCodepoint`.
#[unsafe(no_mangle)]
pub extern "C" fn shaper_add_codepoint(shaper: *mut Shaper, cp: u32, cluster: u32) {
    // SAFETY: live pointer from `shaper_new`.
    unsafe { &mut *shaper }.add_codepoint(cp, cluster);
}

/// Extra (not in Zig): shapes the current run. Writes `(x, glyph_index)` u32
/// pairs to `out` (capacity `cap` pairs) and returns the number of cells, or
/// `u32::MAX` on error.
#[unsafe(no_mangle)]
pub extern "C" fn shaper_shape(
    shaper: *mut Shaper,
    face: *mut Face,
    out: *mut u32,
    cap: usize,
) -> u32 {
    // SAFETY: live pointers from the host; `out` has room for `cap` pairs.
    let (shaper, face, out) = unsafe {
        (
            &mut *shaper,
            &mut *face,
            std::slice::from_raw_parts_mut(out, cap * 2),
        )
    };
    match shaper.shape(|cluster| face.grapheme_glyph_index(cluster)) {
        Ok(cells) if cells.len() <= cap => {
            for (i, c) in cells.iter().enumerate() {
                out[i * 2] = u32::from(c.x);
                out[i * 2 + 1] = c.glyph_index;
            }
            cells.len() as u32
        }
        Ok(_) => u32::MAX,
        Err(err) => {
            log::warn!("error shaping err={err}");
            u32::MAX
        }
    }
}
