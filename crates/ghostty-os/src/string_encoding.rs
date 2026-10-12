//! String decoding/encoding helpers (ZIG/src/os/string_encoding.zig).
//!
//! The implementation lives in `ghostty-vt` (the OSC 133 parser needs it and
//! vt cannot depend on os); this module re-exports it.

pub use ghostty_vt::string_encoding::{
    Error,
    printf_q_decode,
    url_percent_decode,
    url_percent_encode,
};
