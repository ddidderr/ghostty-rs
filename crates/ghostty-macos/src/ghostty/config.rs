//! `Ghostty.Config` (`Ghostty.Config.swift`): maps to a `ghostty_config_t` and
//! the various operations on that.
//!
//! The Swift class is an `ObservableObject`; nothing in the app observes its
//! (never mutated after init) state, so this is a plain owner of the C handle.
//! Share it with `Rc<Config>`.
//!
//! Deviations (recorded in `docs/rewrite/notes/macos-app-MA1.md`):
//! * `background_color` falls back to a fixed light gray where Swift uses the
//!   dynamic `NSColor.windowBackgroundColor`; use [`Config::background_color_opt`]
//!   to detect the fallback.
//! * `window_fullscreen` / `window_fullscreen_mode` return the raw config
//!   strings (`FullscreenMode` belongs to `helpers::fullscreen`).
//!   TODO(MA3): convert to `FullscreenMode`.
#![allow(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::assert_is_empty))]

use std::{
    cell::Cell,
    ffi::{CStr, CString, c_char, c_int, c_uint, c_void},
    time::Duration,
};

use super::{
    capi::{
        self,
        ghostty_config_color_list_s,
        ghostty_config_color_s,
        ghostty_config_command_list_s,
        ghostty_config_path_s,
        ghostty_config_quick_terminal_size_s,
        ghostty_config_t,
    },
    command::Command,
    config_types::{
        AutoUpdate,
        AutoUpdateChannel,
        BackgroundBlur,
        BellFeatures,
        ConfigPath,
        MacDockDropBehavior,
        MacHidden,
        MacOSIcon,
        MacOSIconFrame,
        MacOSTitlebarProxyIcon,
        MacOSTitlebarStyle,
        MacOSWindowButtons,
        MacShortcuts,
        NotifyOnCommandFinish,
        NotifyOnCommandFinishAction,
        ResizeOverlay,
        ResizeOverlayPosition,
        Scrollbar,
        SplitPreserveZoom,
        WindowDecoration,
    },
    input,
};
use crate::{
    features::quick_terminal::{
        QuickTerminalPosition,
        QuickTerminalScreen,
        QuickTerminalSize,
        QuickTerminalSpaceBehavior,
    },
    helpers::{app_info::is_running_in_xcode, color::Color, keyboard_shortcut::KeyboardShortcut},
};

/// Maps to a `ghostty_config_t` and the various operations on that.
pub struct Config {
    /// The underlying C pointer to the Ghostty config structure (null = nil).
    /// This should never be accessed directly. Any operations on this should
    /// be called from the functions on this or another class.
    config: Cell<ghostty_config_t>,
}

impl Drop for Config {
    fn drop(&mut self) {
        let old = self.config.replace(std::ptr::null_mut());
        if !old.is_null() {
            // SAFETY: we own the handle and nothing uses it afterwards.
            unsafe { capi::ghostty_config_free(old) };
        }
    }
}

impl Config {
    /// `init(config:)`: takes ownership of `config` (may be null).
    #[must_use]
    pub fn from_ptr(config: ghostty_config_t) -> Self {
        Self {
            config: Cell::new(config),
        }
    }

    /// `init(at:finalize:)`.
    ///
    /// * `path`: An optional preferred config file path. Pass `None` to load the default configuration files.
    /// * `finalize`: Whether to finalize the configuration to populate default values.
    #[must_use]
    pub fn new(path: Option<&str>, finalize: bool) -> Self {
        Self::from_ptr(Self::load_config(path, finalize))
    }

    /// `init(clone:)`.
    ///
    /// # Safety
    /// `config` must be a valid config handle.
    #[must_use]
    pub unsafe fn clone_from_ptr(config: ghostty_config_t) -> Self {
        // SAFETY: per the contract.
        Self::from_ptr(unsafe { capi::ghostty_config_clone(config) })
    }

    /// The raw handle (for `ghostty_app_update_config` and friends). Owned by
    /// `self`; null when not loaded.
    #[must_use]
    pub fn as_ptr(&self) -> ghostty_config_t {
        self.config.get()
    }

    /// True if the configuration is loaded
    #[must_use]
    pub fn loaded(&self) -> bool {
        !self.config.get().is_null()
    }

    /// Return the errors found while loading the configuration.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        let cfg = self.config.get();
        if cfg.is_null() {
            return Vec::new();
        }

        let mut diags = Vec::new();
        // SAFETY: `cfg` is a valid handle owned by `self`.
        let count = unsafe { capi::ghostty_config_diagnostics_count(cfg) };
        for i in 0..count {
            // SAFETY: as above; the message lives as long as the config.
            let diag = unsafe { capi::ghostty_config_get_diagnostic(cfg, i) };
            diags.push(cstr_to_string(diag.message));
        }

        diags
    }

    /// Initializes a new configuration and loads all the values.
    ///
    /// * `path`: An optional preferred config file path. Pass `None` to load the default configuration files.
    /// * `finalize`: Whether to finalize the configuration to populate default values.
    #[must_use]
    pub fn load_config(path: Option<&str>, finalize: bool) -> ghostty_config_t {
        // Initialize the global configuration.
        let cfg = capi::ghostty_config_new();
        if cfg.is_null() {
            log::error!("ghostty_config_new failed");
            return std::ptr::null_mut();
        }

        // Load our configuration from files, CLI args, and then any referenced files.
        // We only do this on macOS because other Apple platforms do not have the
        // same filesystem concept. (This crate only targets macOS.)
        // SAFETY: `cfg` is the valid handle created above; strings are NUL terminated.
        unsafe {
            if let Some(path) = path {
                match CString::new(path) {
                    Ok(path) => capi::ghostty_config_load_file(cfg, path.as_ptr()),
                    Err(_) => log::warn!("config path contains a NUL byte path={path:?}"),
                }
            } else {
                capi::ghostty_config_load_default_files(cfg);
            }

            // We only load CLI args when not running in Xcode because in Xcode we
            // pass some special parameters to control the debugger.
            if !is_running_in_xcode() {
                capi::ghostty_config_load_cli_args(cfg);
            }

            capi::ghostty_config_load_recursive_files(cfg);
        }

        // TODO: we'd probably do some config loading here... for now we'd
        // have to do this synchronously. When we support config updating we can do
        // this async and update later.

        if finalize {
            // Finalize will make our defaults available.
            // SAFETY: valid handle.
            unsafe { capi::ghostty_config_finalize(cfg) };
        }
        // Log any configuration errors. These will be automatically shown in a
        // pop-up window too.
        // SAFETY: valid handle.
        let diags_count = unsafe { capi::ghostty_config_diagnostics_count(cfg) };
        if diags_count > 0 {
            log::warn!("config error: {diags_count} configuration errors on reload");
            for i in 0..diags_count {
                // SAFETY: valid handle; the message lives as long as the config.
                let diag = unsafe { capi::ghostty_config_get_diagnostic(cfg, i) };
                log::warn!("config error: {}", cstr_to_string(diag.message));
            }
        }

        cfg
    }

    // MARK: - Keybindings

    /// Return the key equivalent for the given action. The action is the name of the action
    /// in the Ghostty configuration. For example `keybind = cmd+q=quit` in Ghostty
    /// configuration would be "quit" action.
    ///
    /// Returns nil if there is no key equivalent for the given action.
    #[must_use]
    pub fn keyboard_shortcut(&self, action: &str) -> Option<KeyboardShortcut> {
        let cfg = self.config.get();
        if cfg.is_null() {
            return None;
        }

        // SAFETY: valid handle; `action` is `len` readable bytes.
        let trigger =
            unsafe { capi::ghostty_config_trigger(cfg, action.as_ptr().cast(), action.len()) };
        input::keyboard_shortcut(&trigger)
    }

    // MARK: - Raw access

    /// `ghostty_config_get(config, &v, key, key.count)`.
    fn get_raw<T>(&self, key: &str, out: &mut T) -> bool {
        let cfg = self.config.get();
        // SAFETY: valid (or null, which the C API tolerates) handle; `out` is
        // storage of the key's C type, chosen by the caller per key.
        unsafe {
            capi::ghostty_config_get(
                cfg,
                std::ptr::from_mut(out).cast::<c_void>(),
                key.as_ptr().cast(),
                key.len(),
            )
        }
    }

    /// A plain value of C type `T` (`bool`, integer, float): `None` if the key is unsupported/unset.
    fn get_value<T>(&self, key: &str, init: T) -> Option<T> {
        let mut v = init;
        self.get_raw(key, &mut v).then_some(v)
    }

    /// Bool getter: the value if the lookup works, `init` otherwise.
    fn get_bool(&self, key: &str, init: bool) -> bool {
        let mut v = init;
        let _ = self.get_raw(key, &mut v);
        v
    }

    /// A `?[:0]const u8` / enum-tag string (`UnsafePointer<Int8>?`).
    fn get_string(&self, key: &str) -> Option<String> {
        let mut v: *const c_char = std::ptr::null();
        if !self.get_raw(key, &mut v) || v.is_null() {
            return None;
        }
        Some(cstr_to_string(v))
    }

    /// A color (`ghostty_config_color_s`).
    fn get_color(&self, key: &str) -> Option<ghostty_config_color_s> {
        let mut v = ghostty_config_color_s { r: 0, g: 0, b: 0 };
        self.get_raw(key, &mut v).then_some(v)
    }

    // MARK: - Configuration Values

    // For all of the configuration values below, see the associated Ghostty documentation for
    // details on what each means. We only add documentation if there is a strange conversion
    // due to the embedded library and Rust.

    #[must_use]
    pub fn bell_features(&self) -> BellFeatures {
        if !self.loaded() {
            return BellFeatures::empty();
        }
        self.get_value::<c_uint>("bell-features", 0)
            .map_or_else(BellFeatures::empty, BellFeatures::from_bits_retain)
    }

    #[must_use]
    pub fn bell_audio_path(&self) -> Option<ConfigPath> {
        if !self.loaded() {
            return None;
        }
        let mut v = ghostty_config_path_s {
            path: c"".as_ptr(),
            optional: false,
        };
        let key = "bell-audio-path";
        if !self.get_raw(key, &mut v) {
            return None;
        }
        let path = cstr_to_string(v.path);
        if path.is_empty() {
            None
        } else {
            Some(ConfigPath {
                path,
                optional: v.optional,
            })
        }
    }

    #[must_use]
    pub fn bell_audio_volume(&self) -> f32 {
        if !self.loaded() {
            return 0.5;
        }
        let mut v: f64 = 0.5;
        let _ = self.get_raw("bell-audio-volume", &mut v);
        v as f32
    }

    #[must_use]
    pub fn notify_on_command_finish(&self) -> NotifyOnCommandFinish {
        if !self.loaded() {
            return NotifyOnCommandFinish::Never;
        }
        self.get_string("notify-on-command-finish")
            .and_then(|s| NotifyOnCommandFinish::from_raw_value(&s))
            .unwrap_or(NotifyOnCommandFinish::Never)
    }

    #[must_use]
    pub fn notify_on_command_finish_action(&self) -> NotifyOnCommandFinishAction {
        let default_value = NotifyOnCommandFinishAction::BELL;
        if !self.loaded() {
            return default_value;
        }
        self.get_value::<c_uint>("notify-on-command-finish-action", 0)
            .map_or(default_value, NotifyOnCommandFinishAction::from_bits_retain)
    }

    #[must_use]
    pub fn notify_on_command_finish_after(&self) -> Duration {
        if !self.loaded() {
            return Duration::from_secs(5);
        }
        let mut v: usize = 0;
        let _ = self.get_raw("notify-on-command-finish-after", &mut v);
        Duration::from_millis(v as u64)
    }

    #[must_use]
    pub fn split_preserve_zoom(&self) -> SplitPreserveZoom {
        if !self.loaded() {
            return SplitPreserveZoom::empty();
        }
        self.get_value::<c_uint>("split-preserve-zoom", 0)
            .map_or_else(
                SplitPreserveZoom::empty,
                SplitPreserveZoom::from_bits_retain,
            )
    }

    #[must_use]
    pub fn initial_window(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("initial-window", true)
    }

    #[must_use]
    pub fn should_quit_after_last_window_closed(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("quit-after-last-window-closed", false)
    }

    #[must_use]
    pub fn title(&self) -> Option<String> {
        if !self.loaded() {
            return None;
        }
        self.get_string("title")
    }

    #[must_use]
    pub fn window_save_state(&self) -> String {
        if !self.loaded() {
            return String::new();
        }
        self.get_string("window-save-state").unwrap_or_default()
    }

    #[must_use]
    pub fn window_position_x(&self) -> Option<i16> {
        if !self.loaded() {
            return None;
        }
        self.get_value::<i16>("window-position-x", 0)
    }

    #[must_use]
    pub fn window_position_y(&self) -> Option<i16> {
        if !self.loaded() {
            return None;
        }
        self.get_value::<i16>("window-position-y", 0)
    }

    #[must_use]
    pub fn window_new_tab_position(&self) -> String {
        if !self.loaded() {
            return String::new();
        }
        self.get_string("window-new-tab-position")
            .unwrap_or_default()
    }

    #[must_use]
    pub fn window_decorations(&self) -> bool {
        let default_value = true;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("window-decoration")
            .and_then(|s| WindowDecoration::from_raw_value(&s))
            .map_or(default_value, WindowDecoration::enabled)
    }

    #[must_use]
    pub fn window_theme(&self) -> Option<String> {
        if !self.loaded() {
            return None;
        }
        self.get_string("window-theme")
    }

    #[must_use]
    pub fn window_step_resize(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("window-step-resize", false)
    }

    /// The raw `fullscreen` enum config string (`"false"`, `"true"`,
    /// `"non-native"`, `"non-native-visible-menu"`, `"non-native-padded-notch"`).
    /// Swift returns the fullscreen mode if fullscreen is enabled, or nil if
    /// disabled; use [`fullscreen_mode_from_config`] for that mapping.
    /// TODO(MA3): return `Option<FullscreenMode>` once `helpers::fullscreen` exists.
    #[must_use]
    pub fn window_fullscreen(&self) -> Option<String> {
        if !self.loaded() {
            return None;
        }
        self.get_string("fullscreen")
    }

    /// The raw `macos-non-native-fullscreen` config string (`"false"`, `"true"`,
    /// `"visible-menu"`, `"padded-notch"`); Swift's default is native
    /// (`"false"`). Controls the fullscreen mode for toggle actions (keybindings).
    /// TODO(MA3): return `FullscreenMode`.
    #[must_use]
    pub fn window_fullscreen_mode(&self) -> String {
        let default_value = "false".to_string();
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-non-native-fullscreen")
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn window_title_font_family(&self) -> Option<String> {
        if !self.loaded() {
            return None;
        }
        self.get_string("window-title-font-family")
    }

    #[must_use]
    pub fn macos_window_buttons(&self) -> MacOSWindowButtons {
        let default_value = MacOSWindowButtons::Visible;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-window-buttons")
            .and_then(|s| MacOSWindowButtons::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn macos_titlebar_style(&self) -> MacOSTitlebarStyle {
        let default_value = MacOSTitlebarStyle::Transparent;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-titlebar-style")
            .and_then(|s| MacOSTitlebarStyle::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn macos_titlebar_proxy_icon(&self) -> MacOSTitlebarProxyIcon {
        let default_value = MacOSTitlebarProxyIcon::Visible;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-titlebar-proxy-icon")
            .and_then(|s| MacOSTitlebarProxyIcon::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn macos_dock_drop_behavior(&self) -> MacDockDropBehavior {
        let default_value = MacDockDropBehavior::NewTab;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-dock-drop-behavior")
            .and_then(|s| MacDockDropBehavior::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn macos_window_shadow(&self) -> bool {
        if !self.loaded() {
            return false;
        }
        self.get_bool("macos-window-shadow", false)
    }

    #[must_use]
    pub fn macos_icon(&self) -> MacOSIcon {
        let default_value = MacOSIcon::Official;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-icon")
            .and_then(|s| MacOSIcon::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn macos_custom_icon(&self) -> String {
        let default_value = expanding_tilde_in_path("~/.config/ghostty/Ghostty.icns");
        if !self.loaded() {
            return default_value;
        }
        match self.get_string("macos-custom-icon") {
            Some(path) => expanding_tilde_in_path(&path),
            None => default_value,
        }
    }

    #[must_use]
    pub fn macos_icon_frame(&self) -> MacOSIconFrame {
        let default_value = MacOSIconFrame::Aluminum;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-icon-frame")
            .and_then(|s| MacOSIconFrame::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    /// `macosIconGhostColor` (an `OSColor?` in Swift; AppKit conversion in `helpers::extensions`).
    #[must_use]
    pub fn macos_icon_ghost_color(&self) -> Option<Color> {
        if !self.loaded() {
            return None;
        }
        self.get_color("macos-icon-ghost-color")
            .map(|c| Color::from_rgb8(c.r, c.g, c.b))
    }

    /// `macosIconScreenColor` (an `[OSColor]?` in Swift).
    #[must_use]
    pub fn macos_icon_screen_color(&self) -> Option<Vec<Color>> {
        if !self.loaded() {
            return None;
        }
        let mut v = ghostty_config_color_list_s {
            colors: std::ptr::null(),
            len: 0,
        };
        if !self.get_raw("macos-icon-screen-color", &mut v) || v.len == 0 || v.colors.is_null() {
            return None;
        }
        // SAFETY: the C API guarantees `len` colors at `colors`, valid while the config lives.
        let buffer = unsafe { std::slice::from_raw_parts(v.colors, v.len) };
        Some(
            buffer
                .iter()
                .map(|c| Color::from_rgb8(c.r, c.g, c.b))
                .collect(),
        )
    }

    #[must_use]
    pub fn macos_hidden(&self) -> MacHidden {
        if !self.loaded() {
            return MacHidden::Never;
        }
        self.get_string("macos-hidden")
            .and_then(|s| MacHidden::from_raw_value(&s))
            .unwrap_or(MacHidden::Never)
    }

    #[must_use]
    pub fn focus_follows_mouse(&self) -> bool {
        if !self.loaded() {
            return false;
        }
        self.get_bool("focus-follows-mouse", false)
    }

    /// `background` when the lookup works.
    #[must_use]
    pub fn background_color_opt(&self) -> Option<Color> {
        self.get_color("background")
            .map(|c| Color::from_rgb8(c.r, c.g, c.b))
    }

    /// `backgroundColor`. Swift falls back to `NSColor.windowBackgroundColor`
    /// (dynamic); here a fixed light gray stands in.
    #[must_use]
    pub fn background_color(&self) -> Color {
        self.background_color_opt()
            .unwrap_or(Color::new(0.925, 0.925, 0.925))
    }

    #[must_use]
    pub fn background_opacity(&self) -> f64 {
        if !self.loaded() {
            return 1.0;
        }
        let mut v: f64 = 1.0;
        let _ = self.get_raw("background-opacity", &mut v);
        v
    }

    #[must_use]
    pub fn background_blur(&self) -> BackgroundBlur {
        if !self.loaded() {
            return BackgroundBlur::Disabled;
        }
        let mut v: i16 = 0;
        let _ = self.get_raw("background-blur", &mut v);
        BackgroundBlur::from_c_value(v)
    }

    #[must_use]
    pub fn unfocused_split_opacity(&self) -> f64 {
        if !self.loaded() {
            return 1.0;
        }
        let mut opacity: f64 = 0.85;
        let _ = self.get_raw("unfocused-split-opacity", &mut opacity);
        1.0 - opacity
    }

    #[must_use]
    pub fn unfocused_split_fill(&self) -> Color {
        if !self.loaded() {
            return Color::WHITE;
        }

        let mut color = ghostty_config_color_s { r: 0, g: 0, b: 0 };
        if !self.get_raw("unfocused-split-fill", &mut color) {
            let _ = self.get_raw("background", &mut color);
        }

        Color::from_rgb8(color.r, color.g, color.b)
    }

    #[must_use]
    pub fn split_divider_color(&self) -> Color {
        let background_color = self.background_color();
        let is_light_background = luminance(background_color) > 0.5;
        let new_color = if is_light_background {
            darken(background_color, 0.08)
        } else {
            darken(background_color, 0.4)
        };

        if !self.loaded() {
            return new_color;
        }

        match self.get_color("split-divider-color") {
            Some(c) => Color::from_rgb8(c.r, c.g, c.b),
            None => new_color,
        }
    }

    // MARK: Quick terminal

    #[must_use]
    pub fn quick_terminal_position(&self) -> QuickTerminalPosition {
        if !self.loaded() {
            return QuickTerminalPosition::Top;
        }
        self.get_string("quick-terminal-position")
            .and_then(|s| QuickTerminalPosition::from_raw_value(&s))
            .unwrap_or(QuickTerminalPosition::Top)
    }

    #[must_use]
    pub fn quick_terminal_screen(&self) -> QuickTerminalScreen {
        if !self.loaded() {
            return QuickTerminalScreen::Main;
        }
        self.get_string("quick-terminal-screen")
            .and_then(|s| QuickTerminalScreen::from_ghostty_config(&s))
            .unwrap_or(QuickTerminalScreen::Main)
    }

    #[must_use]
    pub fn quick_terminal_animation_duration(&self) -> f64 {
        if !self.loaded() {
            return 0.2;
        }
        let mut v: f64 = 0.2;
        let _ = self.get_raw("quick-terminal-animation-duration", &mut v);
        v
    }

    #[must_use]
    pub fn quick_terminal_auto_hide(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("quick-terminal-autohide", true)
    }

    #[must_use]
    pub fn quick_terminal_space_behavior(&self) -> QuickTerminalSpaceBehavior {
        if !self.loaded() {
            return QuickTerminalSpaceBehavior::Move;
        }
        self.get_string("quick-terminal-space-behavior")
            .and_then(|s| QuickTerminalSpaceBehavior::from_ghostty_config(&s))
            .unwrap_or(QuickTerminalSpaceBehavior::Move)
    }

    #[must_use]
    pub fn quick_terminal_size(&self) -> QuickTerminalSize {
        if !self.loaded() {
            return QuickTerminalSize::default();
        }
        // SAFETY: an all-zero value is a valid `none`/`none` size.
        let mut v: ghostty_config_quick_terminal_size_s = unsafe { std::mem::zeroed() };
        if !self.get_raw("quick-terminal-size", &mut v) {
            return QuickTerminalSize::default();
        }
        QuickTerminalSize::from_c(&v)
    }

    #[must_use]
    pub fn resize_overlay(&self) -> ResizeOverlay {
        if !self.loaded() {
            return ResizeOverlay::AfterFirst;
        }
        self.get_string("resize-overlay")
            .and_then(|s| ResizeOverlay::from_raw_value(&s))
            .unwrap_or(ResizeOverlay::AfterFirst)
    }

    #[must_use]
    pub fn resize_overlay_position(&self) -> ResizeOverlayPosition {
        let default_value = ResizeOverlayPosition::Center;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("resize-overlay-position")
            .and_then(|s| ResizeOverlayPosition::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn resize_overlay_duration(&self) -> usize {
        if !self.loaded() {
            return 1000;
        }
        let mut v: usize = 0;
        let _ = self.get_raw("resize-overlay-duration", &mut v);
        v
    }

    #[must_use]
    pub fn undo_timeout(&self) -> Duration {
        if !self.loaded() {
            return Duration::from_secs(5);
        }
        let mut v: usize = 0;
        let _ = self.get_raw("undo-timeout", &mut v);
        Duration::from_millis(v as u64)
    }

    #[must_use]
    pub fn auto_update(&self) -> Option<AutoUpdate> {
        if !self.loaded() {
            return None;
        }
        self.get_string("auto-update")
            .and_then(|s| AutoUpdate::from_raw_value(&s))
    }

    #[must_use]
    pub fn auto_update_channel(&self) -> AutoUpdateChannel {
        let default_value = AutoUpdateChannel::Stable;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("auto-update-channel")
            .and_then(|s| AutoUpdateChannel::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn auto_secure_input(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("macos-auto-secure-input", false)
    }

    #[must_use]
    pub fn secure_input_indication(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("macos-secure-input-indication", false)
    }

    #[must_use]
    pub fn macos_applescript(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("macos-applescript", false)
    }

    #[must_use]
    pub fn maximize(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("maximize", false)
    }

    #[must_use]
    pub fn macos_shortcuts(&self) -> MacShortcuts {
        let default_value = MacShortcuts::Ask;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("macos-shortcuts")
            .and_then(|s| MacShortcuts::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn scrollbar(&self) -> Scrollbar {
        let default_value = Scrollbar::System;
        if !self.loaded() {
            return default_value;
        }
        self.get_string("scrollbar")
            .and_then(|s| Scrollbar::from_raw_value(&s))
            .unwrap_or(default_value)
    }

    #[must_use]
    pub fn command_palette_entries(&self) -> Vec<Command> {
        if !self.loaded() {
            return Vec::new();
        }
        let mut v = ghostty_config_command_list_s {
            commands: std::ptr::null(),
            len: 0,
        };
        if !self.get_raw("command-palette-entry", &mut v) || v.len == 0 || v.commands.is_null() {
            return Vec::new();
        }
        // SAFETY: the C API guarantees `len` commands at `commands`, valid while the config lives.
        let buffer = unsafe { std::slice::from_raw_parts(v.commands, v.len) };
        // SAFETY: the command strings are NUL terminated.
        buffer
            .iter()
            .map(|c| unsafe { Command::from_c(c) })
            .collect()
    }

    #[must_use]
    pub fn progress_style(&self) -> bool {
        if !self.loaded() {
            return true;
        }
        self.get_bool("progress-style", true)
    }
}

/// The mapping `Ghostty.Config.windowFullscreen` applies to the `fullscreen`
/// config string: `None` when disabled (or unknown), otherwise the
/// `FullscreenMode` case name (`native`, `nonNative`, `nonNativeVisibleMenu`,
/// `nonNativePaddedNotch`).
/// TODO(MA3): superseded by `FullscreenMode` once `helpers::fullscreen` exists.
#[must_use]
pub fn fullscreen_mode_from_config(raw: &str) -> Option<&'static str> {
    match raw {
        "true" => Some("native"),
        "non-native" => Some("nonNative"),
        "non-native-visible-menu" => Some("nonNativeVisibleMenu"),
        "non-native-padded-notch" => Some("nonNativePaddedNotch"),
        _ => None,
    }
}

/// `String(cString:)`.
fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: C strings handed out by the library are NUL terminated and live
    // at least as long as the call.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// `(path as NSString).expandingTildeInPath` for `~` and `~/...`.
fn expanding_tilde_in_path(path: &str) -> String {
    let home = || std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        let home = home();
        format!("{}/{rest}", home.trim_end_matches('/'))
    } else {
        path.to_string()
    }
}

/// `OSColor.luminance`.
fn luminance(c: Color) -> f64 {
    0.299 * c.red + 0.587 * c.green + 0.114 * c.blue
}

/// `OSColor.darken(by:)`: scales HSB brightness by `1 - amount` (hue and
/// saturation kept), which is the same as scaling the RGB components.
fn darken(c: Color, amount: f64) -> Color {
    let k = 1.0 - amount;
    let s = |v: f64| (v * k).min(1.0);
    Color::new(s(c.red), s(c.green), s(c.blue))
}

// Silence "unused" for the c_int import on some cfgs.
const _: Option<c_int> = None;

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// `TemporaryConfig`: a config loaded from a temporary file.
    struct TemporaryConfig {
        config: Config,
        _file: tempfile::NamedTempFile,
    }

    impl std::ops::Deref for TemporaryConfig {
        type Target = Config;
        fn deref(&self) -> &Config {
            &self.config
        }
    }

    impl TemporaryConfig {
        fn new(text: &str) -> Self {
            Self::with_finalize(text, true)
        }

        fn with_finalize(text: &str, finalize: bool) -> Self {
            let mut file = tempfile::Builder::new()
                .suffix(".ghostty")
                .tempfile()
                .unwrap();
            file.write_all(text.as_bytes()).unwrap();
            file.flush().unwrap();
            let cfg = Self::load(file.path().to_str().unwrap(), finalize);
            Self {
                config: Config::from_ptr(cfg),
                _file: file,
            }
        }

        /// `Config.loadConfig(at:finalize:)` without the default files / CLI
        /// args that the test process would otherwise pick up from the host.
        fn load(path: &str, finalize: bool) -> ghostty_config_t {
            let path = CString::new(path).unwrap();
            let cfg = capi::ghostty_config_new();
            // SAFETY: valid handle and NUL terminated path.
            unsafe {
                capi::ghostty_config_load_file(cfg, path.as_ptr());
                capi::ghostty_config_load_recursive_files(cfg);
                if finalize {
                    capi::ghostty_config_finalize(cfg);
                }
            }
            cfg
        }

        fn optional_auto_update_channel(&self) -> Option<AutoUpdateChannel> {
            self.get_string("auto-update-channel")
                .and_then(|s| AutoUpdateChannel::from_raw_value(&s))
        }
    }

    // MARK: - Boolean Properties

    #[test]
    fn initial_window_defaults_to_true() {
        assert!(TemporaryConfig::new("").initial_window());
    }

    #[test]
    fn initial_window_set_to_false() {
        assert!(!TemporaryConfig::new("initial-window = false").initial_window());
    }

    #[test]
    fn quit_after_last_window_closed_defaults_to_false() {
        // The default is platform dependent in the config (true on Linux,
        // false on macOS); the Swift test runs on macOS.
        assert_eq!(
            TemporaryConfig::new("").should_quit_after_last_window_closed(),
            !cfg!(target_vendor = "apple")
        );
    }

    #[test]
    fn quit_after_last_window_closed_set_to_true() {
        assert!(
            TemporaryConfig::new("quit-after-last-window-closed = true")
                .should_quit_after_last_window_closed()
        );
    }

    #[test]
    fn window_step_resize_defaults_to_false() {
        assert!(!TemporaryConfig::new("").window_step_resize());
    }

    #[test]
    fn focus_follows_mouse_defaults_to_false() {
        assert!(!TemporaryConfig::new("").focus_follows_mouse());
    }

    #[test]
    fn focus_follows_mouse_set_to_true() {
        assert!(TemporaryConfig::new("focus-follows-mouse = true").focus_follows_mouse());
    }

    #[test]
    fn window_decorations_defaults_to_true() {
        assert!(TemporaryConfig::new("").window_decorations());
    }

    #[test]
    fn window_decorations_none() {
        assert!(!TemporaryConfig::new("window-decoration = none").window_decorations());
    }

    #[test]
    fn macos_window_shadow_defaults_to_true() {
        assert!(TemporaryConfig::new("").macos_window_shadow());
    }

    #[test]
    fn maximize_defaults_to_false() {
        assert!(!TemporaryConfig::new("").maximize());
    }

    #[test]
    fn maximize_set_to_true() {
        assert!(TemporaryConfig::new("maximize = true").maximize());
    }

    // MARK: - String / Optional String Properties

    #[test]
    fn title_defaults_to_nil() {
        assert_eq!(TemporaryConfig::new("").title(), None);
    }

    #[test]
    fn title_set_to_custom_value() {
        assert_eq!(
            TemporaryConfig::new("title = My Terminal")
                .title()
                .as_deref(),
            Some("My Terminal")
        );
    }

    #[test]
    fn window_title_font_family_defaults_to_nil() {
        assert_eq!(TemporaryConfig::new("").window_title_font_family(), None);
    }

    #[test]
    fn window_title_font_family_set_to_value() {
        assert_eq!(
            TemporaryConfig::new("window-title-font-family = Menlo")
                .window_title_font_family()
                .as_deref(),
            Some("Menlo")
        );
    }

    // MARK: - Enum Properties

    #[test]
    fn macos_titlebar_style_defaults_to_transparent() {
        assert_eq!(
            TemporaryConfig::new("").macos_titlebar_style(),
            MacOSTitlebarStyle::Transparent
        );
    }

    #[test]
    fn macos_titlebar_style_values() {
        for (raw, expected) in [
            ("native", MacOSTitlebarStyle::Native),
            ("transparent", MacOSTitlebarStyle::Transparent),
            ("tabs", MacOSTitlebarStyle::Tabs),
            ("hidden", MacOSTitlebarStyle::Hidden),
        ] {
            let config = TemporaryConfig::new(&format!("macos-titlebar-style = {raw}"));
            assert_eq!(config.macos_titlebar_style(), expected, "{raw}");
        }
    }

    #[test]
    fn resize_overlay_defaults_to_after_first() {
        assert_eq!(
            TemporaryConfig::new("").resize_overlay(),
            ResizeOverlay::AfterFirst
        );
    }

    #[test]
    fn resize_overlay_values() {
        for (raw, expected) in [
            ("always", ResizeOverlay::Always),
            ("never", ResizeOverlay::Never),
            ("after-first", ResizeOverlay::AfterFirst),
        ] {
            let config = TemporaryConfig::new(&format!("resize-overlay = {raw}"));
            assert_eq!(config.resize_overlay(), expected, "{raw}");
        }
    }

    #[test]
    fn resize_overlay_position_defaults_to_center() {
        assert_eq!(
            TemporaryConfig::new("").resize_overlay_position(),
            ResizeOverlayPosition::Center
        );
    }

    #[test]
    fn macos_icon_defaults_to_official() {
        assert_eq!(TemporaryConfig::new("").macos_icon(), MacOSIcon::Official);
    }

    #[test]
    fn macos_icon_frame_defaults_to_aluminum() {
        assert_eq!(
            TemporaryConfig::new("").macos_icon_frame(),
            MacOSIconFrame::Aluminum
        );
    }

    #[test]
    fn macos_window_buttons_defaults_to_visible() {
        assert_eq!(
            TemporaryConfig::new("").macos_window_buttons(),
            MacOSWindowButtons::Visible
        );
    }

    #[test]
    fn scrollbar_defaults_to_system() {
        assert_eq!(TemporaryConfig::new("").scrollbar(), Scrollbar::System);
    }

    #[test]
    fn scrollbar_set_to_never() {
        assert_eq!(
            TemporaryConfig::new("scrollbar = never").scrollbar(),
            Scrollbar::Never
        );
    }

    // MARK: - Numeric Properties

    #[test]
    fn background_opacity_defaults_to_one() {
        assert!((TemporaryConfig::new("").background_opacity() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn background_opacity_set_to_custom() {
        let v = TemporaryConfig::new("background-opacity = 0.5").background_opacity();
        assert!((v - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn window_position_defaults_to_nil() {
        let config = TemporaryConfig::new("");
        assert_eq!(config.window_position_x(), None);
        assert_eq!(config.window_position_y(), None);
    }

    // MARK: - Config Loading

    #[test]
    fn loaded_is_true_for_valid_config() {
        assert!(TemporaryConfig::new("").loaded());
    }

    #[test]
    fn unfinalized_config_is_loaded() {
        assert!(TemporaryConfig::with_finalize("", false).loaded());
    }

    #[test]
    fn default_config_is_loaded() {
        let config = TemporaryConfig::new("");
        assert!(config.optional_auto_update_channel().is_some()); // release or tip
        let config1 = TemporaryConfig::with_finalize("", false);
        assert!(config1.optional_auto_update_channel().is_none());
    }

    #[test]
    fn errors_empty_for_valid_config() {
        assert!(TemporaryConfig::new("").errors().is_empty());
    }

    #[test]
    fn errors_reported_for_invalid_config() {
        assert!(
            !TemporaryConfig::new("not-a-real-key = value")
                .errors()
                .is_empty()
        );
    }

    // MARK: - Multiple Config Lines

    #[test]
    fn multiple_config_values() {
        let config = TemporaryConfig::new(
            "initial-window = false\nquit-after-last-window-closed = true\nmaximize = true\nfocus-follows-mouse = true\n",
        );
        assert!(!config.initial_window());
        assert!(config.should_quit_after_last_window_closed());
        assert!(config.maximize());
        assert!(config.focus_follows_mouse());
    }

    // MARK: - Extra

    #[test]
    fn config_new_with_path_loads_the_file() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"title = From Path\n").unwrap();
        file.flush().unwrap();
        let config = Config::new(file.path().to_str(), true);
        assert!(config.loaded());
        assert_eq!(config.title().as_deref(), Some("From Path"));
    }

    #[test]
    fn null_config_returns_swift_defaults() {
        let c = Config::from_ptr(std::ptr::null_mut());
        assert!(!c.loaded());
        assert!(c.errors().is_empty());
        assert!(c.initial_window());
        assert!(c.should_quit_after_last_window_closed());
        assert!(c.window_step_resize());
        assert_eq!(c.window_save_state(), "");
        assert_eq!(c.macos_dock_drop_behavior(), MacDockDropBehavior::NewTab);
        assert_eq!(c.notify_on_command_finish_after(), Duration::from_secs(5));
        assert_eq!(c.undo_timeout(), Duration::from_secs(5));
        assert!((c.unfocused_split_opacity() - 1.0).abs() < f64::EPSILON);
        assert_eq!(c.unfocused_split_fill(), Color::WHITE);
        assert!(c.keyboard_shortcut("quit").is_none());
        assert_eq!(c.resize_overlay_duration(), 1000);
    }

    #[test]
    fn keyboard_shortcut_for_bound_action() {
        let config = TemporaryConfig::new("keybind = clear\nkeybind = cmd+shift+y=new_window\n");
        let s = config.keyboard_shortcut("new_window").expect("bound");
        assert_eq!(s.key.character, 'y');
        assert_eq!(
            s.modifiers,
            crate::helpers::keyboard_shortcut::EventModifiers::COMMAND
                | crate::helpers::keyboard_shortcut::EventModifiers::SHIFT
        );
        assert!(config.keyboard_shortcut("quit").is_none());
    }

    #[test]
    fn bell_features_and_command_finish_flags() {
        let config = TemporaryConfig::new(
            "bell-features = system,attention\nnotify-on-command-finish-action = bell,notify\nnotify-on-command-finish-after = 2s\n",
        );
        assert!(
            config
                .bell_features()
                .contains(BellFeatures::SYSTEM | BellFeatures::ATTENTION)
        );
        assert_eq!(
            config.notify_on_command_finish_action(),
            NotifyOnCommandFinishAction::BELL | NotifyOnCommandFinishAction::NOTIFY
        );
        assert_eq!(
            config.notify_on_command_finish_after(),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn colors_and_divider() {
        let config = TemporaryConfig::new("background = #ffffff\nsplit-divider-color = #102030\n");
        assert_eq!(config.background_color(), Color::WHITE);
        assert_eq!(
            config.split_divider_color(),
            Color::from_rgb8(0x10, 0x20, 0x30)
        );
        let derived = TemporaryConfig::new("background = #000000\n");
        assert_eq!(derived.split_divider_color(), Color::new(0.0, 0.0, 0.0));
        let light = TemporaryConfig::new("background = #ffffff\n");
        let d = light.split_divider_color();
        assert!((d.red - 0.92).abs() < 1e-9);
    }

    #[test]
    fn fullscreen_raw_mapping() {
        assert_eq!(fullscreen_mode_from_config("true"), Some("native"));
        assert_eq!(fullscreen_mode_from_config("false"), None);
        let c = TemporaryConfig::new("");
        assert_eq!(c.window_fullscreen().as_deref(), Some("false"));
    }

    #[test]
    fn tilde_expansion() {
        assert_eq!(expanding_tilde_in_path("/abs"), "/abs");
        assert!(!expanding_tilde_in_path("~/x").starts_with('~'));
    }
}
