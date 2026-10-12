//! Text shapers. Port of `font/shaper/`.

#[cfg(not(target_arch = "wasm32"))]
pub mod cache;
pub mod feature;
#[cfg(not(target_arch = "wasm32"))]
pub mod harfbuzz;
#[cfg(not(target_arch = "wasm32"))]
mod hb;
#[cfg(not(target_arch = "wasm32"))]
pub mod noop;
#[cfg(not(target_arch = "wasm32"))]
pub mod run;

pub mod web_canvas;

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod test_util;
