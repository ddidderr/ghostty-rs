//! Safe page-based terminal memory model.
#![cfg_attr(test, allow(clippy::unwrap_used))]
pub mod cursor;
pub mod formatter;
pub mod highlight;
pub mod hyperlink;
pub mod kitty;
pub mod page;
pub mod page_list;
pub mod pin_iter;
pub mod point;
pub mod ref_counted_set;
pub mod render;
pub mod screen;
pub mod screen_set;
pub mod search;
pub mod selection;
pub mod size;
pub mod stream_readonly;
pub mod string_map;
pub mod style;
pub mod tabstops;
pub mod terminal;
pub mod tmux;
pub mod tripwire;
