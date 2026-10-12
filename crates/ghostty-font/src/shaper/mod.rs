//! Text shapers. Port of `font/shaper/`.

#[cfg(not(target_arch = "wasm32"))]
pub mod cache;
#[cfg(font_coretext_shaper)]
mod cf_release;
#[cfg(font_coretext_shaper)]
pub mod coretext;
pub mod coretext_cells;
pub mod feature;
#[cfg(font_harfbuzz)]
pub mod harfbuzz;
#[cfg(font_harfbuzz)]
mod hb;
#[cfg(not(target_arch = "wasm32"))]
pub mod noop;
#[cfg(not(target_arch = "wasm32"))]
pub mod run;

pub mod web_canvas;

#[cfg(all(test, font_harfbuzz, font_fontconfig))]
pub(crate) mod test_util;
