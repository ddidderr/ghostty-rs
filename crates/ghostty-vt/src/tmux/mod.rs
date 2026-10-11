//! Types and functions related to tmux protocols (ZIG/src/terminal/tmux.zig).
//!
//! Only the pure control-mode parser lives in this crate; layout, output and
//! viewer belong to `ghostty-terminal`.

pub mod control;

pub use control::{Notification as ControlNotification, Parser as ControlParser};
