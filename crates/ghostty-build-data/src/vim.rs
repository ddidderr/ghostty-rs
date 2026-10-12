//! Vim/Neovim plugin files (port of `extra/vim.zig`).

use std::fmt::Write as _;

use crate::chunks_vim::{C0, C1, C2, C3, C4};
use crate::config_fields;

pub const FTDETECT: &str = C0;
pub const FTPLUGIN: &str = C1;
pub const COMPILER: &str = C2;

#[must_use]
pub fn syntax() -> String {
    let mut w = String::from(C3);
    for f in config_fields() {
        let _ = write!(w, "\n\t\\ {}", f.name);
    }
    w.push_str(C4);
    w
}
