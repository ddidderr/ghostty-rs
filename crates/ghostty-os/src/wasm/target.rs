//! The wasm target platform. Port of `ZIG/src/os/wasm/target.zig`.
//!
//! This is used to toggle certain features on and off since the standard
//! triple target is often not specific enough (i.e. we can't tell
//! wasm32-freestanding is for browser or not).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Browser,
}

/// Our specific target platform. `None` on non-wasm targets (Zig); this
/// module only exists on wasm32 so it is always the browser.
pub const TARGET: Option<Target> = Some(Target::Browser);
