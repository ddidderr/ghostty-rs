//! Renderer backend selection (`renderer/backend.zig`).

/// Possible implementations, used for build options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Opengl,
    Metal,
    Webgl,
}

impl Backend {
    /// The default backend for the target. Metal/WebGL are out of scope for
    /// the Rust port (Linux only), so this is always OpenGL on non-Darwin.
    pub fn default_for_target() -> Backend {
        if cfg!(target_vendor = "apple") {
            Backend::Metal
        } else {
            Backend::Opengl
        }
    }
}

/// The health status of a renderer (`renderer.Health`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum Health {
    Healthy,
    Unhealthy,
}
