//! ghostty-vt: VT parsing layer of Ghostty (parser, OSC/CSI/DCS/APC, SGR,
//! colors, modes, stream). See docs/rewrite/ARCHITECTURE.md.

pub mod ansi;
pub mod apc;
pub mod charsets;
pub mod color;
pub mod csi;
pub mod dcs;
pub mod device_status;
pub mod kitty;
pub mod modes;
pub mod mouse_shape;
pub mod osc;
pub mod parse_table;
pub mod parser;
pub mod sgr;
pub mod simd;
pub mod stream;
pub mod tmux;
pub mod utf8_decoder;
pub mod x11_color;
