//! Types and functions related to tmux protocols.

pub mod layout;
pub mod output;
pub mod viewer;

pub use ghostty_vt::tmux::control;
pub use ghostty_vt::tmux::control::{Notification as ControlNotification, Parser as ControlParser};
pub use layout::Layout;
pub use viewer::Viewer;
