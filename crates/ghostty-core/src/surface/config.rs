//! The configuration a surface has (Zig: `Surface.DerivedConfig`).
//!
//! Zig copies a subset of the main config so no pointer to it needs to stay
//! alive. In Rust the whole (conditional-state-applied) [`Config`] is kept
//! behind an `Arc`, which is the same thing, and `DerivedConfig` derefs to
//! it so the fields keep their Zig names (`cfg.copy_on_select`, ...). The
//! few values that are *computed* in Zig live as fields of this struct.

use std::{ops::Deref, sync::Arc};

use ghostty_config::config::Config;
use ghostty_input::link::{Action as LinkAction, Highlight};
use ghostty_renderer::size::Padding;
use onig::Regex;

/// A link with its compiled regex (Zig: `DerivedConfig.Link`).
pub struct Link {
    pub regex: Regex,
    pub action: LinkAction,
    pub highlight: Highlight,
}

/// See the module docs.
pub struct DerivedConfig {
    /// The config this was derived from (conditional state already applied).
    pub config: Arc<Config>,

    /// `font-size`.
    pub original_font_size: f32,

    /// `click-repeat-interval` in nanoseconds (the config is in ms).
    pub mouse_interval: u64,

    /// `abnormal-command-exit-runtime` (ms).
    pub abnormal_command_exit_runtime_ms: u32,

    /// `selection-word-chars` codepoints.
    pub selection_word_chars: Vec<u32>,

    /// The compiled `link` entries.
    pub links: Vec<Link>,
}

impl Deref for DerivedConfig {
    type Target = Config;
    fn deref(&self) -> &Config {
        &self.config
    }
}

impl DerivedConfig {
    /// Derive the surface configuration. Fails if a link regex is invalid.
    pub fn new(config: Arc<Config>) -> Result<DerivedConfig, onig::Error> {
        // Build all of our links
        let mut links = Vec::with_capacity(config.link.links.len());
        for link in &config.link.links {
            links.push(Link {
                regex: Regex::new(&link.regex)?,
                action: link.action,
                highlight: link.highlight,
            });
        }

        Ok(DerivedConfig {
            original_font_size: config.font_size,
            mouse_interval: u64::from(config.click_repeat_interval) * 1_000_000, // 500ms
            abnormal_command_exit_runtime_ms: config.abnormal_command_exit_runtime,
            selection_word_chars: config.selection_word_chars.codepoints.clone(),
            links,
            config,
        })
    }

    /// Padding scaled by the DPI (Zig: `scaledPadding`).
    #[must_use]
    pub fn scaled_padding(&self, x_dpi: f32, y_dpi: f32) -> Padding {
        let scale = |v: u32, dpi: f32| -> u32 { (v as f32 * dpi / 72.0).floor() as u32 };
        Padding {
            top: scale(self.window_padding_y.top_left, y_dpi),
            bottom: scale(self.window_padding_y.bottom_right, y_dpi),
            left: scale(self.window_padding_x.top_left, x_dpi),
            right: scale(self.window_padding_x.bottom_right, x_dpi),
        }
    }
}
