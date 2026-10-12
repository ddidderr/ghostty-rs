//! Build-time information shown by `+version` (Zig: `build_config`,
//! `builtin`, `gtk_version`, `adw_version`).
//!
//! TODO(integration): the root crate / apprt-gtk should call [`set`] with the
//! real GTK and libadwaita versions (the Zig code queries them at runtime).

use std::sync::OnceLock;

/// A `major.minor.micro` version (Zig: `gtk_version.zig` / `adw_version.zig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)
    }
}

#[derive(Debug, Clone)]
pub struct BuildInfo {
    /// `build_config.version_string`.
    pub version_string: String,
    /// `build_config.version.build` (the git commit hash), if known.
    pub commit: Option<String>,
    /// `build_config.release_channel` tag name (`stable` / `tip`).
    pub release_channel: String,
    /// `{}` of `builtin.mode` (`.Debug`, `.ReleaseFast`, ...).
    pub build_mode: String,
    /// `{}` of `build_config.app_runtime`.
    pub app_runtime: String,
    /// `{}` of `build_config.font_backend`.
    pub font_backend: String,
    /// `{}` of `renderer.Renderer`.
    pub renderer: String,
    pub gtk_build: Option<Version>,
    pub gtk_runtime: Option<Version>,
    pub adw_build: Option<Version>,
    pub adw_runtime: Option<Version>,
    /// `build_options.x11`.
    pub x11: bool,
    /// `build_options.wayland`.
    pub wayland: bool,
}

impl Default for BuildInfo {
    fn default() -> Self {
        BuildInfo {
            // Derived by `build.rs` (git describe / workspace app-version).
            version_string: env!("GHOSTTY_VERSION_STRING").to_string(),
            commit: Some(env!("GHOSTTY_COMMIT").to_string()).filter(|c| !c.is_empty()),
            release_channel: env!("GHOSTTY_RELEASE_CHANNEL").to_string(),
            build_mode: env!("GHOSTTY_BUILD_MODE").to_string(),
            app_runtime: ".gtk".to_string(),
            font_backend: ".fontconfig_freetype".to_string(),
            renderer: "renderer.generic.Renderer(renderer.OpenGL)".to_string(),
            gtk_build: None,
            gtk_runtime: None,
            adw_build: None,
            adw_runtime: None,
            x11: true,
            wayland: true,
        }
    }
}

static INFO: OnceLock<BuildInfo> = OnceLock::new();

/// Install the build info. Returns false if it was already set.
pub fn set(info: BuildInfo) -> bool {
    INFO.set(info).is_ok()
}

/// The installed build info, or the defaults.
#[must_use]
pub fn get() -> BuildInfo {
    INFO.get().cloned().unwrap_or_default()
}
