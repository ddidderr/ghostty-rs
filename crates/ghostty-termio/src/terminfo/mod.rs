//! Package terminfo provides functionality related to terminfo/termcap files.
//!
//! At the time of writing this comment, the focus is on generating terminfo
//! files so that we can maintain our terminfo in Rust instead of hand-writing
//! the archaic (imo) terminfo format by hand. But eventually we may want to
//! extract this into a more full-featured library on its own.
//!
//! Port of `terminfo/main.zig`.

pub mod ghostty;
pub mod source;

pub use ghostty::GHOSTTY as ghostty;
pub use source::Source;
