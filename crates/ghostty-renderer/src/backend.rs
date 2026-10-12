//! Renderer backend selection (`renderer/backend.zig`).

/// Possible implementations, used for build options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Opengl,
    Metal,
    Webgl,
}

impl Backend {
    /// The default backend for the target (Metal on Apple platforms, OpenGL
    /// otherwise). WebGL is only a name here: the wasm build has no GL renderer
    /// in this crate. This is what [`crate::Api`] / [`crate::Renderer`] are
    /// instantiated with.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_matches_selected_api() {
        // `renderer.zig` picks the graphics API from the backend.
        if cfg!(target_vendor = "apple") {
            assert_eq!(Backend::default_for_target(), Backend::Metal);
        } else {
            assert_eq!(Backend::default_for_target(), Backend::Opengl);
        }
    }

    #[test]
    fn health_values_match_ghostty_h() {
        // `GHOSTTY_RENDERER_HEALTH_{OK,ERROR}`.
        assert_eq!(Health::Healthy as i32, 0);
        assert_eq!(Health::Unhealthy as i32, 1);
    }
}
