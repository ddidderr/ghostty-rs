//! Config keys (port of `config/key.zig`).
//!
//! Zig builds an enum of all config fields via comptime. In Rust a [`Key`] is
//! the (static) field name; constants for every field are in [`crate::keys`].

/// The name of a configuration field, e.g. `Key("font-size")`. Valid
/// instances are available as constants in [`crate::keys`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(pub &'static str);

impl Key {
    /// The user-facing name of this key.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.0
    }
}
