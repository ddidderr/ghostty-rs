//! Sublime syntax (also used by `bat`) (port of `extra/sublime.zig`).

use crate::chunks_sublime::{C0, C1};
use crate::config_fields;

#[must_use]
pub fn syntax() -> String {
    let keywords: Vec<_> = config_fields().iter().map(|f| f.name).collect();
    format!("{C0}{}{C1}", keywords.join("|"))
}
